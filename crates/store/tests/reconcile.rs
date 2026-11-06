#![allow(clippy::unwrap_used)]
//! What has to hold before a VAT return may be marked prepared, and what happens
//! when it does not.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocType, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money,
    Party, Quantity, SellerId, TaxTreatment, UnitCode, VatCategory, VatRate,
};
use store::reconcile::{Check, readiness};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, StoreError, insert_customer,
    insert_draft, insert_seller, issue_draft, post_document, post_outbound_document, prepare, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T17:00:00.123456Z";

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

    /// Issues a document without posting it, and returns its id.
    fn issue(&mut self, treatment: TaxTreatment, net_units: i64, on: &str) -> DocumentId {
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

    fn receive(&mut self, treatment: TaxTreatment, net_units: i64, on: &str, number: &str) -> DocumentId {
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
            None,
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
fn a_fully_posted_period_is_ready_except_for_the_control_accounts() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-15");
    let note = books.issue(TaxTreatment::standard(VatRate::REDUCED_NL), 500, "2026-01-20");
    let purchase = books.receive(TaxTreatment::standard(VatRate::STANDARD_NL), 400, "2026-01-10", "SUP-1");
    let reverse = books.receive(
        TaxTreatment::zero(VatCategory::ReverseCharge),
        200,
        "2026-01-12",
        "SUP-2",
    );

    for document in [invoice, note, purchase, reverse] {
        post_document(books.database.connection(), document, "ada", NOW).unwrap();
    }

    let ready = readiness(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert!(ready.is_ready(), "{:?}", ready.blockers());
    assert_eq!(ready.ledger_to_return, Check::Passed);
    assert_eq!(ready.invoice_to_ledger, Check::Passed);
    assert_eq!(ready.balance_sheet, Check::Passed);
    assert_eq!(
        ready.control_accounts,
        Check::NotChecked("open items are not implemented yet")
    );
    assert_eq!(ready.unchecked().len(), 1, "the gap is reported, not hidden");
}

#[test]
fn an_unposted_document_blocks_the_return() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-15");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();
    books.issue(TaxTreatment::standard(VatRate::STANDARD_NL), 500, "2026-01-20");

    let ready = readiness(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert!(!ready.is_ready());
    let blockers = ready.blockers();
    assert_eq!(blockers.len(), 1, "{blockers:?}");
    assert!(blockers[0].contains("is not posted"), "{blockers:?}");

    // And preparing refuses, naming the document.
    let error = prepare(books.database.connection(), books.seller, 2026, 1, "ada", NOW).unwrap_err();
    assert!(matches!(error, StoreError::Rejected(_)), "{error}");
    assert!(error.to_string().contains("not ready to be filed"), "{error}");

    let stored: i64 = books
        .database
        .connection()
        .query_row("SELECT count(*) FROM vat_return", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored, 0, "a refused preparation writes nothing");
}

#[test]
fn vat_that_disagrees_with_its_document_blocks_the_return() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-15");
    let posted = post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    // A balanced pair of lines added behind the rules' back: the trial balance
    // still nets to zero, so only the document comparison can catch this.
    books
        .database
        .connection()
        .execute(
            "INSERT INTO journal_line
                 (id, entry_id, line_no, account_code, side, amount_micros, currency,
                  base_amount_micros, vat_category, vat_rate_bp)
             VALUES ('tamper-1', ?1, 90, '1600', 'debit', 10000000, 'EUR', 10000000, 'S', 2100),
                    ('tamper-2', ?1, 91, '1100', 'credit', 10000000, 'EUR', 10000000, NULL, NULL)",
            [posted.id.to_string()],
        )
        .unwrap();

    let ready = readiness(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert_eq!(ready.balance_sheet, Check::Passed, "the books still balance");
    let failures = ready.invoice_to_ledger.failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("2026-0001"), "{failures:?}");
    assert!(failures[0].contains("the document says"), "{failures:?}");
}

#[test]
fn a_credit_note_is_compared_on_its_own_terms() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-15");
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let draft = store::credit_note_for(
        books.database.connection(),
        books.seller,
        "2026-0001",
        date("2026-01-31"),
    )
    .unwrap()
    .unwrap();
    store::insert_draft(books.database.connection(), &draft, NOW).unwrap();
    let note = issue_draft(
        books.database.connection_mut(),
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
    post_outbound_document(books.database.connection(), note.id, "ada", NOW).unwrap();

    let ready = readiness(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert!(ready.is_ready(), "{:?}", ready.blockers());
}
