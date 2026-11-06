#![allow(clippy::unwrap_used)]
//! Importing statements: once, without duplicating overlapping ranges, and never
//! a statement that disagrees with its own balances.

use crux::{AccountCode, Currency, LocalDate, SellerId};
use store::bank::{
    BankAccount, ImportReport, ParsedStatement, ParsedTransaction, import_statement, insert_bank_account,
    list_bank_accounts,
};
use store::{Database, KdfParams, KeyStore, SellerRecord, StoreError, insert_seller, seed_starter_chart};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T19:00:00.123456Z";
const ACCOUNT: &str = "acct-1";

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
}

fn line(day: &str, amount: i64, name: &str, remittance: &str) -> ParsedTransaction {
    ParsedTransaction {
        booking_date: date(day),
        value_date: None,
        amount_micros: amount,
        currency: Currency::EUR,
        counterparty_name: Some(name.to_owned()),
        counterparty_iban: Some("NL02RABO0123456789".to_owned()),
        remittance_info: Some(remittance.to_owned()),
        external_ref: None,
    }
}

fn statement(lines: Vec<ParsedTransaction>) -> ParsedStatement {
    ParsedStatement {
        format: "camt053",
        source_name: Some("jan.camt.xml".to_owned()),
        currency: Currency::EUR,
        period_start: Some(date("2026-01-01")),
        period_end: Some(date("2026-01-31")),
        opening_balance_micros: None,
        closing_balance_micros: None,
        transactions: lines,
    }
}

struct Books {
    _dir: TempDir,
    database: Database,
    seller: SellerId,
}

impl Books {
    fn open() -> Self {
        let dir = TempDir::new().unwrap();
        let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
        let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();
        seed_starter_chart(database.connection(), NOW).unwrap();

        let seller = SellerId::new();
        insert_seller(
            database.connection(),
            &SellerRecord {
                id: seller,
                party: crux::Party::new("Boeken BV", crux::Address::new("Utrecht", crux::CountryCode::NL)),
                default_currency: Currency::EUR,
                default_locale: "nl-NL".to_owned(),
            },
            NOW,
        )
        .unwrap();

        insert_bank_account(
            database.connection(),
            &BankAccount {
                id: ACCOUNT.to_owned(),
                seller_id: seller,
                name: "Zakelijke rekening".to_owned(),
                iban: Some("NL91ABNA0417164300".to_owned()),
                currency: Currency::EUR,
                ledger_account_code: AccountCode::parse("1100").unwrap(),
                import_profile: None,
            },
            NOW,
        )
        .unwrap();

        Self {
            _dir: dir,
            database,
            seller,
        }
    }

    fn import(&self, statement: &ParsedStatement, payload: &str) -> ImportReport {
        import_statement(
            self.database.connection(),
            ACCOUNT,
            statement,
            &store::audit::sha256_hex(payload.as_bytes()),
            NOW,
        )
        .unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.database.connection().query_row(sql, [], |row| row.get(0)).unwrap()
    }
}

#[test]
fn a_statement_is_stored_with_its_lines() {
    let books = Books::open();
    let january = statement(vec![
        line("2026-01-15", 1_210_000_000, "Klant BV", "Betaling factuur 2026-0001"),
        line("2026-01-20", -605_000_000, "Leverancier BV", "Huur januari"),
    ]);

    let report = books.import(&january, "<january/>");
    assert!(report.created);
    assert_eq!(report.imported, 2);
    assert_eq!(report.duplicates, 0);
    assert_eq!(books.count("SELECT count(*) FROM bank_statement"), 1);
    assert_eq!(books.count("SELECT count(*) FROM bank_transaction"), 2);

    // The bank account is listed, and the transactions start unmatched.
    let accounts = list_bank_accounts(books.database.connection(), books.seller).unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].ledger_account_code.as_str(), "1100");
    assert_eq!(
        books.count("SELECT count(*) FROM bank_transaction WHERE status = 'new'"),
        2
    );

    let events = books.count("SELECT count(*) FROM audit_event WHERE kind = 'bank_statement_imported'");
    assert_eq!(events, 1);
}

#[test]
fn the_same_file_twice_is_a_no_op() {
    let books = Books::open();
    let january = statement(vec![line("2026-01-15", 1_210_000_000, "Klant BV", "Betaling")]);

    let first = books.import(&january, "<january/>");
    let second = books.import(&january, "<january/>");

    assert!(first.created);
    assert!(!second.created, "the same bytes are the same statement");
    assert_eq!(second.imported, 0);
    assert_eq!(second.statement_id, first.statement_id);
    assert_eq!(books.count("SELECT count(*) FROM bank_statement"), 1);
    assert_eq!(books.count("SELECT count(*) FROM bank_transaction"), 1);
}

#[test]
fn overlapping_ranges_do_not_duplicate_a_payment() {
    let books = Books::open();
    let shared = line("2026-01-31", 1_210_000_000, "Klant BV", "Betaling factuur 2026-0001");

    let january = statement(vec![shared.clone()]);
    let february = statement(vec![shared, line("2026-02-03", -1_000_000, "Bank", "Kosten")]);

    books.import(&january, "<january/>");
    let second = books.import(&february, "<february/>");

    assert!(second.created, "the file is new even though one line is not");
    assert_eq!(second.imported, 1);
    assert_eq!(second.duplicates, 1);
    assert_eq!(books.count("SELECT count(*) FROM bank_transaction"), 2);
}

#[test]
fn a_statement_that_does_not_add_up_is_refused_whole() {
    let books = Books::open();
    let mut broken = statement(vec![line("2026-01-15", 1_210_000_000, "Klant BV", "Betaling")]);
    broken.opening_balance_micros = Some(1_000_000_000);
    broken.closing_balance_micros = Some(2_000_000_000);

    let error = import_statement(
        books.database.connection(),
        ACCOUNT,
        &broken,
        &store::audit::sha256_hex(b"broken"),
        NOW,
    )
    .unwrap_err();

    assert!(matches!(error, StoreError::Rejected(_)), "{error}");
    assert!(error.to_string().contains("does not add up"), "{error}");
    assert_eq!(books.count("SELECT count(*) FROM bank_statement"), 0);
    assert_eq!(books.count("SELECT count(*) FROM bank_transaction"), 0);
}

#[test]
fn balances_that_agree_are_accepted() {
    let books = Books::open();
    let mut good = statement(vec![
        line("2026-01-15", 1_210_000_000, "Klant BV", "Betaling"),
        line("2026-01-20", -605_000_000, "Leverancier BV", "Huur"),
    ]);
    good.opening_balance_micros = Some(1_000_000_000);
    good.closing_balance_micros = Some(1_605_000_000);

    let report = books.import(&good, "<good/>");
    assert_eq!(report.balance_difference_micros, 0);
    assert_eq!(report.imported, 2);
}

#[test]
fn a_statement_for_an_unknown_account_is_refused() {
    let books = Books::open();
    let error = import_statement(
        books.database.connection(),
        "acct-unknown",
        &statement(vec![line("2026-01-15", 1, "x", "y")]),
        &store::audit::sha256_hex(b"x"),
        NOW,
    )
    .unwrap_err();
    assert!(matches!(error, StoreError::NotFound(_)), "{error}");
}

#[test]
fn transactions_are_immutable() {
    let books = Books::open();
    books.import(
        &statement(vec![line("2026-01-15", 1_210_000_000, "Klant BV", "Betaling")]),
        "<a/>",
    );
    let error = books
        .database
        .connection()
        .execute("DELETE FROM bank_transaction", [])
        .unwrap_err();
    assert!(error.to_string().contains("evidence"), "{error}");
}
