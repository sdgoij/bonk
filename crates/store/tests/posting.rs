#![allow(clippy::unwrap_used)]
//! Posting rules against the scenario matrix: each row is an acceptance test of
//! the entry the ledger produces, not a description of what it might produce.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocType, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money,
    Party, Quantity, SellerId, TaxTreatment, UnitCode, VatCategory, VatRate,
};
use store::ledger::PostedEntry;
use store::posting::DEBTORS;
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, PostingSource, SellerRecord, StoreError,
    credit_note_for, insert_customer, insert_draft, insert_seller, issue_draft, post_outbound_document,
    preview_outbound_entry, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T14:00:00.123456Z";

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

    /// Issues a one-line invoice and returns its id.
    fn issue(&mut self, treatment: TaxTreatment, net_units: i64, date: &str) -> DocumentId {
        let mut draft = InvoiceDraft::new(
            self.seller,
            self.customer,
            Currency::EUR,
            LocalDate::parse_iso(date).unwrap(),
        );
        draft.lines.push(DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Consulting".to_owned(),
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(net_units, Currency::EUR).unwrap(),
            price_base_quantity: Quantity::ONE,
            treatment,
        });
        self.issue_draft(&draft)
    }

    fn issue_draft(&mut self, draft: &InvoiceDraft) -> DocumentId {
        insert_draft(self.database.connection(), draft, NOW).unwrap();
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

    fn post(&self, document: DocumentId) -> PostedEntry {
        post_outbound_document(self.database.connection(), document, "ada", NOW).unwrap()
    }

    /// The posted lines as `account side cents`, for readable assertions.
    fn lines_of(&self, entry: &PostedEntry) -> Vec<String> {
        let mut statement = self
            .database
            .connection()
            .prepare(
                "SELECT account_code, side, amount_micros FROM journal_line
                 WHERE entry_id = ?1 ORDER BY line_no",
            )
            .unwrap();
        let rows: Vec<(String, String, i64)> = statement
            .query_map([entry.id.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows.into_iter()
            .map(|(account, side, micros)| format!("{account} {side} {}", micros / 10_000))
            .collect()
    }

    /// Every account's net movement across the given entries, in cents.
    fn nets(&self, entries: &[&PostedEntry]) -> Vec<(String, i64)> {
        let ids: Vec<String> = entries.iter().map(|entry| entry.id.to_string()).collect();
        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!(
            "SELECT account_code, sum(CASE side WHEN 'debit' THEN amount_micros ELSE -amount_micros END)
             FROM journal_line WHERE entry_id IN ({placeholders})
             GROUP BY account_code ORDER BY account_code"
        );
        let mut statement = self.database.connection().prepare(&sql).unwrap();
        statement
            .query_map(rusqlite::params_from_iter(ids), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? / 10_000))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

#[test]
fn a_domestic_sale_posts_the_matrix_row() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-15");
    let posted = books.post(invoice);

    assert_eq!(
        books.lines_of(&posted),
        vec![
            format!("{DEBTORS} debit 121000"),
            "8000 credit 100000".to_owned(),
            "1600 credit 21000".to_owned(),
        ]
    );
    assert_eq!(posted.number, "2026-0001");

    let (journal, description, profile, version): (String, String, String, String) = books
        .database
        .connection()
        .query_row(
            "SELECT journal_code, description, posting_profile_id, rule_version
             FROM journal_entry WHERE id = ?1",
            [posted.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(journal, "VK");
    assert_eq!(description, "Factuur 2026-0001");
    assert_eq!(profile, store::PROFILE_ID);
    assert_eq!(version, store::RULE_VERSION);
}

#[test]
fn a_low_rate_sale_uses_its_own_revenue_and_vat_accounts() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::standard(VatRate::REDUCED_NL), 1_000, "2026-01-15");
    let posted = books.post(invoice);

    assert_eq!(
        books.lines_of(&posted),
        vec![
            "1300 debit 109000".to_owned(),
            "8010 credit 100000".to_owned(),
            "1610 credit 9000".to_owned(),
        ]
    );
}

#[test]
fn shifting_vat_to_an_eu_customer_leaves_one_revenue_line() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::zero(VatCategory::ReverseCharge), 1_000, "2026-01-15");
    let posted = books.post(invoice);

    assert_eq!(
        books.lines_of(&posted),
        vec!["1300 debit 100000".to_owned(), "8020 credit 100000".to_owned()],
        "a reverse-charged service carries no VAT line of ours"
    );
}

#[test]
fn an_export_outside_the_eu_has_no_vat_and_its_own_revenue_account() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::zero(VatCategory::Export), 1_000, "2026-01-15");
    let posted = books.post(invoice);

    assert_eq!(
        books.lines_of(&posted),
        vec!["1300 debit 100000".to_owned(), "8040 credit 100000".to_owned()]
    );
}

#[test]
fn a_credit_note_mirrors_the_invoice() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-15");
    let invoice_entry = books.post(invoice);

    let draft = credit_note_for(
        books.database.connection(),
        books.seller,
        "2026-0001",
        LocalDate::parse_iso("2026-02-10").unwrap(),
    )
    .unwrap()
    .unwrap();
    let note = books.issue_draft(&draft);
    let note_entry = books.post(note);

    assert_eq!(
        books.lines_of(&note_entry),
        vec![
            "1300 credit 121000".to_owned(),
            "8000 debit 100000".to_owned(),
            "1600 debit 21000".to_owned(),
        ],
        "the correction is the invoice with every side reversed"
    );
    assert_eq!(
        note_entry.number, "2026-0002",
        "entry numbers run per journal, so the credit note follows the invoice rather than repeating its number"
    );
    assert_eq!(
        books.nets(&[&invoice_entry, &note_entry]),
        vec![("1300".to_owned(), 0), ("1600".to_owned(), 0), ("8000".to_owned(), 0),],
        "posted together, the pair leaves every account where it started"
    );
}

#[test]
fn posting_a_document_twice_does_not_double_the_books() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-15");

    let first = books.post(invoice);
    let second = books.post(invoice);

    assert!(first.created);
    assert!(!second.created);
    assert_eq!(first.id, second.id);

    let entries: i64 = books
        .database
        .connection()
        .query_row("SELECT count(*) FROM journal_entry", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entries, 1);
}

#[test]
fn a_preview_matches_what_posting_writes() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::standard(VatRate::STANDARD_NL), 1_000, "2026-01-15");

    let preview = preview_outbound_entry(books.database.connection(), invoice).unwrap();
    let posted = books.post(invoice);

    let recomputed = store::content_hash(
        &preview,
        &posted.number,
        LocalDate::parse_iso("2026-01-15").unwrap(),
        PostingSource::OutboundDocument(invoice),
    )
    .unwrap();
    assert_eq!(
        recomputed, posted.content_hash,
        "the preview is the same arithmetic the posting used"
    );
    assert_eq!(preview.lines().len(), 3);
}

#[test]
fn a_document_with_nothing_to_post_is_refused() {
    let mut books = Books::open();
    let invoice = books.issue(TaxTreatment::standard(VatRate::STANDARD_NL), 0, "2026-01-15");

    // A zero-value invoice is legal and must still be explainable; the ledger
    // cannot carry a zero line, so posting says so instead of writing nothing.
    let error = post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap_err();
    assert!(matches!(error, StoreError::Rejected(_)), "unexpected error: {error}");
    assert!(
        error.to_string().contains("nothing") || error.to_string().contains("no amount"),
        "{error}"
    );
}

#[test]
fn a_received_document_cannot_be_posted_as_a_sale() {
    let books = Books::open();
    let net = Money::from_major_units(100, Currency::EUR).unwrap();
    let tax = Money::from_major_units(21, Currency::EUR).unwrap();
    let totals = crux::InvoiceTotals {
        net,
        tax,
        gross: Money::from_major_units(121, Currency::EUR).unwrap(),
        breakdown: vec![crux::TaxBreakdown {
            category: VatCategory::Standard,
            rate_bp: 2100,
            taxable_base: net,
            tax,
        }],
    };
    let issuer = Party::new("Leverancier BV", Address::new("Rotterdam", CountryCode::NL));
    let seller = store::load_seller(books.database.connection(), books.seller).unwrap();
    let received = store::ReceivedDocument {
        number: "SUP-1",
        doc_type: DocType::Invoice,
        currency: Currency::EUR,
        issue_date: LocalDate::parse_iso("2026-01-10").unwrap(),
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
    let payload = b"<Invoice/>";
    store::record_received(
        books.database.connection(),
        books.seller,
        None,
        &received,
        &store::ImportSource {
            kind: "ubl",
            path: None,
            payload_hash: &store::audit::sha256_hex(payload),
            payload,
        },
        None,
        NOW,
    )
    .unwrap();
    let inbound: String = books
        .database
        .connection()
        .query_row("SELECT id FROM document WHERE direction = 'inbound'", [], |row| {
            row.get(0)
        })
        .unwrap();

    let error = post_outbound_document(
        books.database.connection(),
        DocumentId::parse_str(&inbound).unwrap(),
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("cannot be posted as a sale"),
        "unexpected error: {error}"
    );
}
