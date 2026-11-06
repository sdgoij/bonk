#![allow(clippy::unwrap_used)]
//! Memorial entries: the correction a person writes.
//!
//! The memorial journal is the escape hatch for what no rule can derive. Its
//! lines carry the treatment and the tax code because that is what a report reads
//! a box from; a correction that only moved amounts between accounts could not fix
//! a supply filed in the wrong box.

use crux::{
    AccountCode, Address, CountryCode, Currency, CustomerId, DocType, DocumentId, Journal, LedgerLine, LocalDate,
    Money, Party, SellerId, Side, TaxCode, VatCategory, VatRate,
};
use store::vat::{Rubriek, compute};
use store::{
    CustomerRecord, Database, KdfParams, KeyStore, SellerRecord, insert_customer, insert_seller, post_document,
    post_manual_entry, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T20:00:00.123456Z";

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
}

fn euros(units: i64) -> Money {
    Money::from_major_units(units, Currency::EUR).unwrap()
}

fn line(account: &str, side: Side, amount: &str) -> LedgerLine {
    LedgerLine::new(
        AccountCode::parse(account).unwrap(),
        side,
        Money::parse_fixed(amount, Currency::EUR).unwrap(),
    )
    .unwrap()
}

fn tagged(line: LedgerLine, category: VatCategory, rate: &str) -> LedgerLine {
    line.tagged(category, VatRate::parse_percent(rate).unwrap())
}

fn coded(line: LedgerLine, code: &str) -> LedgerLine {
    line.coded(TaxCode::parse(code).unwrap())
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
                party: Party::new("Leverancier BV", Address::new("Rotterdam", CountryCode::NL)),
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

    /// A reverse-charged purchase: the supplier charged no VAT, so the gross is
    /// the net and we assess the VAT ourselves.
    fn buy_reverse_charged(&self, net_units: i64, on: &str, number: &str) -> DocumentId {
        let net = euros(net_units);
        let totals = crux::InvoiceTotals {
            net,
            tax: Money::zero(Currency::EUR),
            gross: net,
            breakdown: vec![crux::TaxBreakdown {
                category: VatCategory::ReverseCharge,
                rate_bp: 0,
                taxable_base: net,
                tax: Money::zero(Currency::EUR),
            }],
        };
        let issuer = Party::new("Leverancier BV", Address::new("Rotterdam", CountryCode::NL));
        let seller = store::load_seller(self.database.connection(), self.seller).unwrap();
        let received = store::ReceivedDocument {
            number,
            doc_type: DocType::Invoice,
            currency: Currency::EUR,
            issue_date: date(on),
            due_date: None,
            delivery_date: None,
            buyer_reference: None,
            order_reference: None,
            note: None,
            issuer: &issuer,
            recipient: &seller.party,
            lines: &[],
            totals: &totals,
            preceding_number: None,
            preceding_issue_date: None,
        };
        let payload = format!("<Invoice>{number}</Invoice>");
        store::record_received(
            self.database.connection(),
            self.seller,
            Some(self.customer),
            &received,
            &store::ImportSource {
                kind: "ubl",
                path: None,
                payload_hash: &store::audit::sha256_hex(payload.as_bytes()),
                payload: payload.as_bytes(),
            },
            None,
            NOW,
        )
        .unwrap();
        let id: String = self
            .database
            .connection()
            .query_row(
                "SELECT id FROM document WHERE direction = 'inbound' AND number = ?1",
                [number],
                |row| row.get(0),
            )
            .unwrap();
        DocumentId::parse_str(&id).unwrap()
    }
}

#[test]
fn a_memorial_entry_balances_the_books() {
    let books = Books::open();
    let posted = post_manual_entry(
        books.database.connection(),
        books.seller,
        date("2026-01-31"),
        "Bankkosten januari",
        vec![line("7100", Side::Debit, "12.50"), line("1100", Side::Credit, "12.50")],
        "ada",
        NOW,
    )
    .unwrap();

    assert_eq!(posted.journal, Journal::Memorial);
    assert_eq!(posted.number, "2026-0001", "numbered per journal and year");
    assert!(posted.created);

    let balance = store::trial_balance(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-12-31"),
    )
    .unwrap();
    assert!(balance.balances());
    assert!(balance.rows.iter().any(|row| row.code.as_str() == "7100"));
}

#[test]
fn an_unbalanced_memorial_entry_is_refused() {
    let books = Books::open();
    let error = post_manual_entry(
        books.database.connection(),
        books.seller,
        date("2026-01-31"),
        "Half a correction",
        vec![line("7100", Side::Debit, "12.50")],
        "ada",
        NOW,
    )
    .unwrap_err();

    assert!(error.to_string().contains("does not balance"), "{error}");
    assert!(error.to_string().contains("12.50"), "the amount is named: {error}");
}

#[test]
fn a_memorial_entry_needs_a_description() {
    let books = Books::open();
    let error = post_manual_entry(
        books.database.connection(),
        books.seller,
        date("2026-01-31"),
        "   ",
        vec![line("7100", Side::Debit, "12.50"), line("1100", Side::Credit, "12.50")],
        "ada",
        NOW,
    )
    .unwrap_err();

    assert!(error.to_string().contains("description"), "{error}");
}

#[test]
fn a_memorial_line_naming_an_unknown_account_is_refused() {
    let books = Books::open();
    let error = post_manual_entry(
        books.database.connection(),
        books.seller,
        date("2026-01-31"),
        "Typo",
        vec![line("7109", Side::Debit, "12.50"), line("1100", Side::Credit, "12.50")],
        "ada",
        NOW,
    )
    .unwrap_err();

    assert!(error.to_string().contains("7109"), "{error}");
}

#[test]
fn a_memorial_correction_moves_a_reverse_charged_purchase_to_4b() {
    let books = Books::open();
    let connection = books.database.connection();
    let purchase = books.buy_reverse_charged(200, "2026-01-12", "SUP-1");
    let posted = post_document(connection, purchase, "ada", NOW).unwrap();

    // As posted, the purchase is in 4a: the category cannot say otherwise.
    let report = compute(connection, books.seller, 2026, 1).unwrap();
    assert_eq!(
        report.line(Rubriek::ReverseChargeServices).unwrap().base_micros,
        200_000_000
    );

    // The correction: undo the entry, then book the same amounts as goods.
    store::reverse_entry(
        connection,
        posted.id,
        "verkeerde rubriek",
        date("2026-01-31"),
        "ada",
        NOW,
    )
    .unwrap();
    post_manual_entry(
        connection,
        books.seller,
        date("2026-01-31"),
        "Herindeling SUP-1 naar 4b",
        vec![
            coded(
                tagged(line("9999", Side::Debit, "200.00"), VatCategory::ReverseCharge, "21"),
                "NL-AE-GOODS",
            ),
            coded(
                tagged(line("1500", Side::Debit, "42.00"), VatCategory::ReverseCharge, "21"),
                "NL-AE-GOODS",
            ),
            coded(
                tagged(line("1620", Side::Credit, "42.00"), VatCategory::ReverseCharge, "21"),
                "NL-AE-GOODS",
            ),
            line("1400", Side::Credit, "200.00"),
        ],
        "ada",
        NOW,
    )
    .unwrap();

    let report = compute(connection, books.seller, 2026, 1).unwrap();
    assert!(
        report.line(Rubriek::ReverseChargeServices).is_none(),
        "4a is back to nothing"
    );
    let goods = report.line(Rubriek::ReverseChargeGoods).unwrap();
    assert_eq!(goods.base_micros, 200_000_000);
    assert_eq!(goods.vat_micros, 42_000_000);
    assert_eq!(
        report.line(Rubriek::VatDeductible).unwrap().vat_micros,
        42_000_000,
        "the deduction was never in question"
    );

    // The period is fileable again, which is the point of the exercise.
    let readiness = store::readiness(connection, books.seller, 2026, 1).unwrap();
    assert!(readiness.is_ready(), "{:?}", readiness.blockers());
}

#[test]
fn a_memorial_line_on_the_other_rate_account_needs_a_code() {
    let books = Books::open();
    let error = post_manual_entry(
        books.database.connection(),
        books.seller,
        date("2026-01-31"),
        "Correction",
        vec![
            line("9999", Side::Debit, "200.00"),
            tagged(line("1500", Side::Debit, "42.00"), VatCategory::ReverseCharge, "21"),
            tagged(line("1620", Side::Credit, "42.00"), VatCategory::ReverseCharge, "21"),
            line("1400", Side::Credit, "200.00"),
        ],
        "ada",
        NOW,
    )
    .unwrap_err();

    assert!(error.to_string().contains("needs a tax code"), "{error}");
}

#[test]
fn a_memorial_code_must_match_the_line_it_names() {
    let books = Books::open();
    let error = post_manual_entry(
        books.database.connection(),
        books.seller,
        date("2026-01-31"),
        "Correction",
        vec![
            coded(
                tagged(line("9999", Side::Debit, "200.00"), VatCategory::IntraCommunity, "21"),
                "NL-AE-GOODS",
            ),
            line("1400", Side::Credit, "200.00"),
        ],
        "ada",
        NOW,
    )
    .unwrap_err();

    assert!(error.to_string().contains("resolves category AE"), "{error}");
    assert!(error.to_string().contains("carries K"), "{error}");
}
