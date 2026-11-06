#![allow(clippy::unwrap_used)]
//! The ledger's storage contract: seeding, periods, locks, and the fact that
//! postings and journals cannot be edited once they exist.

use crux::{AccountCode, AccountKind, Journal, LocalDate, SellerId};
use store::ledger::{
    Account, PeriodStatus, UNCLASSIFIED_ACCOUNT, ensure_period, insert_account, list_accounts, list_periods,
    load_account, load_period, lock_period, seed_starter_chart, starter_chart, update_account,
};
use store::{Database, KdfParams, KeyStore, SellerRecord, StoreError, insert_seller};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T09:15:00.123456Z";

fn database() -> (TempDir, Database, SellerId) {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
    let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();

    let seller_id = SellerId::new();
    let seller = SellerRecord {
        id: seller_id,
        party: crux::Party::new("Boeken BV", crux::Address::new("Utrecht", crux::CountryCode::NL)),
        default_currency: crux::Currency::EUR,
        default_locale: "nl-NL".to_owned(),
    };
    insert_seller(database.connection(), &seller, NOW).unwrap();
    (dir, database, seller_id)
}

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
}

#[test]
fn the_starter_chart_seeds_once_and_never_undoes_a_rename() {
    let (_dir, database, _seller) = database();
    let expected = starter_chart().unwrap().len();

    assert_eq!(seed_starter_chart(database.connection(), NOW).unwrap(), expected);
    assert_eq!(
        seed_starter_chart(database.connection(), NOW).unwrap(),
        0,
        "seeding twice must add nothing"
    );

    let bank = AccountCode::parse("1100").unwrap();
    let mut account = load_account(database.connection(), &bank).unwrap().unwrap();
    assert_eq!(account.name, "Bank");
    account.name = "Betaalrekening ING".to_owned();
    account.rgs_code = Some("BEd1".to_owned());
    update_account(database.connection(), &account, NOW).unwrap();

    seed_starter_chart(database.connection(), NOW).unwrap();
    let renamed = load_account(database.connection(), &bank).unwrap().unwrap();
    assert_eq!(renamed.name, "Betaalrekening ING", "a reseed must not undo the rename");
    assert_eq!(renamed.rgs_code.as_deref(), Some("BEd1"));
    assert_eq!(list_accounts(database.connection()).unwrap().len(), expected);
}

#[test]
fn accounts_are_unique_and_archivable() {
    let (_dir, database, _seller) = database();
    let code = AccountCode::parse("4200").unwrap();
    let account = Account {
        code: code.clone(),
        name: "Huisvestingskosten".to_owned(),
        kind: AccountKind::Expense,
        rgs_code: None,
        parent: None,
        is_control: false,
        currency: None,
        archived: false,
    };
    insert_account(database.connection(), &account, NOW).unwrap();

    let duplicate = insert_account(database.connection(), &account, NOW).unwrap_err();
    assert!(matches!(duplicate, StoreError::Rejected(_)), "{duplicate}");

    let mut archived = account;
    archived.archived = true;
    update_account(database.connection(), &archived, NOW).unwrap();
    assert!(load_account(database.connection(), &code).unwrap().unwrap().archived);

    let missing = AccountCode::parse("9998").unwrap();
    let error = update_account(
        database.connection(),
        &Account {
            code: missing,
            ..archived
        },
        NOW,
    )
    .unwrap_err();
    assert!(matches!(error, StoreError::NotFound(_)), "{error}");
}

#[test]
fn periods_are_created_on_demand_and_reused() {
    let (_dir, database, seller) = database();

    let january = ensure_period(database.connection(), seller, date("2026-01-15")).unwrap();
    assert_eq!(january.year, 2026);
    assert_eq!(january.period_no, 1);
    assert_eq!(january.starts_on.to_iso_string(), "2026-01-01");
    assert_eq!(january.ends_on.to_iso_string(), "2026-01-31");
    assert_eq!(january.status, PeriodStatus::Open);

    let again = ensure_period(database.connection(), seller, date("2026-01-31")).unwrap();
    assert_eq!(again.id, january.id, "the same month is the same period");

    let february = ensure_period(database.connection(), seller, date("2026-02-01")).unwrap();
    assert_ne!(february.id, january.id);
    assert_eq!(february.ends_on.to_iso_string(), "2026-02-28");

    let periods = list_periods(database.connection(), seller, 2026).unwrap();
    assert_eq!(periods.len(), 2);
    assert_eq!(periods[0].period_no, 1);
    assert_eq!(periods[1].period_no, 2);
    assert!(list_periods(database.connection(), seller, 2025).unwrap().is_empty());
    assert_eq!(
        load_period(database.connection(), seller, 2026, 3).unwrap(),
        None,
        "February does not create March"
    );
}

#[test]
fn locking_a_period_is_an_event_and_is_not_repeatable() {
    let (_dir, database, seller) = database();
    let period = ensure_period(database.connection(), seller, date("2026-01-15")).unwrap();

    lock_period(database.connection(), period.id, "ada", NOW).unwrap();
    let locked = load_period(database.connection(), seller, 2026, 1).unwrap().unwrap();
    assert_eq!(locked.status, PeriodStatus::Locked);
    assert!(!locked.is_open());

    let again = lock_period(database.connection(), period.id, "ada", NOW).unwrap_err();
    assert!(matches!(again, StoreError::Rejected(_)), "{again}");

    let events: i64 = database
        .connection()
        .query_row("SELECT count(*) FROM period_event WHERE kind = 'locked'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(events, 1);
}

#[test]
fn the_database_refuses_a_posting_into_a_locked_period() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();
    let period = ensure_period(database.connection(), seller, date("2026-01-15")).unwrap();
    lock_period(database.connection(), period.id, "ada", NOW).unwrap();

    // Written as raw SQL on purpose: the trigger has to refuse a posting, not
    // just the module that will eventually build one.
    let error = database
        .connection()
        .execute(
            "INSERT INTO journal_entry
                 (id, journal_code, period_id, number, entry_date_local, description,
                  source_kind, posting_profile_id, rule_version, currency,
                  created_at, created_by, content_hash)
             VALUES ('e1', 'MEM', ?1, '2026-0001', '2026-01-15', 'Manual entry',
                     'manual', 'test', '1', 'EUR', ?2, 'ada', 'hash')",
            rusqlite::params![period.id.to_string(), NOW],
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("fiscal period is locked"),
        "unexpected error: {error}"
    );
}

#[test]
fn journals_and_postings_cannot_be_edited() {
    let (_dir, database, _seller) = database();
    let connection = database.connection();

    // The seven journals exist, and each is the code the domain knows.
    let mut statement = connection.prepare("SELECT code FROM journal ORDER BY code").unwrap();
    let codes: Vec<String> = statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut expected: Vec<&str> = Journal::ALL.iter().map(|journal| journal.code()).collect();
    expected.sort_unstable();
    assert_eq!(codes, expected);

    let update = connection
        .execute("UPDATE journal SET name = 'Anders' WHERE code = 'VK'", [])
        .unwrap_err();
    assert!(update.to_string().contains("journals are fixed"), "{update}");
    let delete = connection
        .execute("DELETE FROM journal WHERE code = 'VK'", [])
        .unwrap_err();
    assert!(delete.to_string().contains("journals are fixed"), "{delete}");

    // Account codes are never reused for a different kind of account.
    let unclassified = AccountCode::parse(UNCLASSIFIED_ACCOUNT).unwrap();
    assert!(load_account(connection, &unclassified).unwrap().is_none());
    seed_starter_chart(connection, NOW).unwrap();
    assert_eq!(
        load_account(connection, &unclassified).unwrap().unwrap().kind,
        AccountKind::Expense
    );
}
