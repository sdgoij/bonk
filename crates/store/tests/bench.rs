#![allow(clippy::unwrap_used)]
//! The synthetic ledger behind the report benchmark.

use crux::{Address, CountryCode, Currency, LocalDate, Party, SellerId};
use store::{Database, KdfParams, KeyStore, SellerRecord, insert_seller, seed_starter_chart};
use tempfile::TempDir;

const NOW: &str = "2026-11-19T12:00:00.000000Z";

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
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

#[test]
fn the_synthetic_ledger_balances_and_refuses_a_second_run() {
    let books = Books::open();
    let lines = store::bench::seed_ledger(books.database.connection(), books.seller, 2026, 10).unwrap();
    assert_eq!(lines, 30, "three lines per synthetic sale");

    let report = store::trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-12-31"),
    )
    .unwrap();
    assert!(report.balances());
    let account = |code: &str| report.rows.iter().find(|row| row.code.as_str() == code).unwrap();
    assert_eq!(account("1300").debit_micros, 1_210_000_000);
    assert_eq!(account("8000").credit_micros, 1_000_000_000);
    assert_eq!(account("1600").credit_micros, 210_000_000);
    assert_eq!(
        store::bench::line_count(
            books.database.connection(),
            books.seller,
            date("2026-01-01"),
            date("2026-12-31")
        )
        .unwrap(),
        30
    );

    let again = store::bench::seed_ledger(books.database.connection(), books.seller, 2026, 10).unwrap_err();
    assert!(again.to_string().contains("empty"), "{again}");
}

#[test]
fn the_synthetic_ledger_refuses_an_archive_without_the_chart() {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
    let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();
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
    let error = store::bench::seed_ledger(database.connection(), seller, 2026, 1).unwrap_err();
    assert!(error.to_string().contains("does not exist"), "{error}");
}
