#![allow(clippy::unwrap_used)]
//! Whole-archive integrity: payloads, artefacts, imports, and orphans.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money, Party,
    Quantity, SellerId, TaxTreatment, UnitCode, VatRate,
};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, attach_artefact, audit::sha256_hex,
    insert_customer, insert_draft, insert_seller, integrity::audit, issue_draft,
};
use tempfile::TempDir;

const NOW: &str = "2026-10-28T12:00:00Z";

fn party(name: &str) -> Party {
    Party::new(name, Address::new("Amsterdam", CountryCode::NL))
}

/// One issued invoice, which is the parent every artefact and import needs.
fn seeded() -> (TempDir, Database, DocumentId) {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "pass", KdfParams::TEST).unwrap();
    let mut database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();

    let seller_id = SellerId::new();
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

    let customer_id = CustomerId::new();
    insert_customer(
        database.connection(),
        &CustomerRecord {
            id: customer_id,
            party: party("Customer BV"),
        },
        NOW,
    )
    .unwrap();

    let mut draft = InvoiceDraft::new(
        seller_id,
        customer_id,
        Currency::EUR,
        LocalDate::parse_iso("2026-10-27").unwrap(),
    );
    draft.lines.push(DraftLine {
        id: LineId::new(),
        position: 1,
        name: "Consulting".to_owned(),
        description: None,
        quantity: Quantity::ONE,
        unit: UnitCode::PIECE,
        unit_price: Money::from_major_units(100, Currency::EUR).unwrap(),
        price_base_quantity: Quantity::ONE,
        treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
    });
    insert_draft(database.connection(), &draft, NOW).unwrap();

    let issued = issue_draft(
        database.connection_mut(),
        IssueRequest {
            draft_id: draft.id,
            actor: "test",
            device: "test",
            renderer_version: "test",
            now: NOW,
        },
        |_| Ok(b"<Invoice/>".to_vec()),
    )
    .unwrap();

    (dir, database, issued.id)
}

fn insert_blob(database: &Database, sha256: &str, payload: &[u8]) -> String {
    let id = DocumentId::new().to_string();
    database
        .connection()
        .execute(
            "INSERT INTO blob (id, sha256, size_bytes, compression, payload, created_at)
             VALUES (?1, ?2, ?3, NULL, ?4, ?5)",
            rusqlite::params![id, sha256, payload.len() as i64, payload, NOW],
        )
        .unwrap();
    id
}

#[test]
fn a_clean_archive_verifies() {
    let (_dir, database, document_id) = seeded();
    attach_artefact(database.connection(), document_id, "ubl", b"<Invoice/>", "test", NOW).unwrap();

    let report = audit(database.connection()).unwrap();
    assert_eq!(
        report.blobs, 2,
        "the issue path stores one payload, and attaching an artefact adds one"
    );
    assert_eq!(report.artefacts, 2);
    assert_eq!(report.imports, 0);
    assert_eq!(report.orphaned_blobs, 0);
}

#[test]
fn a_payload_that_no_longer_hashes_to_its_record_is_caught() {
    let (_dir, database, _document_id) = seeded();
    insert_blob(&database, &"0".repeat(64), b"this is not what was recorded");

    let error = audit(database.connection()).unwrap_err();
    assert!(error.to_string().contains("does not hash"), "{error}");
}

#[test]
fn an_unreferenced_blob_is_reported_but_not_fatal() {
    let (_dir, database, _document_id) = seeded();
    insert_blob(&database, &sha256_hex(b"stray"), b"stray");

    let report = audit(database.connection()).unwrap();
    assert_eq!(report.blobs, 2, "the issued artefact plus the stray payload");
    assert_eq!(report.orphaned_blobs, 1, "intact but wasteful");
}

#[test]
fn an_artefact_whose_hash_disagrees_with_its_blob_is_caught() {
    let (_dir, database, document_id) = seeded();
    let payload = b"<Invoice/>";
    let blob_id = insert_blob(&database, &sha256_hex(payload), payload);
    database
        .connection()
        .execute(
            "INSERT INTO artefact
                 (id, document_id, kind, syntax_profile, bytes_hash, size_bytes, blob_id,
                  rendered_at, renderer_version)
             VALUES (?1, ?2, 'ubl', NULL, ?3, ?4, ?5, ?6, 'test')",
            rusqlite::params![
                DocumentId::new().to_string(),
                document_id.to_string(),
                "1".repeat(64),
                payload.len() as i64,
                blob_id,
                NOW
            ],
        )
        .unwrap();

    let error = audit(database.connection()).unwrap_err();
    assert!(error.to_string().contains("does not match the payload"), "{error}");
}

#[test]
fn an_import_without_its_payload_is_caught() {
    let (_dir, database, document_id) = seeded();
    database
        .connection()
        .execute(
            "INSERT INTO import_record
                 (id, document_id, source_kind, source_path, payload_hash, match_status,
                  match_confidence, imported_at)
             VALUES (?1, ?2, 'ubl', NULL, ?3, 'unmatched', NULL, ?4)",
            rusqlite::params![
                DocumentId::new().to_string(),
                document_id.to_string(),
                "f".repeat(64),
                NOW
            ],
        )
        .unwrap();

    let error = audit(database.connection()).unwrap_err();
    assert!(error.to_string().contains("has no payload"), "{error}");
}

#[test]
fn blobs_cannot_be_rewritten_or_deleted() {
    let (_dir, database, _document_id) = seeded();
    insert_blob(&database, &sha256_hex(b"evidence"), b"evidence");

    let update = database.connection().execute("UPDATE blob SET payload = X'00'", []);
    assert!(update.is_err(), "a payload must not be rewritten");

    let delete = database.connection().execute("DELETE FROM blob", []);
    assert!(delete.is_err(), "a payload must not be deleted");
}
