#![allow(clippy::unwrap_used)]
//! Opening balances: the documented file, the checks, and the one entry it posts.

use crux::{Address, CountryCode, Currency, LocalDate, Money, Party, SellerId, Side};
use store::{
    Database, KdfParams, KeyStore, SellerRecord, check_opening_balances, insert_seller, opening_balances_csv,
    post_opening_balances, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-19T12:00:00.000000Z";

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
}

fn euros(units: i64) -> Money {
    Money::from_major_units(units, Currency::EUR).unwrap()
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
                party: Party::new("Boeken BV", Address::new("Utrecht", CountryCode::NL)),
                default_currency: Currency::EUR,
                default_locale: "nl-NL".to_owned(),
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
}

const FILE: &str = "\u{feff}# Beginbalans 2026\naccount,side,amount\n1100,debit,5000.00\n0500,credit,5000.00\n";

#[test]
fn the_documented_file_parses_into_the_two_sides() {
    let balances = opening_balances_csv(FILE, Currency::EUR).unwrap();
    assert_eq!(balances.len(), 2);
    assert_eq!(balances[0].account.as_str(), "1100");
    assert_eq!(balances[0].side, Side::Debit);
    assert_eq!(balances[0].amount, euros(5_000));
    assert_eq!(balances[1].account.as_str(), "0500");
    assert_eq!(balances[1].side, Side::Credit);
    assert_eq!(balances[1].amount, euros(5_000));
}

#[test]
fn a_file_that_would_guess_is_refused() {
    let missing_header = opening_balances_csv("1100,debit,1.00\n", Currency::EUR).unwrap_err();
    assert!(missing_header.to_string().contains("header"), "{missing_header}");

    let duplicate = opening_balances_csv(
        "account,side,amount\n1100,debit,1.00\n1100,credit,1.00\n",
        Currency::EUR,
    )
    .unwrap_err();
    assert!(duplicate.to_string().contains("named twice"), "{duplicate}");

    let side = opening_balances_csv("account,side,amount\n1100,left,1.00\n", Currency::EUR).unwrap_err();
    assert!(side.to_string().contains("debit"), "{side}");

    let negative = opening_balances_csv("account,side,amount\n1100,debit,-1.00\n", Currency::EUR).unwrap_err();
    assert!(negative.to_string().contains("positive"), "{negative}");

    let empty = opening_balances_csv("account,side,amount\n", Currency::EUR).unwrap_err();
    assert!(empty.to_string().contains("no balances"), "{empty}");
}

#[test]
fn the_check_sums_the_sides_and_keeps_the_balance_sheet() {
    let books = Books::open();
    let balances = opening_balances_csv(FILE, Currency::EUR).unwrap();
    let summary = check_opening_balances(books.database.connection(), &balances).unwrap();
    assert_eq!(summary.debit, euros(5_000));
    assert_eq!(summary.credit, euros(5_000));
    assert!(summary.balances());

    let profit_and_loss = opening_balances_csv(
        "account,side,amount\n1100,debit,5000.00\n8000,credit,5000.00\n",
        Currency::EUR,
    )
    .unwrap();
    let error = check_opening_balances(books.database.connection(), &profit_and_loss).unwrap_err();
    assert!(error.to_string().contains("balance sheet"), "{error}");

    let unknown = opening_balances_csv("account,side,amount\n9998,debit,1.00\n", Currency::EUR).unwrap();
    let error = check_opening_balances(books.database.connection(), &unknown).unwrap_err();
    assert!(error.to_string().contains("does not exist"), "{error}");
}

#[test]
fn the_import_posts_one_entry_and_refuses_a_second() {
    let books = Books::open();
    let balances = opening_balances_csv(FILE, Currency::EUR).unwrap();

    let posted = post_opening_balances(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        &balances,
        "ada",
        NOW,
    )
    .unwrap();
    let stored = store::load_entry(books.database.connection(), posted.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.journal, crux::Journal::Opening);
    assert_eq!(stored.entry.description(), "Beginbalans 2026");
    assert_eq!(stored.entry.debit_total().unwrap(), euros(5_000));

    let again = post_opening_balances(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        &balances,
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(again.to_string().contains("already posted"), "{again}");
}

#[test]
fn an_unbalanced_file_is_refused_before_it_is_posted() {
    let books = Books::open();
    let balances = opening_balances_csv(
        "account,side,amount\n1100,debit,5000.00\n0500,credit,4000.00\n",
        Currency::EUR,
    )
    .unwrap();

    let summary = check_opening_balances(books.database.connection(), &balances).unwrap();
    assert!(!summary.balances());
    assert_eq!(summary.debit, euros(5_000));
    assert_eq!(summary.credit, euros(4_000));

    let error = post_opening_balances(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        &balances,
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(error.to_string().contains("do not balance"), "{error}");
    let entries: i64 = books
        .database
        .connection()
        .query_row("SELECT count(*) FROM journal_entry", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entries, 0, "a refused import writes nothing");
}
