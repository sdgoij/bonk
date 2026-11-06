#![allow(clippy::unwrap_used)]
//! A quarter's VAT return: every rubriek the shipped rule set can produce, the
//! basis it is computed from, and what preparing it twice does.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocType, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money,
    Party, Quantity, SellerId, TaxTreatment, UnitCode, VatCategory, VatRate,
};
use store::vat::{Rubriek, compute, prepare, prepared};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, insert_customer, insert_draft,
    insert_seller, issue_draft, post_document, post_outbound_document, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T16:00:00.123456Z";

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

    fn buy(&mut self, treatment: TaxTreatment, net_units: i64, on: &str, number: &str) {
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
        post_document(
            self.database.connection(),
            DocumentId::parse_str(&id).unwrap(),
            "ada",
            NOW,
        )
        .unwrap();
    }

    /// A traded quarter: five sales across the treatments the rules support and
    /// three purchases.
    fn quarter(&mut self) {
        self.sell(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-15");
        self.sell(TaxTreatment::standard(VatRate::REDUCED_NL), 2_000, "2026-01-20");
        self.sell(TaxTreatment::zero(VatCategory::ReverseCharge), 500, "2026-01-22");
        self.sell(TaxTreatment::zero(VatCategory::Export), 300, "2026-01-25");
        self.sell(TaxTreatment::zero(VatCategory::Exempt), 100, "2026-01-28");

        self.buy(TaxTreatment::standard(VatRate::STANDARD_NL), 400, "2026-01-10", "SUP-1");
        self.buy(
            TaxTreatment::zero(VatCategory::ReverseCharge),
            200,
            "2026-01-12",
            "SUP-2",
        );
        self.buy(
            TaxTreatment::zero(VatCategory::IntraCommunity),
            100,
            "2026-01-14",
            "SUP-3",
        );
    }
}

#[test]
fn a_quarter_reports_every_rubriek_the_rules_can_produce() {
    let mut books = Books::open();
    books.quarter();

    let report = compute(books.database.connection(), books.seller, 2026, 1).unwrap();
    let amount = |rubriek: Rubriek| {
        let line = report
            .line(rubriek)
            .unwrap_or_else(|| panic!("{} is missing", rubriek.code()));
        (line.base_micros / 10_000, line.vat_micros / 10_000)
    };

    assert_eq!(amount(Rubriek::HighRate), (100_000, 21_000));
    assert_eq!(amount(Rubriek::LowRate), (200_000, 18_000));
    assert_eq!(amount(Rubriek::EuSupplies), (50_000, 0));
    assert_eq!(amount(Rubriek::Exports), (30_000, 0));
    assert_eq!(amount(Rubriek::ZeroOrExempt), (10_000, 0));

    // An acquisition carries its own VAT column: the tax we assessed on it belongs
    // in the box of the supply that triggered it.
    assert_eq!(amount(Rubriek::EuAcquisitions), (10_000, 2_100));
    assert_eq!(amount(Rubriek::ReverseChargeServices), (20_000, 4_200));

    // 5a: 21 000 + 18 000 on sales, plus 2 100 and 4 200 assessed on the purchases;
    // 5b: 8 400 the supplier charged, plus the same 6 300 deducted.
    assert_eq!(report.vat_due_micros() / 10_000, 45_300);
    assert_eq!(report.vat_deductible_micros() / 10_000, 14_700);
    assert_eq!(report.payable_micros() / 10_000, 30_600);
    assert!(report.unmapped.is_empty(), "{:?}", report.unmapped);
    assert_eq!(report.basis.len(), 8, "one entry per document, eight documents");
}

#[test]
fn preparing_stores_the_figures_and_their_basis() {
    let mut books = Books::open();
    books.quarter();

    let return_ = prepare(books.database.connection(), books.seller, 2026, 1, "ada", NOW).unwrap();
    assert!(return_.supersedes.is_none());
    assert_eq!(return_.computed.payable_micros() / 10_000, 30_600);
    assert_eq!(return_.basis_hash.len(), 64);

    let stored = prepared(books.database.connection(), books.seller, 2026, 1)
        .unwrap()
        .unwrap();
    assert_eq!(stored.id, return_.id);
    assert_eq!(stored.computed.payable_micros(), return_.computed.payable_micros());
    assert_eq!(stored.computed.line(Rubriek::HighRate).unwrap().vat_micros, 210_000_000);
    assert!(store::vat::basis_is_intact(books.database.connection(), &stored.id).unwrap());

    let events: i64 = books
        .database
        .connection()
        .query_row(
            "SELECT count(*) FROM audit_event WHERE kind = 'vat_return_prepared'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);
}

#[test]
fn preparing_again_supersedes_the_first_return_and_keeps_it() {
    let mut books = Books::open();
    books.quarter();
    let first = prepare(books.database.connection(), books.seller, 2026, 1, "ada", NOW).unwrap();

    // A late invoice changes the quarter.
    books.sell(TaxTreatment::standard(VatRate::STANDARD_NL), 100, "2026-01-30");
    let second = prepare(books.database.connection(), books.seller, 2026, 1, "ada", NOW).unwrap();

    assert_eq!(second.supersedes.as_deref(), Some(first.id.as_str()));
    assert_ne!(second.basis_hash, first.basis_hash);

    let connection = books.database.connection();
    let (prepared_count, superseded_count): (i64, i64) = connection
        .query_row(
            "SELECT sum(CASE status WHEN 'prepared' THEN 1 ELSE 0 END),
                    sum(CASE status WHEN 'superseded' THEN 1 ELSE 0 END)
             FROM vat_return",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((prepared_count, superseded_count), (1, 1));

    // The superseded basis survives for good, which is what makes it evidence.
    let kept: i64 = connection
        .query_row(
            "SELECT count(*) FROM vat_basis_posting WHERE return_id = ?1",
            [&first.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kept as usize, first.computed.basis.len());
}

#[test]
fn a_period_with_no_postings_reports_zeros() {
    let books = Books::open();
    let report = compute(books.database.connection(), books.seller, 2026, 5).unwrap();

    assert!(report.is_empty());
    assert_eq!(report.payable_micros(), 0);
    assert_eq!(report.currency, Currency::EUR);

    let return_ = prepare(books.database.connection(), books.seller, 2026, 5, "ada", NOW).unwrap();
    assert!(return_.computed.is_empty());
    assert!(
        store::vat::basis_is_intact(books.database.connection(), &return_.id).unwrap(),
        "an empty basis hashes to a stable value"
    );
}

#[test]
fn a_locked_period_can_still_be_reported_on() {
    let mut books = Books::open();
    books.quarter();
    let report = compute(books.database.connection(), books.seller, 2026, 1).unwrap();

    let period = store::ensure_period(books.database.connection(), books.seller, date("2026-01-15")).unwrap();
    store::lock_period(books.database.connection(), period.id, "ada", NOW).unwrap();

    let after = compute(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert_eq!(after.payable_micros(), report.payable_micros());
    assert!(prepare(books.database.connection(), books.seller, 2026, 1, "ada", NOW).is_ok());
}

#[test]
fn a_credit_note_reduces_the_rubriek_it_corrects() {
    let mut books = Books::open();
    books.sell(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-15");

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

    let report = compute(books.database.connection(), books.seller, 2026, 1).unwrap();
    assert_eq!(report.payable_micros(), 0, "the pair cancels in the return");
    assert!(
        report
            .line(Rubriek::HighRate)
            .is_none_or(|line| line.base_micros == 0 && line.vat_micros == 0),
        "a cancelled sale leaves nothing to report"
    );
}
