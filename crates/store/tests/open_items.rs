#![allow(clippy::unwrap_used)]
//! Open items: written once when a document is posted, aged by due date, and
//! never edited afterwards.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocType, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money,
    Party, Quantity, SellerId, TaxTreatment, UnitCode, VatRate,
};
use store::open_items::{OpenItemKind, aging, list, total};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, insert_customer, insert_draft,
    insert_seller, issue_draft, post_document, post_outbound_document, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T18:00:00.123456Z";

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

    fn issue(&mut self, net_units: i64, on: &str, due: Option<&str>) -> DocumentId {
        let mut draft = InvoiceDraft::new(self.seller, self.customer, Currency::EUR, date(on));
        draft.due_date = due.map(date);
        draft.lines.push(DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Consulting".to_owned(),
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: euros(net_units),
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

    fn receive(&mut self, net_units: i64, on: &str, number: &str) -> DocumentId {
        let net = euros(net_units);
        let tax = crux::tax_amount(net, VatRate::STANDARD_NL).unwrap();
        let totals = crux::InvoiceTotals {
            net,
            tax,
            gross: net.checked_add(tax).unwrap(),
            breakdown: vec![crux::TaxBreakdown {
                category: crux::VatCategory::Standard,
                rate_bp: 2100,
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
            due_date: Some(date("2026-02-15")),
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
fn posting_a_document_writes_one_open_item() {
    let mut books = Books::open();
    let invoice = books.issue(1_000, "2026-01-10", Some("2026-02-09"));
    let purchase = books.receive(400, "2026-01-12", "SUP-1");

    let posted = post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();
    post_document(books.database.connection(), purchase, "ada", NOW).unwrap();

    let receivable = list(books.database.connection(), books.seller, OpenItemKind::Receivable).unwrap();
    assert_eq!(receivable.len(), 1);
    assert_eq!(receivable[0].amount, euros(1_210), "the gross, not the net");
    assert_eq!(receivable[0].due_date, Some(date("2026-02-09")));
    assert_eq!(receivable[0].party_id, books.customer);
    assert_eq!(receivable[0].source, invoice);
    assert!(receivable[0].is_open());

    let payable = list(books.database.connection(), books.seller, OpenItemKind::Payable).unwrap();
    assert_eq!(payable.len(), 1);
    assert_eq!(payable[0].amount, euros(484));

    // Posting again adds nothing: the entry is the same entry.
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();
    assert_eq!(
        list(books.database.connection(), books.seller, OpenItemKind::Receivable)
            .unwrap()
            .len(),
        1
    );
    let _ = posted;
}

#[test]
fn a_credit_note_is_a_negative_open_item() {
    let mut books = Books::open();
    let invoice = books.issue(1_000, "2026-01-10", Some("2026-02-09"));
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

    let items = list(books.database.connection(), books.seller, OpenItemKind::Receivable).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1].amount, euros(-1_210), "the correction reduces what is owed");
    assert_eq!(
        total(books.database.connection(), books.seller, OpenItemKind::Receivable).unwrap(),
        euros(0),
        "and the subledger nets to nothing"
    );
}

#[test]
fn aging_puts_each_item_in_the_bucket_its_due_date_earns() {
    let mut books = Books::open();
    for (index, due) in ["2026-04-30", "2026-03-15", "2026-02-15", "2025-12-20"]
        .iter()
        .enumerate()
    {
        let invoice = books.issue(100, "2026-01-05", Some(due));
        post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();
        let _ = index;
    }

    // Cut at 2026-04-01: one not yet due, one 17 days over, one 45 days over,
    // one 102 days over.
    let report = aging(
        books.database.connection(),
        books.seller,
        OpenItemKind::Receivable,
        date("2026-04-01"),
    )
    .unwrap();

    assert_eq!(report.current, euros(121));
    assert_eq!(report.days_1_30, euros(121));
    assert_eq!(report.days_31_60, euros(121));
    assert_eq!(report.days_61_90, euros(0));
    assert_eq!(report.over_90, euros(121));
    assert_eq!(report.total().unwrap(), euros(484));
}

#[test]
fn an_open_item_cannot_be_edited_or_deleted() {
    let mut books = Books::open();
    let invoice = books.issue(100, "2026-01-05", Some("2026-02-04"));
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();

    let update = books
        .database
        .connection()
        .execute("UPDATE open_item SET amount_micros = 1", [])
        .unwrap_err();
    assert!(update.to_string().contains("created once"), "{update}");
    let delete = books
        .database
        .connection()
        .execute("DELETE FROM open_item", [])
        .unwrap_err();
    assert!(delete.to_string().contains("created once"), "{delete}");
}

#[test]
fn a_rebuild_writes_the_items_an_older_archive_never_wrote() {
    let mut books = Books::open();
    let invoice = books.issue(1_000, "2026-01-10", Some("2026-02-09"));
    post_outbound_document(books.database.connection(), invoice, "ada", NOW).unwrap();
    let purchase = books.receive(400, "2026-01-12", "SUP-1");
    post_document(books.database.connection(), purchase, "ada", NOW).unwrap();

    // A credit note as well, so the sign the posting used is rebuilt too.
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

    // An archive from before 0013 has the entries and no rows. The delete trigger
    // is exactly what stops that state from being made now, so the fixture removes
    // it before emptying the table.
    let connection = books.database.connection();
    connection.execute("DROP TRIGGER open_item_no_delete", []).unwrap();
    connection.execute("DELETE FROM open_item", []).unwrap();

    let report = store::rebuild_open_items(connection, books.seller, "ada", NOW).unwrap();
    assert_eq!(report.added, 3, "two invoices and a purchase");
    assert_eq!(report.present, 0);
    assert!(report.is_complete());

    let receivable = list(connection, books.seller, OpenItemKind::Receivable).unwrap();
    assert_eq!(receivable.len(), 2);
    assert_eq!(receivable[0].amount, euros(1_210), "the gross, as posting wrote it");
    assert_eq!(receivable[0].due_date, Some(date("2026-02-09")));
    assert_eq!(receivable[0].original_date, date("2026-01-10"));
    assert_eq!(receivable[0].source, invoice);
    assert_eq!(receivable[1].amount, euros(-1_210), "the credit note keeps its sign");
    assert_eq!(
        total(connection, books.seller, OpenItemKind::Receivable).unwrap(),
        euros(0)
    );
    let payable = list(connection, books.seller, OpenItemKind::Payable).unwrap();
    assert_eq!(payable.len(), 1);
    assert_eq!(payable[0].amount, euros(484));

    // The subledger changed after the postings that explain it, so the chain says
    // who rebuilt it and how much it wrote.
    let events: i64 = connection
        .query_row(
            "SELECT count(*) FROM audit_event WHERE kind = 'open_items_rebuilt'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(events, 1);

    // Running it again adds nothing: the items are there, so they are left alone.
    let again = store::rebuild_open_items(connection, books.seller, "ada", NOW).unwrap();
    assert_eq!(again.added, 0);
    assert_eq!(again.present, 3);
    assert_eq!(again.unmatched, Vec::<String>::new());
}

#[test]
fn a_document_without_a_counterparty_has_no_open_item() {
    let books = Books::open();
    let net = euros(100);
    let tax = crux::tax_amount(net, VatRate::STANDARD_NL).unwrap();
    let totals = crux::InvoiceTotals {
        net,
        tax,
        gross: net.checked_add(tax).unwrap(),
        breakdown: vec![crux::TaxBreakdown {
            category: crux::VatCategory::Standard,
            rate_bp: 2100,
            taxable_base: net,
            tax,
        }],
    };
    let issuer = Party::new("Onbekend BV", Address::new("Rotterdam", CountryCode::NL));
    let seller = store::load_seller(books.database.connection(), books.seller).unwrap();
    let received = store::ReceivedDocument {
        number: "UNMATCHED-1",
        doc_type: DocType::Invoice,
        currency: Currency::EUR,
        issue_date: date("2026-01-12"),
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
    store::record_received(
        books.database.connection(),
        books.seller,
        None,
        &received,
        &store::ImportSource {
            kind: "ubl",
            path: None,
            payload_hash: &store::audit::sha256_hex(b"unmatched"),
            payload: b"unmatched",
        },
        None,
        NOW,
    )
    .unwrap();
    let id: String = books
        .database
        .connection()
        .query_row("SELECT id FROM document WHERE number = 'UNMATCHED-1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    post_document(
        books.database.connection(),
        DocumentId::parse_str(&id).unwrap(),
        "ada",
        NOW,
    )
    .unwrap();

    assert!(
        list(books.database.connection(), books.seller, OpenItemKind::Payable)
            .unwrap()
            .is_empty(),
        "the books cannot say who is owed, so there is nothing to reconcile"
    );

    // A rebuild cannot invent the party either, so it names the document instead
    // of quietly leaving the control account unexplained.
    let report = store::rebuild_open_items(books.database.connection(), books.seller, "ada", NOW).unwrap();
    assert_eq!(report.added, 0);
    assert_eq!(report.unmatched, vec!["UNMATCHED-1".to_owned()]);
    assert!(!report.is_complete());
}
