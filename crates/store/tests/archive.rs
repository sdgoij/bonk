#![allow(clippy::unwrap_used)]
//! Reading an issued document back for display.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocType, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money,
    Party, Quantity, SellerId, TaxTreatment, UnitCode, VatRate,
};
use store::{
    CustomerRecord, Database, DocumentStatus, IssueRequest, KdfParams, KeyStore, SellerRecord, archive,
    insert_customer, insert_draft, insert_seller, issue_draft,
};
use tempfile::TempDir;

const NOW: &str = "2026-10-27T12:00:00Z";

fn fixture() -> (TempDir, Database) {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "passphrase", KdfParams::TEST).unwrap();
    let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();
    (dir, database)
}

fn party(name: &str) -> Party {
    let mut party = Party::new(name, Address::new("Amsterdam", CountryCode::NL));
    party.vat_id = Some("NL123456782B01".to_owned());
    party
}

fn line(position: u32, name: &str, units: i64, price: i64) -> DraftLine {
    DraftLine {
        id: LineId::new(),
        position,
        name: name.to_owned(),
        description: None,
        quantity: Quantity::from_units(units).unwrap(),
        unit: UnitCode::PIECE,
        unit_price: Money::from_major_units(price, Currency::EUR).unwrap(),
        price_base_quantity: Quantity::ONE,
        treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
    }
}

fn seeded() -> (TempDir, Database, SellerId, CustomerId) {
    let (dir, database) = fixture();
    let seller_id = SellerId::new();
    let customer_id = CustomerId::new();

    insert_seller(
        database.connection(),
        &SellerRecord {
            id: seller_id,
            party: party("Seller BV"),
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
            party: party("Customer BV"),
        },
        NOW,
    )
    .unwrap();

    (dir, database, seller_id, customer_id)
}

fn issue_one(database: &mut Database, seller_id: SellerId, customer_id: CustomerId) -> DocumentId {
    let mut draft = InvoiceDraft::new(
        seller_id,
        customer_id,
        Currency::EUR,
        LocalDate::parse_iso("2026-10-27").unwrap(),
    );
    draft.due_date = Some(LocalDate::parse_iso("2026-11-26").unwrap());
    draft.buyer_reference = Some("PO-1".to_owned());
    draft.note = Some("Thanks".to_owned());
    draft.lines.push(line(1, "Consulting", 2, 50));
    draft.lines.push(line(2, "Travel", 1, 20));
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
        |view| Ok(format!("<Invoice><ID>{}</ID></Invoice>", view.number).into_bytes()),
    )
    .unwrap()
    .id
}

#[test]
fn an_issued_document_reads_back_as_it_was_written() {
    let (_dir, mut database, seller_id, customer_id) = seeded();
    let id = issue_one(&mut database, seller_id, customer_id);

    let view = archive::load(database.connection(), id).unwrap().unwrap();
    assert_eq!(view.number, "2026-0001");
    assert_eq!(view.doc_type, DocType::Invoice);
    assert_eq!(view.status, DocumentStatus::Issued);
    assert_eq!(view.issue_date, LocalDate::parse_iso("2026-10-27").unwrap());
    assert_eq!(view.buyer_reference.as_deref(), Some("PO-1"));
    assert_eq!(view.note.as_deref(), Some("Thanks"));
    assert_eq!(view.seller.legal_name, "Seller BV");
    assert_eq!(view.buyer.legal_name, "Customer BV");

    assert_eq!(view.lines.len(), 2);
    assert_eq!(view.lines[0].name, "Consulting");
    assert_eq!(view.lines[0].quantity.to_trimmed_string(), "2");
    assert_eq!(view.lines[0].unit_price.micros(), 50_000_000);
    assert_eq!(view.lines[0].line_net.micros(), 100_000_000);
    assert_eq!(view.lines[1].name, "Travel");

    // 120.00 net, 21 %, so 25.20 tax and 145.20 payable.
    assert_eq!(view.totals.net.micros(), 120_000_000);
    assert_eq!(view.totals.tax.micros(), 25_200_000);
    assert_eq!(view.totals.gross.micros(), 145_200_000);
    assert_eq!(view.totals.breakdown.len(), 1);

    assert_eq!(view.artefacts.len(), 1);
    assert_eq!(view.artefacts[0].kind, "ubl");
    assert_eq!(view.semantic_hash.len(), 64);
    assert!(view.events.is_empty(), "nothing has happened to it yet");
}

#[test]
fn a_rename_after_issue_does_not_change_the_record() {
    let (_dir, mut database, seller_id, customer_id) = seeded();
    let id = issue_one(&mut database, seller_id, customer_id);

    // A rename writes the customer row; the document keeps its snapshot.
    database
        .connection()
        .execute(
            "UPDATE customer SET legal_name = 'Renamed BV', display_name = 'Renamed BV' WHERE id = ?1",
            [customer_id.to_string()],
        )
        .unwrap();

    let view = archive::load(database.connection(), id).unwrap().unwrap();
    assert_eq!(
        view.buyer.legal_name, "Customer BV",
        "the view reads the snapshot taken at issue, not today's customer row"
    );
}

#[test]
fn a_status_change_appears_after_the_initial_state() {
    let (_dir, mut database, seller_id, customer_id) = seeded();
    let id = issue_one(&mut database, seller_id, customer_id);

    // Issued documents are not in the payable register, so the event is written
    // directly for this test rather than through `advance_status`.
    database
        .connection()
        .execute(
            "INSERT INTO document_event (id, document_id, kind, at, actor, detail)
             VALUES (?1, ?2, 'archived', ?3, 'test', NULL)",
            rusqlite::params![DocumentId::new().to_string(), id.to_string(), NOW],
        )
        .unwrap();

    let view = archive::load(database.connection(), id).unwrap().unwrap();
    assert_eq!(view.status, DocumentStatus::Archived);
    assert_eq!(view.events.len(), 1);
    assert_eq!(view.events[0].kind, "archived");
    assert_eq!(view.events[0].actor, "test");
}

#[test]
fn a_document_that_is_not_there_is_not_an_error() {
    let (_dir, database, ..) = seeded();
    assert!(
        archive::load(database.connection(), DocumentId::new())
            .unwrap()
            .is_none()
    );
}
