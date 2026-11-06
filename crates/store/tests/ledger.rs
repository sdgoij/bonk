#![allow(clippy::unwrap_used)]
//! The ledger's storage contract: seeding, periods, locks, and the fact that
//! postings and journals cannot be edited once they exist.

use crux::{
    AccountCode, AccountKind, Currency, DocumentId, Journal, LedgerEntry, LedgerLine, LocalDate, Money, SellerId,
};
use store::ledger::{
    Account, PeriodStatus, PostingRequest, PostingSource, UNCLASSIFIED_ACCOUNT, content_hash, ensure_period,
    insert_account, list_accounts, list_periods, load_account, load_period, lock_period, post_entry,
    seed_starter_chart, starter_chart, update_account,
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

fn code(raw: &str) -> AccountCode {
    AccountCode::parse(raw).unwrap()
}

fn euros(units: i64) -> Money {
    Money::from_major_units(units, Currency::EUR).unwrap()
}

/// The domestic sale from the scenario matrix: 1 000 net at 21 %.
fn domestic_sale(description: &str) -> LedgerEntry {
    LedgerEntry::new(
        Journal::Sales,
        description,
        vec![
            LedgerLine::debit(code("1300"), euros(1_210)).unwrap(),
            LedgerLine::credit(code("8000"), euros(1_000)).unwrap(),
            LedgerLine::credit(code("1600"), euros(210)).unwrap(),
        ],
    )
    .unwrap()
}

fn post(
    database: &Database,
    seller: SellerId,
    entry: &LedgerEntry,
    on: &str,
    source: PostingSource,
) -> store::ledger::PostedEntry {
    post_entry(
        database.connection(),
        PostingRequest {
            entry,
            seller_id: seller,
            entry_date: date(on),
            source,
            actor: "ada",
            now: NOW,
            posting_profile_id: "nl-sme",
            rule_version: "1",
        },
    )
    .unwrap()
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

#[test]
fn posting_writes_the_entry_its_lines_and_one_audit_event() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();

    let entry = domestic_sale("Invoice 2026-0001");
    let posted = post(&database, seller, &entry, "2026-01-15", PostingSource::Manual);
    assert!(posted.created);
    assert_eq!(posted.number, "2026-0001");

    let connection = database.connection();
    let stored: (String, String, i64) = connection
        .query_row(
            "SELECT journal_code, currency, length(number) FROM journal_entry WHERE id = ?1",
            [posted.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(stored, ("VK".to_owned(), "EUR".to_owned(), 9));

    let (debits, credits): (i64, i64) = connection
        .query_row(
            "SELECT sum(CASE side WHEN 'debit' THEN amount_micros ELSE 0 END),
                    sum(CASE side WHEN 'credit' THEN amount_micros ELSE 0 END)
             FROM journal_line WHERE entry_id = ?1",
            [posted.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(debits, credits, "what was written must balance");
    assert_eq!(debits, 1_210_000_000);

    let events: i64 = connection
        .query_row(
            "SELECT count(*) FROM audit_event WHERE kind = 'posted' AND subject_id = ?1",
            [posted.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);

    let head = store::audit::head(connection).unwrap().unwrap();
    assert_eq!(head.hash, posted.audit_head);
}

#[test]
fn posting_the_same_document_twice_returns_the_first_entry() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();

    let document = DocumentId::new();
    let entry = domestic_sale("Invoice 2026-0001");
    let first = post(
        &database,
        seller,
        &entry,
        "2026-01-15",
        PostingSource::OutboundDocument(document),
    );
    let second = post(
        &database,
        seller,
        &entry,
        "2026-01-15",
        PostingSource::OutboundDocument(document),
    );

    assert!(first.created);
    assert!(!second.created, "the second call found the first posting");
    assert_eq!(first.id, second.id);
    assert_eq!(first.number, second.number);
    assert_eq!(first.content_hash, second.content_hash);
    assert_eq!(second.audit_head, "", "finding an entry records nothing new");

    let connection = database.connection();
    let entries: i64 = connection
        .query_row("SELECT count(*) FROM journal_entry", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entries, 1);
    let events: i64 = connection
        .query_row("SELECT count(*) FROM audit_event WHERE kind = 'posted'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(events, 1);
}

#[test]
fn a_locked_period_refuses_a_posting() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();
    let period = ensure_period(database.connection(), seller, date("2026-01-15")).unwrap();
    lock_period(database.connection(), period.id, "ada", NOW).unwrap();

    let error = post_entry(
        database.connection(),
        PostingRequest {
            entry: &domestic_sale("Invoice 2026-0001"),
            seller_id: seller,
            entry_date: date("2026-01-15"),
            source: PostingSource::Manual,
            actor: "ada",
            now: NOW,
            posting_profile_id: "nl-sme",
            rule_version: "1",
        },
    )
    .unwrap_err();

    assert!(matches!(error, StoreError::Rejected(_)), "{error}");
    assert!(error.to_string().contains("locked"), "{error}");
    let entries: i64 = database
        .connection()
        .query_row("SELECT count(*) FROM journal_entry", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entries, 0);
}

#[test]
fn numbers_run_per_journal_and_per_year() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();

    let first = post(
        &database,
        seller,
        &domestic_sale("One"),
        "2026-01-15",
        PostingSource::Manual,
    );
    let second = post(
        &database,
        seller,
        &domestic_sale("Two"),
        "2026-01-20",
        PostingSource::Manual,
    );
    assert_eq!(first.number, "2026-0001");
    assert_eq!(second.number, "2026-0002");

    // A different journal starts its own series.
    let memorial = LedgerEntry::new(
        Journal::Memorial,
        "Bank fee",
        vec![
            LedgerLine::debit(code("7100"), euros(5)).unwrap(),
            LedgerLine::credit(code("1100"), euros(5)).unwrap(),
        ],
    )
    .unwrap();
    let fee = post(&database, seller, &memorial, "2026-02-01", PostingSource::Manual);
    assert_eq!(fee.number, "2026-0001");

    // And a new year starts again from one.
    let next_year = post(
        &database,
        seller,
        &domestic_sale("Three"),
        "2027-01-05",
        PostingSource::Manual,
    );
    assert_eq!(next_year.number, "2027-0001");
}

#[test]
fn the_content_hash_is_a_function_of_the_entry() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();

    let document = DocumentId::new();
    let entry = domestic_sale("Invoice 2026-0001");
    let posted = post(
        &database,
        seller,
        &entry,
        "2026-01-15",
        PostingSource::OutboundDocument(document),
    );

    let recomputed = content_hash(
        &entry,
        &posted.number,
        date("2026-01-15"),
        PostingSource::OutboundDocument(document),
    )
    .unwrap();
    assert_eq!(
        recomputed, posted.content_hash,
        "a verifier must be able to re-derive it"
    );

    // Any change to what the entry says changes the hash; the identifier and the
    // actor do not, which is why re-deriving it from the stored facts works.
    let edited = domestic_sale("Invoice 2026-0001 (corrected)");
    let edited_hash = content_hash(
        &edited,
        &posted.number,
        date("2026-01-15"),
        PostingSource::OutboundDocument(document),
    )
    .unwrap();
    assert_ne!(edited_hash, posted.content_hash);

    let other_number = content_hash(
        &entry,
        "2026-0002",
        date("2026-01-15"),
        PostingSource::OutboundDocument(document),
    )
    .unwrap();
    assert_ne!(other_number, posted.content_hash);
}

#[test]
fn an_archived_account_takes_no_postings() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();

    let revenue = code("8000");
    let mut account = load_account(database.connection(), &revenue).unwrap().unwrap();
    account.archived = true;
    update_account(database.connection(), &account, NOW).unwrap();

    let error = post_entry(
        database.connection(),
        PostingRequest {
            entry: &domestic_sale("Invoice 2026-0001"),
            seller_id: seller,
            entry_date: date("2026-01-15"),
            source: PostingSource::Manual,
            actor: "ada",
            now: NOW,
            posting_profile_id: "nl-sme",
            rule_version: "1",
        },
    )
    .unwrap_err();
    assert!(matches!(error, StoreError::Rejected(_)), "{error}");
    assert!(error.to_string().contains("archived"), "{error}");
}
