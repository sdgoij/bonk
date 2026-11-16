#![allow(clippy::unwrap_used)]
//! Foreign-currency postings: a document keeps its own currency, the ledger
//! keeps the reporting currency, and the rate travels with the posting.

use crux::{
    AccountKind, Address, CountryCode, Currency, CustomerId, DraftLine, ExchangeRate, InvoiceDraft, LineId, LocalDate,
    Money, Party, Quantity, SellerId, TaxTreatment, UnitCode, VatRate,
};
use store::open_items::{OpenItemKind, list};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, insert_customer, insert_draft,
    insert_seller, issue_draft, post_outbound_document, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-16T12:00:00.000000Z";

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
}

fn eur_micros(micros: i64) -> Money {
    Money::from_micros(micros, Currency::EUR)
}

fn pounds(units: i64) -> Money {
    Money::from_major_units(units, Currency::GBP).unwrap()
}

struct Books {
    _dir: TempDir,
    database: Database,
    seller: SellerId,
    customer: CustomerId,
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
        let customer = CustomerId::new();
        insert_customer(
            database.connection(),
            &CustomerRecord {
                id: customer,
                party: Party::new("Klant BV", Address::new("Amsterdam", CountryCode::NL)),
            },
            NOW,
        )
        .unwrap();
        Self {
            _dir: dir,
            database,
            seller,
            customer,
        }
    }

    fn rate(&self, currency: Currency, as_of: &str, micros: i64) {
        store::set_manual_rate(
            self.database.connection(),
            self.seller,
            currency,
            date(as_of),
            ExchangeRate::from_per_unit_micros(micros).unwrap(),
            "ada",
            NOW,
        )
        .unwrap();
    }

    fn issue(&mut self, currency: Currency, net_units: i64, on: &str, due: &str) -> crux::DocumentId {
        let mut draft = InvoiceDraft::new(self.seller, self.customer, currency, date(on));
        draft.due_date = Some(date(due));
        draft.lines.push(DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Consulting".to_owned(),
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(net_units, currency).unwrap(),
            price_base_quantity: Quantity::ONE,
            treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
        });
        insert_draft(self.database.connection(), &draft, NOW).unwrap();
        issue_draft(
            self.database.connection_mut(),
            IssueRequest {
                draft_id: draft.id,
                actor: "ada",
                device: "test",
                renderer_version: "test",
                now: NOW,
            },
            |_| Ok(b"<Invoice/>".to_vec()),
        )
        .unwrap()
        .id
    }
}

#[test]
fn a_foreign_invoice_posts_in_both_currencies() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    let posted = post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let stored = store::load_entry(books.database.connection(), posted.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.entry.currency(), Currency::GBP);
    let fx = stored.entry.fx().expect("the entry names its rate");
    assert_eq!(fx.base_currency, Currency::EUR);
    assert_eq!(fx.rate.per_unit_micros(), 1_170_000);

    // The debtor is the gross in the reporting currency; revenue and VAT keep
    // their historical amounts.
    let balance = store::trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-12-31"),
    )
    .unwrap();
    assert!(balance.balances(), "the reporting-currency columns must agree");
    let debtor = balance.rows.iter().find(|row| row.code.as_str() == "1300").unwrap();
    assert_eq!(debtor.balance_micros(), 1_415_700_000, "£1 210 at 1.17");
    let revenue = balance.rows.iter().find(|row| row.code.as_str() == "8000").unwrap();
    assert_eq!(revenue.balance_micros(), 1_170_000_000);
    let vat = balance.rows.iter().find(|row| row.code.as_str() == "1600").unwrap();
    assert_eq!(vat.balance_micros(), 245_700_000);

    // The subledger keeps the document currency and snapshots the carrying value.
    let items = list(books.database.connection(), books.seller, OpenItemKind::Receivable).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].amount, pounds(1_210));
    assert_eq!(items[0].base_amount, eur_micros(1_415_700_000));
    assert_eq!(items[0].fx_rate_micros, Some(1_170_000));
}

#[test]
fn posting_without_a_rate_is_refused_and_names_the_fix() {
    let mut books = Books::open();
    let invoice = books.issue(Currency::GBP, 100, "2026-01-10", "2026-02-09");
    let error = post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("no rate"), "{text}");
    assert!(text.contains("fx import"), "{text}");
    assert!(text.contains("fx override"), "{text}");
}

#[test]
fn an_override_wins_over_the_cache_and_is_part_of_the_posting() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    store::set_override(
        books.database.connection(),
        books.seller,
        invoice,
        ExchangeRate::from_per_unit_micros(1_200_000).unwrap(),
        date("2026-01-10"),
        "ada",
        NOW,
    )
    .unwrap();
    let posted = post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();
    let stored = store::load_entry(books.database.connection(), posted.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.entry.fx().unwrap().rate.per_unit_micros(), 1_200_000);

    // An override cannot be changed after the posting: the entry is append-only.
    let refused = store::set_override(
        books.database.connection(),
        books.seller,
        invoice,
        ExchangeRate::from_per_unit_micros(1_300_000).unwrap(),
        date("2026-01-10"),
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(refused.to_string().contains("posted"), "{refused}");
}

#[test]
fn a_reversal_keeps_the_rate_and_nets_the_books_to_zero() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    let posted = post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();
    let reversal = store::reverse_entry(
        books.database.connection(),
        posted.id,
        "wrong customer",
        date("2026-01-31"),
        "ada",
        NOW,
    )
    .unwrap();

    let original_fx = store::load_entry(books.database.connection(), posted.id)
        .unwrap()
        .unwrap()
        .entry
        .fx()
        .unwrap();
    let stored = store::load_entry(books.database.connection(), reversal.id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.entry.fx(), Some(original_fx));
    let balance = store::trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-12-31"),
    )
    .unwrap();
    assert!(balance.balances());
    assert!(
        balance.rows.iter().all(|row| row.balance_micros() == 0),
        "{:?}",
        balance.rows
    );
}

#[test]
fn an_ecb_file_is_crossed_into_the_reporting_currency() {
    let books = Books::open();
    let file = b"Date,USD,GBP\n2026-01-02,1.08,0.85\n2026-01-03,1.09,0.86\n";
    let report = store::import_ecb(books.database.connection(), books.seller, file, "ada", NOW).unwrap();
    assert_eq!(report.rows, 2);
    assert_eq!(report.currencies, 2);
    assert_eq!(report.provider_version.len(), 64);

    // 1 / 1.08 = 0.9259259..., rounded once by the ledger's money rule; by the
    // third the file quotes 1.09, which the later dates take.
    let dollar = store::rate_on(
        books.database.connection(),
        books.seller,
        Currency::USD,
        date("2026-01-02"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(dollar.per_unit_micros(), 925_926);
    let latest = store::rate_on(
        books.database.connection(),
        books.seller,
        Currency::USD,
        date("2026-01-05"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(latest.per_unit_micros(), 917_431);
    let pound = store::rate_on(
        books.database.connection(),
        books.seller,
        Currency::GBP,
        date("2026-01-03"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(pound.per_unit_micros(), 1_162_791);

    // A weekend takes the last published reference rate; before the file starts
    // there is no rate at all.
    assert_eq!(
        store::rate_on(
            books.database.connection(),
            books.seller,
            Currency::USD,
            date("2026-01-04")
        )
        .unwrap()
        .unwrap()
        .per_unit_micros(),
        917_431
    );
    assert!(
        store::rate_on(
            books.database.connection(),
            books.seller,
            Currency::USD,
            date("2026-01-01")
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(
        store::list_rates(books.database.connection(), books.seller, None)
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn a_books_reporting_outside_the_euro_gets_its_euro_leg() {
    let books = Books::open();
    let other = SellerId::new();
    insert_seller(
        books.database.connection(),
        &SellerRecord {
            id: other,
            party: Party::new("Dollar Co", Address::new("Boston", CountryCode::parse("US").unwrap())),
            default_currency: Currency::USD,
            default_locale: "en-US".to_owned(),
        },
        NOW,
    )
    .unwrap();
    let file = b"Date,GBP,USD\n2026-01-02,0.85,1.08\n";
    store::import_ecb(books.database.connection(), other, file, "ada", NOW).unwrap();

    // USD per GBP = 1.08 / 0.85; USD per EUR = 1.08, because the file has no
    // euro column: the euro is its unit.
    let pound = store::rate_on(books.database.connection(), other, Currency::GBP, date("2026-01-03"))
        .unwrap()
        .unwrap();
    assert_eq!(pound.per_unit_micros(), 1_270_588);
    let euro = store::rate_on(books.database.connection(), other, Currency::EUR, date("2026-01-03"))
        .unwrap()
        .unwrap();
    assert_eq!(euro.per_unit_micros(), 1_080_000);
    let dollar = store::rate_on(books.database.connection(), other, Currency::USD, date("2026-01-03"))
        .unwrap()
        .unwrap();
    assert_eq!(dollar.per_unit_micros(), 1_000_000);
}

#[test]
fn a_posted_foreign_document_verifies_offline() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let bundle = TempDir::new().unwrap();
    store::evidence::export(
        books.database.connection(),
        books.seller,
        invoice,
        bundle.path(),
        "ada",
        NOW,
    )
    .unwrap();
    let report = store::evidence::verify(bundle.path()).unwrap();
    assert_eq!(report.ledger_entries, 1);
}

#[test]
fn a_rebuild_uses_the_rate_the_posting_used() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    // An older archive has the entry and no subledger row. The cache then moves,
    // but a repair must reproduce the posting's rate rather than today's.
    let connection = books.database.connection();
    connection.execute("DROP TRIGGER open_item_no_delete", []).unwrap();
    connection.execute("DELETE FROM open_item", []).unwrap();
    books.rate(Currency::GBP, "2026-01-11", 1_300_000);

    let report = store::rebuild_open_items(connection, books.seller, "ada", NOW).unwrap();
    assert_eq!(report.added, 1);
    let items = list(connection, books.seller, OpenItemKind::Receivable).unwrap();
    assert_eq!(items[0].fx_rate_micros, Some(1_170_000));
    assert_eq!(items[0].base_amount, eur_micros(1_415_700_000));
}

#[test]
fn the_control_account_check_uses_the_carrying_value() {
    let mut books = Books::open();
    books.rate(Currency::GBP, "2026-01-01", 1_170_000);
    let invoice = books.issue(Currency::GBP, 1_000, "2026-01-10", "2026-02-09");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let readiness = store::readiness(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert!(
        matches!(readiness.control_accounts, store::Check::Passed),
        "{:?}",
        readiness.control_accounts
    );
    assert!(
        matches!(readiness.invoice_to_ledger, store::Check::Passed),
        "the document's VAT and the posted VAT are compared after conversion: {:?}",
        readiness.invoice_to_ledger
    );
    assert!(
        matches!(readiness.ledger_to_return, store::Check::Passed),
        "{:?}",
        readiness.ledger_to_return
    );
    assert!(
        matches!(readiness.balance_sheet, store::Check::Passed),
        "{:?}",
        readiness.balance_sheet
    );
    assert_eq!(
        store::load_account(books.database.connection(), &crux::AccountCode::parse("1300").unwrap())
            .unwrap()
            .unwrap()
            .kind,
        AccountKind::Asset
    );
}
