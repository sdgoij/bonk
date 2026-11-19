#![allow(clippy::unwrap_used)]
//! The intra-EU supplies listing (`Opgaaf ICP`): rubriek 3a grouped by buyer.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money, Party,
    Quantity, SellerId, TaxTreatment, UnitCode, VatCategory, VatRate,
};
use store::vat::{Rubriek, compute, icp};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, insert_customer, insert_draft,
    insert_seller, issue_draft, post_outbound_document, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T16:00:00.123456Z";

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
}

fn euros(units: i64) -> Money {
    Money::from_major_units(units, Currency::EUR).unwrap()
}

fn country(code: &str) -> CountryCode {
    CountryCode::parse(code).unwrap()
}

struct Books {
    _dir: TempDir,
    database: Database,
    seller: SellerId,
    domestic: CustomerId,
    german: CustomerId,
    french: CustomerId,
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

        let domestic = add_customer(&database, "Klant BV", "NL", None);
        let german = add_customer(&database, "Kunde GmbH", "DE", Some("DE123456789"));
        let french = add_customer(&database, "Client SA", "FR", Some("FR 12 345 678 901"));
        Self {
            _dir: dir,
            database,
            seller,
            domestic,
            german,
            french,
        }
    }

    fn sell_to(&mut self, customer: CustomerId, treatment: TaxTreatment, net_units: i64, on: &str) -> DocumentId {
        let mut draft = InvoiceDraft::new(self.seller, customer, Currency::EUR, date(on));
        // Intra-community supplies need a delivery date; giving every sale one is
        // harmless and keeps the fixture from tripping BR-IC-11.
        draft.delivery_date = Some(date(on));
        draft.lines.push(DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Consulting".to_owned(),
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: euros(net_units),
            price_base_quantity: Quantity::ONE,
            treatment,
        });
        insert_draft(self.database.connection(), &draft, NOW).unwrap();
        let issued = issue_draft(
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
        .unwrap();
        post_outbound_document(self.database.connection(), issued.id, "ada", NOW).unwrap();
        issued.id
    }

    fn credit_note(&mut self, number: &str, net_units: i64, on: &str) {
        let mut draft = store::credit_note_for(self.database.connection(), self.seller, number, date(on))
            .unwrap()
            .unwrap();
        draft.lines[0].unit_price = euros(net_units);
        store::insert_draft(self.database.connection(), &draft, NOW).unwrap();
        let note = issue_draft(
            self.database.connection_mut(),
            IssueRequest {
                draft_id: draft.id,
                actor: "ada",
                device: "test",
                renderer_version: "test",
                now: NOW,
            },
            |_| Ok(b"<CreditNote/>".to_vec()),
        )
        .unwrap();
        post_outbound_document(self.database.connection(), note.id, "ada", NOW).unwrap();
    }
}

fn add_customer(database: &Database, name: &str, country_code: &str, vat_id: Option<&str>) -> CustomerId {
    let id = CustomerId::new();
    let mut party = Party::new(name, Address::new("Stad", country(country_code)));
    party.vat_id = vat_id.map(str::to_owned);
    insert_customer(database.connection(), &CustomerRecord { id, party }, NOW).unwrap();
    id
}

#[test]
fn the_listing_groups_intra_eu_supplies_by_buyer() {
    let mut books = Books::open();
    books.sell_to(
        books.german,
        TaxTreatment::zero(VatCategory::IntraCommunity),
        1_000,
        "2026-01-10",
    );
    books.sell_to(
        books.french,
        TaxTreatment::zero(VatCategory::ReverseCharge),
        500,
        "2026-01-12",
    );
    books.sell_to(
        books.domestic,
        TaxTreatment::standard(VatRate::STANDARD_NL),
        2_000,
        "2026-01-14",
    );
    books.sell_to(
        books.domestic,
        TaxTreatment::zero(VatCategory::Export),
        300,
        "2026-01-16",
    );

    let listing = icp(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert!(listing.is_ready(), "{:?}", listing.problems);
    assert_eq!(listing.rows.len(), 2, "domestic and export supplies are not ICP");

    assert_eq!(listing.rows[0].country, "DE");
    assert_eq!(listing.rows[0].vat_id, "DE123456789");
    assert_eq!(listing.rows[0].customer, "Kunde GmbH");
    assert_eq!(listing.rows[0].supplies_micros, 1_000_000_000);

    // The identifier is stored as the document recorded it, but the separators a
    // person typed are not part of it.
    assert_eq!(listing.rows[1].country, "FR");
    assert_eq!(listing.rows[1].vat_id, "FR12345678901");
    assert_eq!(listing.rows[1].supplies_micros, 500_000_000);

    assert_eq!(listing.total_micros(), 1_500_000_000);
    assert_eq!(listing.basis.len(), 2, "one entry per EU sale");

    // The listing is box 3a, and it has to say the same thing.
    let report = compute(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert_eq!(
        report.line(Rubriek::EuSupplies).unwrap().base_micros,
        listing.total_micros()
    );
}

#[test]
fn a_credit_note_reduces_the_buyer_s_line() {
    let mut books = Books::open();
    books.sell_to(
        books.german,
        TaxTreatment::zero(VatCategory::IntraCommunity),
        1_000,
        "2026-01-10",
    );
    books.credit_note("2026-0001", 400, "2026-01-31");

    let listing = icp(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert!(listing.is_ready(), "{:?}", listing.problems);
    assert_eq!(listing.rows.len(), 1);
    assert_eq!(listing.rows[0].supplies_micros, 600_000_000);

    let report = compute(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert_eq!(report.line(Rubriek::EuSupplies).unwrap().base_micros, 600_000_000);
}

#[test]
fn a_buyer_without_a_vat_id_blocks_the_listing() {
    let mut books = Books::open();
    // Reverse-charged services go to 3a, and this customer has no VAT identifier,
    // which is exactly the case the listing must not drop.
    books.sell_to(
        books.domestic,
        TaxTreatment::zero(VatCategory::ReverseCharge),
        400,
        "2026-01-20",
    );

    let listing = icp(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert!(!listing.is_ready());
    assert!(listing.rows.is_empty(), "an unnameable buyer is not a row");
    assert_eq!(listing.problems.len(), 1);
    assert_eq!(listing.problems[0].number, "2026-0001");
    assert!(
        listing.problems[0].reason.contains("VAT identifier"),
        "{}",
        listing.problems[0].reason
    );

    // Box 3a still carries it, which is why the listing refuses instead of
    // reporting a total that does not match the return.
    let report = compute(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert_eq!(report.line(Rubriek::EuSupplies).unwrap().base_micros, 400_000_000);
}

#[test]
fn a_month_without_postings_has_an_empty_listing() {
    let books = Books::open();
    let listing = icp(books.database.connection(), books.seller, 2026, 5).unwrap();
    assert!(listing.rows.is_empty());
    assert!(listing.problems.is_empty());
    assert!(listing.basis.is_empty());
    assert_eq!(listing.total_micros(), 0);
}
