#![allow(clippy::unwrap_used)]
//! End-to-end storage test for the M1 slice: parties → draft → issue → verify.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocType, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money,
    Party, PrecedingInvoice, Quantity, SellerId, TaxTreatment, UnitCode, VatRate,
};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, Suppression, attach_artefact,
    credit_note_for, find_issued_reference, insert_customer, insert_draft, insert_seller, issue_draft,
    list_suppressions, load_customer, load_draft, load_seller, save_suppression, verify_artefacts,
};
use tempfile::TempDir;

const NOW: &str = "2026-10-27T12:00:00Z";

fn fixture() -> (TempDir, Database) {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
    let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();
    (dir, database)
}

fn party(name: &str, vat: Option<&str>) -> Party {
    let mut party = Party::new(name, Address::new("Amsterdam", CountryCode::NL));
    party.vat_id = vat.map(str::to_owned);
    party
}

fn draft_for(seller: SellerId, customer: CustomerId) -> InvoiceDraft {
    let mut draft = InvoiceDraft::new(
        seller,
        customer,
        Currency::EUR,
        LocalDate::parse_iso("2026-10-27").unwrap(),
    );
    draft.due_date = Some(LocalDate::parse_iso("2026-11-26").unwrap());
    draft.delivery_date = Some(LocalDate::parse_iso("2026-10-25").unwrap());
    draft.buyer_reference = Some("PO-1".to_owned());
    draft.lines.push(DraftLine {
        id: LineId::new(),
        position: 1,
        name: "Consulting".to_owned(),
        description: Some("Advice".to_owned()),
        quantity: Quantity::from_units(2).unwrap(),
        unit: UnitCode::PIECE,
        unit_price: Money::from_major_units(50, Currency::EUR).unwrap(),
        price_base_quantity: Quantity::ONE,
        treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
    });
    draft
}

fn seeded() -> (TempDir, Database, SellerId, CustomerId) {
    let (dir, database) = fixture();
    let seller_id = SellerId::new();
    let customer_id = CustomerId::new();

    insert_seller(
        database.connection(),
        &SellerRecord {
            id: seller_id,
            party: party("Seller BV", Some("NL123456782B01")),
            default_currency: Currency::EUR,
            default_locale: "nl-NL".to_owned(),
        },
        NOW,
    )
    .unwrap();
    insert_customer(
        database.connection(),
        &CustomerRecord {
            id: customer_id,
            party: party("Customer BV", None),
        },
        NOW,
    )
    .unwrap();

    (dir, database, seller_id, customer_id)
}

#[test]
fn parties_and_drafts_round_trip() {
    let (_dir, database, seller_id, customer_id) = seeded();

    let seller = load_seller(database.connection(), seller_id).unwrap();
    assert_eq!(seller.party.legal_name, "Seller BV");
    assert_eq!(seller.party.vat_id.as_deref(), Some("NL123456782B01"));
    assert_eq!(seller.default_currency, Currency::EUR);

    let customer = load_customer(database.connection(), customer_id).unwrap();
    assert_eq!(customer.party.legal_name, "Customer BV");

    let draft = draft_for(seller_id, customer_id);
    insert_draft(database.connection(), &draft, NOW).unwrap();

    let loaded = load_draft(database.connection(), draft.id).unwrap().unwrap();
    assert_eq!(loaded.lines.len(), 1);
    assert_eq!(loaded.lines[0].name, "Consulting");
    assert_eq!(loaded.lines[0].unit_price.micros(), 50_000_000);
    assert_eq!(loaded.lines[0].treatment.rate, VatRate::STANDARD_NL);
    assert_eq!(loaded.due_date, draft.due_date);
    assert_eq!(loaded.delivery_date, draft.delivery_date);
}

#[test]
fn suppressions_are_recorded_and_updated() {
    let (_dir, database, seller_id, customer_id) = seeded();
    let draft = draft_for(seller_id, customer_id);
    insert_draft(database.connection(), &draft, NOW).unwrap();

    save_suppression(
        database.connection(),
        draft.id,
        "T1-013",
        "lines[0].exemption_reason",
        "Known exempt supply",
        "test",
        NOW,
    )
    .unwrap();
    assert_eq!(
        list_suppressions(database.connection(), draft.id).unwrap(),
        vec![Suppression {
            code: "T1-013".to_owned(),
            field: "lines[0].exemption_reason".to_owned(),
            reason: "Known exempt supply".to_owned(),
        }]
    );

    save_suppression(
        database.connection(),
        draft.id,
        "T1-013",
        "lines[0].exemption_reason",
        "Updated reason",
        "test",
        NOW,
    )
    .unwrap();
    let suppressions = list_suppressions(database.connection(), draft.id).unwrap();
    assert_eq!(suppressions.len(), 1);
    assert_eq!(suppressions[0].reason, "Updated reason");

    assert!(
        save_suppression(
            database.connection(),
            draft.id,
            "T1-013",
            "lines[0].exemption_reason",
            "   ",
            "test",
            NOW,
        )
        .is_err()
    );
}

#[test]
fn issuing_allocates_numbers_and_stores_artefacts() {
    let (_dir, mut database, seller_id, customer_id) = seeded();

    let mut numbers = Vec::new();
    for _ in 0..2 {
        let draft = draft_for(seller_id, customer_id);
        insert_draft(database.connection(), &draft, NOW).unwrap();

        let issued = issue_draft(
            database.connection_mut(),
            IssueRequest {
                draft_id: draft.id,
                actor: "test",
                device: "device-1",
                renderer_version: "test-renderer-1",
                now: NOW,
            },
            |view| {
                assert_eq!(view.totals.net.micros(), 100_000_000);
                assert_eq!(view.totals.gross.micros(), 121_000_000);
                Ok(format!("<Invoice><ID>{}</ID></Invoice>", view.number).into_bytes())
            },
        )
        .unwrap();

        assert_eq!(issued.ubl_sha256.len(), 64);
        assert_eq!(issued.audit_head.len(), 64);
        numbers.push(issued.number);
    }

    assert_eq!(numbers, vec!["2026-0001", "2026-0002"]);
    assert_eq!(verify_artefacts(database.connection()).unwrap(), 2);
    assert_eq!(store::audit::verify_chain(database.connection()).unwrap(), 4);
}

#[test]
fn attached_artefacts_are_hashed_and_verified() {
    let (_dir, mut database, seller_id, customer_id) = seeded();
    let draft = draft_for(seller_id, customer_id);
    insert_draft(database.connection(), &draft, NOW).unwrap();

    let issued = issue_draft(
        database.connection_mut(),
        IssueRequest {
            draft_id: draft.id,
            actor: "test",
            device: "device-1",
            renderer_version: "test-renderer-1",
            now: NOW,
        },
        |_| Ok(b"<Invoice/>".to_vec()),
    )
    .unwrap();

    let hash = attach_artefact(
        database.connection(),
        issued.id,
        "pdf",
        b"%PDF-1.7\n%test",
        "test-renderer-1",
        NOW,
    )
    .unwrap();

    assert_eq!(hash.len(), 64);
    assert_eq!(verify_artefacts(database.connection()).unwrap(), 2);
}

#[test]
fn issued_documents_are_write_once() {
    let (_dir, mut database, seller_id, customer_id) = seeded();
    let draft = draft_for(seller_id, customer_id);
    insert_draft(database.connection(), &draft, NOW).unwrap();
    issue_draft(
        database.connection_mut(),
        IssueRequest {
            draft_id: draft.id,
            actor: "test",
            device: "device-1",
            renderer_version: "test-renderer-1",
            now: NOW,
        },
        |_| Ok(b"<Invoice/>".to_vec()),
    )
    .unwrap();

    let update = database
        .connection()
        .execute("UPDATE issued_invoice SET number = 'tampered'", []);
    assert!(update.is_err(), "the trigger must reject updates");

    let delete = database.connection().execute("DELETE FROM issued_invoice", []);
    assert!(delete.is_err(), "the trigger must reject deletes");
}

#[test]
fn tampering_with_the_audit_chain_is_detected() {
    let (_dir, mut database, seller_id, customer_id) = seeded();
    let draft = draft_for(seller_id, customer_id);
    insert_draft(database.connection(), &draft, NOW).unwrap();
    issue_draft(
        database.connection_mut(),
        IssueRequest {
            draft_id: draft.id,
            actor: "test",
            device: "device-1",
            renderer_version: "test-renderer-1",
            now: NOW,
        },
        |_| Ok(b"<Invoice/>".to_vec()),
    )
    .unwrap();

    database
        .connection()
        .execute(
            "UPDATE audit_event SET payload_json = '{\"tampered\":true}' WHERE seq = 1",
            [],
        )
        .unwrap();

    assert!(store::audit::verify_chain(database.connection()).is_err());
}

#[test]
fn missing_records_are_reported() {
    let (_dir, database, _seller, _customer) = seeded();
    let missing = SellerId::new();
    assert!(load_seller(database.connection(), missing).is_err());
    assert!(load_draft(database.connection(), DocumentId::new()).unwrap().is_none());
}

fn issue(database: &mut Database, draft: &InvoiceDraft) -> store::IssuedInvoice {
    insert_draft(database.connection(), draft, NOW).unwrap();
    issue_draft(
        database.connection_mut(),
        IssueRequest {
            draft_id: draft.id,
            actor: "test",
            device: "device-1",
            renderer_version: "test-renderer-1",
            now: NOW,
        },
        |view| Ok(format!("<Invoice><ID>{}</ID></Invoice>", view.number).into_bytes()),
    )
    .unwrap()
}

#[test]
fn a_credit_note_copies_the_issued_invoice() {
    let (_dir, mut database, seller_id, customer_id) = seeded();
    let issued = issue(&mut database, &draft_for(seller_id, customer_id));

    let credit = credit_note_for(
        database.connection(),
        seller_id,
        &issued.number,
        LocalDate::parse_iso("2026-10-28").unwrap(),
    )
    .unwrap()
    .expect("the issued invoice exists");

    assert_eq!(credit.doc_type, DocType::CreditNote);
    assert_eq!(credit.customer_id, customer_id);
    assert_eq!(credit.currency, Currency::EUR);
    assert_eq!(credit.buyer_reference.as_deref(), Some("PO-1"));
    assert_eq!(credit.lines.len(), 1);
    assert_eq!(credit.lines[0].unit_price.micros(), 50_000_000);
    assert_eq!(credit.lines[0].quantity, Quantity::from_units(2).unwrap());

    let preceding = credit.preceding_invoice.as_ref().expect("BG-3");
    assert_eq!(preceding.id, issued.id);
    assert_eq!(preceding.number, issued.number);
    assert_eq!(preceding.issue_date, issued.issue_date);

    insert_draft(database.connection(), &credit, NOW).unwrap();
    let loaded = load_draft(database.connection(), credit.id).unwrap().unwrap();
    assert_eq!(loaded.doc_type, DocType::CreditNote);
    assert_eq!(loaded.preceding_invoice.as_ref().unwrap().number, issued.number);

    let referenced = find_issued_reference(database.connection(), seller_id, &issued.number)
        .unwrap()
        .unwrap();
    assert_eq!(referenced.id, issued.id);
    assert_eq!(referenced.customer_id, customer_id);

    assert!(
        credit_note_for(database.connection(), seller_id, "2026-9999", LocalDate::today_utc())
            .unwrap()
            .is_none()
    );
}

#[test]
fn credit_notes_and_invoices_number_separately() {
    let (_dir, mut database, seller_id, customer_id) = seeded();
    let invoice = issue(&mut database, &draft_for(seller_id, customer_id));
    let mut credit = credit_note_for(
        database.connection(),
        seller_id,
        &invoice.number,
        LocalDate::parse_iso("2026-10-28").unwrap(),
    )
    .unwrap()
    .unwrap();
    credit.due_date = None;
    let issued_credit = issue(&mut database, &credit);

    assert_eq!(invoice.number, "2026-0001");
    assert_eq!(issued_credit.number, "2026-C0001");

    let second = issue(&mut database, &draft_for(seller_id, customer_id));
    assert_eq!(second.number, "2026-0002");
}

#[test]
fn issuing_rejects_an_unissued_preceding_invoice() {
    let (_dir, mut database, seller_id, customer_id) = seeded();
    let mut draft = InvoiceDraft::credit_note(
        seller_id,
        customer_id,
        Currency::EUR,
        LocalDate::parse_iso("2026-10-28").unwrap(),
        PrecedingInvoice::new(
            DocumentId::new(),
            "2026-0042",
            LocalDate::parse_iso("2026-10-01").unwrap(),
        ),
    );
    draft.lines = draft_for(seller_id, customer_id).lines;

    insert_draft(database.connection(), &draft, NOW).unwrap();
    let rejected = issue_draft(
        database.connection_mut(),
        IssueRequest {
            draft_id: draft.id,
            actor: "test",
            device: "device-1",
            renderer_version: "test-renderer-1",
            now: NOW,
        },
        |_| Ok(b"<CreditNote/>".to_vec()),
    );
    assert!(rejected.is_err(), "an unknown reference must not be issued");

    let invoice = issue(&mut database, &draft_for(seller_id, customer_id));
    assert_eq!(invoice.number, "2026-0001", "the rejected issue consumed no number");
}
