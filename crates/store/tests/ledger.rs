#![allow(clippy::unwrap_used)]
//! The ledger's storage contract: seeding, periods, locks, and the fact that
//! postings and journals cannot be edited once they exist.

use crux::{
    AccountCode, AccountKind, Currency, DocumentId, Journal, LedgerEntry, LedgerLine, LocalDate, Money, SellerId, Side,
};
use store::ledger::{
    Account, OpeningBalance, PeriodStatus, PostingRequest, PostingSource, UNCLASSIFIED_ACCOUNT, content_hash,
    ensure_period, insert_account, list_accounts, list_periods, load_account, load_entry, load_period, lock_period,
    post_entry, post_opening_balances, reversal_of, reverse_entry, seed_starter_chart, starter_chart, update_account,
};
use store::{Database, KdfParams, KeyStore, SellerRecord, StoreError, insert_seller};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T09:15:00.123456Z";
const LATER: &str = "2026-11-07T10:00:00.000000Z";

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
            reverses: None,
            open_item: None,
        },
    )
    .unwrap()
}

/// Debits minus credits for one account: positive is a debit balance.
fn balance(database: &Database, account: &str) -> i64 {
    database
        .connection()
        .query_row(
            "SELECT COALESCE(sum(CASE side WHEN 'debit' THEN amount_micros ELSE -amount_micros END), 0)
             FROM journal_line WHERE account_code = ?1",
            [account],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn the_starter_chart_seeds_once_and_never_undoes_a_rename() {
    let (_dir, database, _seller) = database();
    // 0021 seeds the exchange-difference account, so a fresh archive already has
    // that one row; the rest are what this call adds.
    let chart = starter_chart().unwrap().len();
    let expected = chart - 1;

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
    assert_eq!(list_accounts(database.connection()).unwrap().len(), chart);
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
fn closing_a_year_moves_the_result_to_equity_and_locks_it() {
    let (_dir, database, seller) = database();
    let connection = database.connection();
    seed_starter_chart(connection, NOW).unwrap();

    // The scenario matrix: 1 000 of revenue in March and 200 of cost in April.
    post(
        &database,
        seller,
        &domestic_sale("Invoice 2026-0001"),
        "2026-03-15",
        PostingSource::Manual,
    );
    let cost = LedgerEntry::new(
        Journal::Purchase,
        "Invoice INK-1",
        vec![
            LedgerLine::debit(code("7000"), euros(200)).unwrap(),
            LedgerLine::credit(code("1400"), euros(200)).unwrap(),
        ],
    )
    .unwrap();
    post(&database, seller, &cost, "2026-04-02", PostingSource::Manual);

    let preview = store::year_result(connection, seller, 2026).unwrap();
    assert_eq!(preview.micros(), 800_000_000, "1 000 revenue less 200 cost");

    let close = store::close_year(connection, seller, 2026, "ada", NOW).unwrap();
    assert_eq!(close.year, 2026);
    assert_eq!(close.result_micros, 800_000_000);
    assert_eq!(close.locked, vec![3, 4], "the months that existed were the open ones");
    assert_eq!(
        close.warnings.len(),
        2,
        "neither month has a prepared return: {:?}",
        close.warnings
    );

    // The result is on equity and the profit and loss accounts are empty again.
    assert_eq!(
        balance(&database, store::RESULT_ACCOUNT),
        -800_000_000,
        "equity is credited"
    );
    assert_eq!(balance(&database, "8000"), 0);
    assert_eq!(balance(&database, "7000"), 0);

    // The closing entry is a memorial one in the adjustment period, dated the last
    // day of the year, so December's own period — and its return — never sees it.
    let posted = close.posted.as_ref().expect("a result was moved");
    assert_eq!(posted.journal, Journal::Memorial);
    let (period_no, entry_date): (i64, String) = connection
        .query_row(
            "SELECT p.period_no, e.entry_date_local FROM journal_entry e
             JOIN fiscal_period p ON p.id = e.period_id WHERE e.id = ?1",
            [posted.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(period_no, 13);
    assert_eq!(entry_date, "2026-12-31");

    // Every period of the year is locked, and the close itself is on the record.
    for period in list_periods(connection, seller, 2026).unwrap() {
        assert!(
            !period.is_open(),
            "{}-{:02} is still open",
            period.year,
            period.period_no
        );
    }
    let reason: String = connection
        .query_row(
            "SELECT e.reason FROM period_event e JOIN fiscal_period p ON p.id = e.period_id
             WHERE p.year = 2026 AND p.period_no = 13 AND e.kind = 'closed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(reason.contains("resultaatbestemming"), "{reason}");

    // The books still add up, with nothing left in the derived result line.
    let sheet = store::report::balance_sheet(connection, seller, date("2026-12-31")).unwrap();
    assert!(sheet.balances(), "assets equal liabilities plus equity");
    assert_eq!(sheet.result_micros, 0, "the result is in equity now");

    // A posting into the closed year is refused, and so is a second close.
    let refused = post_entry(
        connection,
        PostingRequest {
            entry: &domestic_sale("Invoice 2026-0002"),
            seller_id: seller,
            entry_date: date("2026-05-01"),
            source: PostingSource::Manual,
            actor: "ada",
            now: NOW,
            posting_profile_id: "nl-sme",
            rule_version: "1",
            reverses: None,
            open_item: None,
        },
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("fiscal year 2026 is closed"), "{refused}");

    let again = store::close_year(connection, seller, 2026, "ada", NOW)
        .unwrap_err()
        .to_string();
    assert!(again.contains("already closed"), "{again}");
}

#[test]
fn a_year_close_names_what_stops_it() {
    let (_dir, database, seller) = database();
    let connection = database.connection();
    seed_starter_chart(connection, NOW).unwrap();

    // Nothing has been posted, so there is no year to close.
    let blockers = store::year_close_blockers(connection, seller, 2026).unwrap();
    assert_eq!(blockers.len(), 1, "{blockers:?}");
    assert!(blockers[0].contains("nothing was posted in 2026"), "{}", blockers[0]);

    post(
        &database,
        seller,
        &domestic_sale("Invoice 2026-0001"),
        "2026-03-15",
        PostingSource::Manual,
    );
    // A cost nobody placed, in April: the month is named, because the remedy is
    // per month and not in the year.
    let unplaced = LedgerEntry::new(
        Journal::Purchase,
        "Invoice INK-1",
        vec![
            LedgerLine::debit(code(UNCLASSIFIED_ACCOUNT), euros(50)).unwrap(),
            LedgerLine::credit(code("1400"), euros(50)).unwrap(),
        ],
    )
    .unwrap();
    post(&database, seller, &unplaced, "2026-04-02", PostingSource::Manual);

    let blockers = store::year_close_blockers(connection, seller, 2026).unwrap();
    assert_eq!(blockers.len(), 1, "{blockers:?}");
    assert!(blockers[0].starts_with("2026-04: "), "{}", blockers[0]);
    assert!(blockers[0].contains(UNCLASSIFIED_ACCOUNT), "{}", blockers[0]);

    let refused = store::close_year(connection, seller, 2026, "ada", NOW)
        .unwrap_err()
        .to_string();
    assert!(refused.contains(UNCLASSIFIED_ACCOUNT), "{refused}");
    assert!(
        load_period(connection, seller, 2026, 13).unwrap().is_none(),
        "a refused close writes nothing, not even the adjustment period"
    );
}

#[test]
fn a_year_with_nothing_to_move_is_closed_without_an_entry() {
    let (_dir, database, seller) = database();
    let connection = database.connection();
    seed_starter_chart(connection, NOW).unwrap();

    // Capital paid in: a balance-sheet fact, so the year has no result.
    let capital = LedgerEntry::new(
        Journal::Memorial,
        "Kapitaal",
        vec![
            LedgerLine::debit(code("1100"), euros(500)).unwrap(),
            LedgerLine::credit(code(store::RESULT_ACCOUNT), euros(500)).unwrap(),
        ],
    )
    .unwrap();
    post(&database, seller, &capital, "2026-01-02", PostingSource::Manual);

    let close = store::close_year(connection, seller, 2026, "ada", NOW).unwrap();
    assert!(close.posted.is_none(), "there was no result to move");
    assert_eq!(close.result_micros, 0);
    assert_eq!(
        balance(&database, store::RESULT_ACCOUNT),
        -500_000_000,
        "equity is untouched"
    );
    assert!(!list_periods(connection, seller, 2026).unwrap()[0].is_open());
}

#[test]
fn a_reopened_year_takes_a_correction_and_closes_again() {
    let (_dir, database, seller) = database();
    let connection = database.connection();
    seed_starter_chart(connection, NOW).unwrap();
    post(
        &database,
        seller,
        &domestic_sale("Invoice 2026-0001"),
        "2026-03-15",
        PostingSource::Manual,
    );

    let first = store::close_year(connection, seller, 2026, "ada", NOW).unwrap();
    assert_eq!(first.result_micros, 1_000_000_000);

    let reversal = store::reopen_year(connection, seller, 2026, "vergeten kosten", "ada", LATER)
        .unwrap()
        .expect("there was a resultaatbestemming to reverse");
    assert_eq!(reversal.journal, Journal::Memorial);
    assert_eq!(balance(&database, "8000"), -1_000_000_000, "the revenue is back");
    assert_eq!(
        balance(&database, store::RESULT_ACCOUNT),
        0,
        "equity no longer holds it"
    );

    // A year that is open is not reopened again, and a reason is always required.
    let said = store::reopen_year(connection, seller, 2026, "again", "ada", LATER)
        .unwrap_err()
        .to_string();
    assert!(said.contains("not closed"), "{said}");
    let said = store::reopen_year(connection, seller, 2026, "  ", "ada", LATER)
        .unwrap_err()
        .to_string();
    assert!(said.contains("needs a reason"), "{said}");

    // The month a correction belongs in is reopened on its own: a month lock is its
    // own fact, and the year being open does not reopen twelve months.
    let march = load_period(connection, seller, 2026, 3).unwrap().unwrap();
    store::reopen_period(connection, march.id, "vergeten kosten", "ada", LATER).unwrap();
    let cost = LedgerEntry::new(
        Journal::Purchase,
        "Invoice INK-1",
        vec![
            LedgerLine::debit(code("7000"), euros(100)).unwrap(),
            LedgerLine::credit(code("1400"), euros(100)).unwrap(),
        ],
    )
    .unwrap();
    post(&database, seller, &cost, "2026-03-20", PostingSource::Manual);

    let second = store::close_year(connection, seller, 2026, "ada", LATER).unwrap();
    assert_eq!(second.result_micros, 900_000_000, "the cost is in the year now");
    assert_eq!(second.locked, vec![3], "March was the only open month");
    assert_eq!(balance(&database, store::RESULT_ACCOUNT), -900_000_000);

    // A month inside the closed year is not reopened on its own: the year is what
    // refuses the postings, so the year is what has to open first.
    let said = store::reopen_period(connection, march.id, "weer", "ada", LATER)
        .unwrap_err()
        .to_string();
    assert!(said.contains("reopen the year"), "{said}");
    assert!(said.contains("reopen-year"), "the remedy is a command: {said}");
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
            reverses: None,
            open_item: None,
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

    let document = DocumentId::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
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
    // Pinned shape: this is what an entry written before foreign-currency support
    // hashes to. A new key must be added only when it applies, or every old
    // posting (and every evidence bundle carrying one) stops verifying.
    assert_eq!(
        posted.content_hash,
        "917a446f440ff87aa866827d2b677f6e38f55307b1ec689c33aa4cd4e6552f70"
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
            reverses: None,
            open_item: None,
        },
    )
    .unwrap_err();
    assert!(matches!(error, StoreError::Rejected(_)), "{error}");
    assert!(error.to_string().contains("archived"), "{error}");
}

#[test]
fn a_reversal_names_the_entry_it_undoes_and_nets_to_zero() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();

    let original = post(
        &database,
        seller,
        &domestic_sale("Invoice 2026-0001"),
        "2026-01-15",
        PostingSource::Manual,
    );
    let reversal = reverse_entry(
        database.connection(),
        original.id,
        "wrong customer",
        date("2026-02-03"),
        "ada",
        NOW,
    )
    .unwrap();

    assert!(reversal.created);
    assert_eq!(reversal.number, "2026-0002", "the reversal is a new entry, not an edit");
    assert_eq!(
        reversal_of(database.connection(), original.id).unwrap().as_deref(),
        Some("2026-0002")
    );

    let stored = load_entry(database.connection(), reversal.id).unwrap().unwrap();
    assert_eq!(stored.reverses, Some(original.id));
    assert_eq!(stored.journal, Journal::Sales);
    assert_eq!(
        stored.entry_date.to_iso_string(),
        "2026-02-03",
        "dated by the caller, in an open period"
    );
    assert!(stored.entry.description().contains("Tegenboeking 2026-0001"));
    assert!(stored.entry.description().contains("wrong customer"));
    assert_eq!(
        stored.posting_profile_id,
        load_entry(database.connection(), original.id)
            .unwrap()
            .unwrap()
            .posting_profile_id,
        "a reversal is produced by the rules that produced the original"
    );

    // The pair cancels per account.
    let net: i64 = database
        .connection()
        .query_row(
            "SELECT sum(CASE side WHEN 'debit' THEN amount_micros ELSE -amount_micros END)
             FROM journal_line WHERE entry_id IN (?1, ?2)",
            rusqlite::params![original.id.to_string(), reversal.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(net, 0);
}

#[test]
fn an_entry_is_reversed_at_most_once() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();

    let original = post(
        &database,
        seller,
        &domestic_sale("Invoice 2026-0001"),
        "2026-01-15",
        PostingSource::Manual,
    );
    reverse_entry(
        database.connection(),
        original.id,
        "typo",
        date("2026-02-03"),
        "ada",
        NOW,
    )
    .unwrap();

    let error = reverse_entry(
        database.connection(),
        original.id,
        "again",
        date("2026-02-04"),
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(matches!(error, StoreError::Rejected(_)), "{error}");
    assert!(error.to_string().contains("already reversed"), "{error}");

    let entries: i64 = database
        .connection()
        .query_row("SELECT count(*) FROM journal_entry", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entries, 2, "the original and its one reversal");
}

#[test]
fn a_reversal_needs_a_reason() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();
    let original = post(
        &database,
        seller,
        &domestic_sale("Invoice 2026-0001"),
        "2026-01-15",
        PostingSource::Manual,
    );

    let error = reverse_entry(
        database.connection(),
        original.id,
        "   ",
        date("2026-02-03"),
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(error.to_string().contains("needs a reason"), "{error}");
}

#[test]
fn a_reversal_into_a_locked_period_is_refused() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();
    let original = post(
        &database,
        seller,
        &domestic_sale("Invoice 2026-0001"),
        "2026-01-15",
        PostingSource::Manual,
    );

    let period = ensure_period(database.connection(), seller, date("2026-02-10")).unwrap();
    lock_period(database.connection(), period.id, "ada", NOW).unwrap();

    let error = reverse_entry(
        database.connection(),
        original.id,
        "wrong period",
        date("2026-02-10"),
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(matches!(error, StoreError::Rejected(_)), "{error}");
    assert_eq!(
        reversal_of(database.connection(), original.id).unwrap(),
        None,
        "a refused reversal leaves nothing behind"
    );
}

#[test]
fn opening_balances_post_into_the_opening_journal() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();

    let posted = post_opening_balances(
        database.connection(),
        seller,
        date("2026-01-01"),
        &[
            OpeningBalance {
                account: code("1100"),
                side: Side::Debit,
                amount: euros(5_000),
            },
            OpeningBalance {
                account: code("0500"),
                side: Side::Credit,
                amount: euros(5_000),
            },
        ],
        "ada",
        NOW,
    )
    .unwrap();

    let stored = load_entry(database.connection(), posted.id).unwrap().unwrap();
    assert_eq!(stored.journal, Journal::Opening);
    assert_eq!(stored.entry.description(), "Beginbalans 2026");
    assert_eq!(stored.number, "2026-0001");
    assert_eq!(stored.entry.debit_total().unwrap(), euros(5_000));
}

#[test]
fn opening_balances_must_balance_and_stay_on_the_balance_sheet() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();

    let unbalanced = post_opening_balances(
        database.connection(),
        seller,
        date("2026-01-01"),
        &[
            OpeningBalance {
                account: code("1100"),
                side: Side::Debit,
                amount: euros(5_000),
            },
            OpeningBalance {
                account: code("0500"),
                side: Side::Credit,
                amount: euros(4_000),
            },
        ],
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(
        unbalanced.to_string().contains("do not balance"),
        "unexpected error: {unbalanced}"
    );

    let profit_and_loss = post_opening_balances(
        database.connection(),
        seller,
        date("2026-01-01"),
        &[
            OpeningBalance {
                account: code("1100"),
                side: Side::Debit,
                amount: euros(5_000),
            },
            OpeningBalance {
                account: code("8000"),
                side: Side::Credit,
                amount: euros(5_000),
            },
        ],
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(
        profit_and_loss.to_string().contains("balance sheet"),
        "unexpected error: {profit_and_loss}"
    );
}

#[test]
fn opening_balances_are_posted_once() {
    let (_dir, database, seller) = database();
    seed_starter_chart(database.connection(), NOW).unwrap();

    let balances = vec![
        OpeningBalance {
            account: code("1100"),
            side: Side::Debit,
            amount: euros(5_000),
        },
        OpeningBalance {
            account: code("0500"),
            side: Side::Credit,
            amount: euros(5_000),
        },
    ];
    post_opening_balances(database.connection(), seller, date("2026-01-01"), &balances, "ada", NOW).unwrap();

    let again =
        post_opening_balances(database.connection(), seller, date("2026-01-01"), &balances, "ada", NOW).unwrap_err();
    assert!(
        again.to_string().contains("already posted"),
        "unexpected error: {again}"
    );
}
