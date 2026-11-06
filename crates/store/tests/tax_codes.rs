#![allow(clippy::unwrap_used)]
//! Tax codes: the treatment a purchase is filed under, and the box it reaches.
//!
//! `AE` says "reverse charge" and nothing more. The Dutch return asks goods and
//! services in different boxes, so without a code every reverse-charged purchase
//! lands in 4a and 4b stays empty. These tests are about that one decision
//! reaching the posting, the report, and the evidence bundle.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocType, DocumentId, DraftLine, InvoiceDraft, Journal, LedgerEntry,
    LedgerLine, LineId, LocalDate, Money, Party, Quantity, SellerId, TaxTreatment, UnitCode, VatCategory, VatRate,
};
use store::vat::{Rubriek, compute};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, PostingSource, SellerRecord, content_hash,
    insert_customer, insert_draft, insert_seller, issue_draft, load_entry, post_document, post_outbound_document,
    seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T19:00:00.123456Z";

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
}

fn euros(units: i64) -> Money {
    Money::from_major_units(units, Currency::EUR).unwrap()
}

struct Books {
    dir: TempDir,
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
            dir,
            database,
            seller,
            customer,
        }
    }

    fn sell(&mut self, treatment: TaxTreatment, net_units: i64, on: &str) -> DocumentId {
        let mut draft = InvoiceDraft::new(self.seller, self.customer, Currency::EUR, date(on));
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

    /// A received invoice, recorded but not posted, so a code can be assigned.
    fn buy(&self, treatment: TaxTreatment, net_units: i64, on: &str, number: &str) -> DocumentId {
        let net = euros(net_units);
        let tax = if treatment.category == VatCategory::Standard {
            crux::tax_amount(net, treatment.rate).unwrap()
        } else {
            Money::zero(Currency::EUR)
        };
        let totals = crux::InvoiceTotals {
            net,
            tax,
            gross: net.checked_add(tax).unwrap(),
            breakdown: vec![crux::TaxBreakdown {
                category: treatment.category,
                rate_bp: treatment.rate.basis_points(),
                taxable_base: net,
                tax,
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

fn reverse_charge() -> TaxTreatment {
    TaxTreatment::zero(VatCategory::ReverseCharge)
}

#[test]
fn the_catalogue_names_the_box_each_code_resolves() {
    let books = Books::open();
    let codes = store::list_tax_codes(books.database.connection()).unwrap();
    assert_eq!(codes.len(), 3, "purchases are the only ambiguous side");

    let find = |code: &str| {
        codes
            .iter()
            .find(|entry| entry.code.as_str() == code)
            .unwrap_or_else(|| panic!("{code} is missing"))
    };
    assert_eq!(find("NL-AE-SERVICES").rubriek, Rubriek::ReverseChargeServices);
    assert_eq!(find("NL-AE-GOODS").rubriek, Rubriek::ReverseChargeGoods);
    assert_eq!(find("NL-K-GOODS").rubriek, Rubriek::EuAcquisitions);
    for code in &codes {
        assert!(
            matches!(code.category, VatCategory::ReverseCharge | VatCategory::IntraCommunity),
            "{code:?} resolves a category that is already unambiguous"
        );
    }
}

#[test]
fn a_reverse_charged_purchase_lands_in_the_box_its_code_names() {
    let books = Books::open();
    let plain = books.buy(reverse_charge(), 200, "2026-01-12", "SUP-1");
    let coded = books.buy(reverse_charge(), 200, "2026-01-13", "SUP-2");
    store::assign_tax_code(books.database.connection(), coded, "NL-AE-GOODS", "ada", NOW).unwrap();
    post_document(books.database.connection(), plain, "ada", NOW).unwrap();
    post_document(books.database.connection(), coded, "ada", NOW).unwrap();

    let report = compute(books.database.connection(), books.seller, 2026, 1).unwrap();

    // Without a code, `AE` keeps the safe default: 4a.
    let services = report.line(Rubriek::ReverseChargeServices).unwrap();
    assert_eq!(services.base_micros, 200_000_000, "only the uncoded purchase");
    assert_eq!(services.vat_micros, 42_000_000);

    // With the goods code, the same base and its VAT sit in 4b instead.
    let goods = report.line(Rubriek::ReverseChargeGoods).unwrap();
    assert_eq!(goods.base_micros, 200_000_000);
    assert_eq!(goods.vat_micros, 42_000_000);

    // The deduction stays in 5b: a code says where the supply belongs, not where
    // the input VAT belongs.
    let deductible = report.line(Rubriek::VatDeductible).unwrap();
    assert_eq!(deductible.vat_micros, 84_000_000, "both purchases deducted");
}

#[test]
fn a_code_is_refused_after_posting_because_the_entry_snapshots_it() {
    let books = Books::open();
    let purchase = books.buy(reverse_charge(), 200, "2026-01-12", "SUP-1");
    post_document(books.database.connection(), purchase, "ada", NOW).unwrap();

    let error = store::assign_tax_code(books.database.connection(), purchase, "NL-AE-GOODS", "ada", NOW).unwrap_err();
    assert!(
        error.to_string().contains("already posted"),
        "the message has to say why: {error}"
    );
}

#[test]
fn a_code_that_matches_no_line_of_the_document_is_refused() {
    let books = Books::open();
    let purchase = books.buy(reverse_charge(), 200, "2026-01-12", "SUP-1");

    // The document has no intra-community line, so this code would change nothing
    // and the user would believe it had.
    let error = store::assign_tax_code(books.database.connection(), purchase, "NL-K-GOODS", "ada", NOW).unwrap_err();
    assert!(error.to_string().contains("would change nothing"), "{error}");

    // A code the catalogue does not have, and one that is not a code at all, are
    // different mistakes and say so differently.
    let error = store::assign_tax_code(books.database.connection(), purchase, "NL-NOT-A-CODE", "ada", NOW).unwrap_err();
    assert!(matches!(error, store::StoreError::NotFound(_)), "{error}");
    let error = store::assign_tax_code(books.database.connection(), purchase, "nl-ae-goods", "ada", NOW).unwrap_err();
    assert!(matches!(error, store::StoreError::Rejected(_)), "{error}");
}

#[test]
fn only_received_documents_take_a_code() {
    let mut books = Books::open();
    let sale = books.sell(TaxTreatment::standard(VatRate::STANDARD_NL), 100, "2026-01-10");

    let error = store::assign_tax_code(books.database.connection(), sale, "NL-AE-GOODS", "ada", NOW).unwrap_err();
    assert!(error.to_string().contains("received documents"), "{error}");
}

#[test]
fn the_code_travels_on_the_entry_and_into_its_hash() {
    let books = Books::open();
    let purchase = books.buy(reverse_charge(), 200, "2026-01-12", "SUP-1");
    store::assign_tax_code(books.database.connection(), purchase, "NL-AE-GOODS", "ada", NOW).unwrap();
    let posted = post_document(books.database.connection(), purchase, "ada", NOW).unwrap();

    let stored = load_entry(books.database.connection(), posted.id).unwrap().unwrap();
    assert!(
        stored
            .entry
            .lines()
            .iter()
            .filter(|line| line.vat().is_some())
            .all(|line| line.tax_code().map(crux::TaxCode::as_str) == Some("NL-AE-GOODS")),
        "every line of the supply carries the treatment"
    );
    assert!(
        stored
            .entry
            .lines()
            .iter()
            .any(|line| line.vat().is_none() && line.tax_code().is_none()),
        "what we owe the supplier carries no treatment, and no code either"
    );

    // The stored hash is the hash of the facts as stored, code included.
    let source = PostingSource::InboundDocument(purchase);
    let recomputed = content_hash(&stored.entry, &stored.number, stored.entry_date, source).unwrap();
    assert_eq!(recomputed, stored.content_hash);

    // Strip the code and the entry means something else, which is why the code is
    // part of the hash rather than context around it.
    let stripped: Vec<LedgerLine> = stored
        .entry
        .lines()
        .iter()
        .map(|line| {
            let mut rebuilt = LedgerLine::new(line.account().clone(), line.side(), line.amount()).unwrap();
            if let Some(vat) = line.vat() {
                rebuilt = rebuilt.tagged(vat.category, VatRate::from_basis_points(vat.rate_bp).unwrap());
            }
            rebuilt
        })
        .collect();
    let uncoded = LedgerEntry::new(Journal::Purchase, stored.entry.description().to_owned(), stripped).unwrap();
    assert_ne!(
        content_hash(&uncoded, &stored.number, stored.entry_date, source).unwrap(),
        stored.content_hash
    );
}

#[test]
fn a_coded_purchase_bundles_and_verifies_offline() {
    let books = Books::open();
    let purchase = books.buy(reverse_charge(), 200, "2026-01-12", "SUP-1");
    store::assign_tax_code(books.database.connection(), purchase, "NL-AE-GOODS", "ada", NOW).unwrap();
    post_document(books.database.connection(), purchase, "ada", NOW).unwrap();

    let bundle = books.dir.path().join("bundle");
    let report = store::evidence::export(
        books.database.connection(),
        books.seller,
        purchase,
        &bundle,
        "test",
        NOW,
    )
    .unwrap();
    assert_eq!(report.ledger_entries, 1);

    // Verification re-derives the entry's hash from the bundle; if the code were
    // dropped from the record, this would fail.
    assert_eq!(store::evidence::verify(&bundle).unwrap(), report);
    let ledger = std::fs::read_to_string(bundle.join("ledger.jsonl")).unwrap();
    assert!(ledger.contains("\"tax_code_id\":\"NL-AE-GOODS\""), "{ledger}");
}

#[test]
fn a_posted_purchase_can_be_filed_under_another_code() {
    let books = Books::open();
    let connection = books.database.connection();
    let purchase = books.buy(reverse_charge(), 200, "2026-01-12", "SUP-1");
    let posted = post_document(connection, purchase, "ada", NOW).unwrap();
    assert!(
        compute(connection, books.seller, 2026, 1)
            .unwrap()
            .line(Rubriek::ReverseChargeServices)
            .is_some()
    );

    let corrected = store::reclassify(
        connection,
        posted.id,
        "NL-AE-GOODS",
        "verkeerde rubriek",
        date("2026-01-31"),
        "ada",
        NOW,
    )
    .unwrap();
    assert_eq!(corrected.original, "2026-0001");
    assert_eq!(corrected.reversal.journal, Journal::Purchase);
    assert_ne!(corrected.reversal.number, corrected.corrected.number);

    let report = compute(connection, books.seller, 2026, 1).unwrap();
    assert!(
        report.line(Rubriek::ReverseChargeServices).is_none(),
        "the reversal empties 4a"
    );
    let goods = report.line(Rubriek::ReverseChargeGoods).unwrap();
    assert_eq!(goods.base_micros, 200_000_000);
    assert_eq!(goods.vat_micros, 42_000_000);
    assert_eq!(
        report.line(Rubriek::VatDeductible).unwrap().vat_micros,
        42_000_000,
        "the deduction is untouched"
    );
    assert!(store::readiness(connection, books.seller, 2026, 1).unwrap().is_ready());

    // The replacement names the code on every line of the supply, so a report and
    // a bundle read the same treatment.
    let stored = load_entry(connection, corrected.corrected.id).unwrap().unwrap();
    assert!(
        stored
            .entry
            .lines()
            .iter()
            .filter(|line| line.vat().is_some())
            .all(|line| line.tax_code().map(crux::TaxCode::as_str) == Some("NL-AE-GOODS"))
    );

    // The pair is one fact in the chain, not two unexplained entries.
    let events: i64 = connection
        .query_row(
            "SELECT count(*) FROM audit_event WHERE kind = 'entry_reclassified'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);

    // A second correction is refused: the entry is already reversed.
    let error = store::reclassify(
        connection,
        posted.id,
        "NL-AE-GOODS",
        "again",
        date("2026-01-31"),
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(error.to_string().contains("already reversed"), "{error}");
}

#[test]
fn a_sale_is_not_reclassified_by_a_purchase_code() {
    let mut books = Books::open();
    let sale = books.sell(TaxTreatment::standard(VatRate::STANDARD_NL), 100, "2026-01-10");
    let posted = post_outbound_document(books.database.connection(), sale, "ada", NOW).unwrap();

    let error = store::reclassify(
        books.database.connection(),
        posted.id,
        "NL-AE-GOODS",
        "x",
        date("2026-01-31"),
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(error.to_string().contains("only a purchase"), "{error}");
}

#[test]
fn clearing_returns_the_document_to_its_own_category() {
    let books = Books::open();
    let purchase = books.buy(reverse_charge(), 200, "2026-01-12", "SUP-1");
    store::assign_tax_code(books.database.connection(), purchase, "NL-AE-GOODS", "ada", NOW).unwrap();
    assert!(
        store::assigned_tax_code(books.database.connection(), purchase)
            .unwrap()
            .is_some()
    );

    store::clear_tax_code(books.database.connection(), purchase, "ada", NOW).unwrap();
    assert!(
        store::assigned_tax_code(books.database.connection(), purchase)
            .unwrap()
            .is_none()
    );

    post_document(books.database.connection(), purchase, "ada", NOW).unwrap();
    let report = compute(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert!(
        report.line(Rubriek::ReverseChargeServices).is_some(),
        "with the code gone, the category alone decides again"
    );
    assert!(report.line(Rubriek::ReverseChargeGoods).is_none());
}

#[test]
fn a_sale_is_placed_by_its_category_and_needs_no_code() {
    let mut books = Books::open();
    let sale = books.sell(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-10");
    let posted = post_outbound_document(books.database.connection(), sale, "ada", NOW).unwrap();
    let stored = load_entry(books.database.connection(), posted.id).unwrap().unwrap();
    assert!(
        stored.entry.lines().iter().all(|line| line.tax_code().is_none()),
        "a sale is placed by its category and rate, so it has no code"
    );
}
