#![allow(clippy::unwrap_used)]
//! Evidence bundles: exported, verified offline, and hard to forge.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money, Party,
    Quantity, SellerId, TaxTreatment, UnitCode, VatRate,
};
use serde_json::{Value, json};
use store::evidence::{self, FORMAT};
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, audit, insert_customer, insert_draft,
    insert_seller, issue_draft,
};
use tempfile::TempDir;

const NOW: &str = "2026-10-28T12:00:00Z";

fn party(name: &str) -> Party {
    Party::new(name, Address::new("Amsterdam", CountryCode::NL))
}

fn archived() -> (TempDir, Database, SellerId, DocumentId) {
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
    draft.buyer_reference = Some("PO-1".to_owned());
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
        |_| Ok(b"<Invoice><ID>2026-0001</ID></Invoice>".to_vec()),
    )
    .unwrap();

    (dir, database, seller_id, issued.id)
}

fn export(
    database: &Database,
    seller: SellerId,
    document: DocumentId,
    directory: &std::path::Path,
) -> store::EvidenceReport {
    evidence::export(database.connection(), seller, document, directory, "test", NOW).unwrap()
}

#[test]
fn a_bundle_verifies_offline() {
    let (dir, database, seller, document) = archived();
    let bundle = dir.path().join("bundle");

    let report = export(&database, seller, document, &bundle);
    assert_eq!(report.document_id, document);
    assert_eq!(report.number, "2026-0001");
    assert_eq!(report.direction, "outbound");
    assert!(report.events >= 1, "the issue event travels with the bundle");
    assert!(report.files >= 3, "document, chain, and the UBL artefact");

    // Everything the bundle claims is on disk, named as the manifest says.
    for path in [
        "manifest.json",
        "document.json",
        "audit-chain.json",
        "artefacts/2026-0001-ubl.xml",
    ] {
        assert!(bundle.join(path).exists(), "{path} is missing");
    }

    let manifest: Value = serde_json::from_slice(&std::fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["format"], FORMAT);
    assert_eq!(manifest["document"]["number"], "2026-0001");

    // Verification alone is enough: no database is involved.
    let verified = evidence::verify(&bundle).unwrap();
    assert_eq!(verified, report);
}

#[test]
fn a_changed_file_is_caught() {
    let (dir, database, seller, document) = archived();
    let bundle = dir.path().join("bundle");
    export(&database, seller, document, &bundle);

    let artefact = bundle.join("artefacts/2026-0001-ubl.xml");
    let mut bytes = std::fs::read(&artefact).unwrap();
    bytes[1] ^= 0xff;
    std::fs::write(&artefact, &bytes).unwrap();

    let error = evidence::verify(&bundle).unwrap_err();
    assert!(error.to_string().contains("does not match the manifest"), "{error}");
}

#[test]
fn a_rewritten_chain_is_caught_even_with_a_matching_manifest() {
    let (dir, database, seller, document) = archived();
    let bundle = dir.path().join("bundle");
    export(&database, seller, document, &bundle);

    // Forge the way a forger would: change an event and fix up every hash the
    // manifest records, so only recomputing the chain itself can catch it.
    let chain_path = bundle.join("audit-chain.json");
    let mut chain: Vec<Value> = serde_json::from_slice(&std::fs::read(&chain_path).unwrap()).unwrap();
    chain[0]["payload"]["tampered"] = json!(true);
    let bytes = serde_json::to_vec_pretty(&chain).unwrap();
    std::fs::write(&chain_path, &bytes).unwrap();
    let hash = audit::sha256_hex(&bytes);

    let manifest_path = bundle.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["audit"]["sha256"] = json!(hash);
    for file in manifest["files"].as_array_mut().unwrap() {
        if file["path"] == "audit-chain.json" {
            file["sha256"] = json!(hash);
            file["size"] = json!(bytes.len());
        }
    }
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

    let error = evidence::verify(&bundle).unwrap_err();
    assert!(error.to_string().contains("has been altered"), "{error}");
}

#[test]
fn a_stray_file_is_caught() {
    let (dir, database, seller, document) = archived();
    let bundle = dir.path().join("bundle");
    export(&database, seller, document, &bundle);

    std::fs::write(bundle.join("notes.txt"), b"added later").unwrap();

    let error = evidence::verify(&bundle).unwrap_err();
    assert!(error.to_string().contains("does not list"), "{error}");
}

#[test]
fn a_manifest_that_points_outside_is_refused() {
    let (dir, database, seller, document) = archived();
    let bundle = dir.path().join("bundle");
    export(&database, seller, document, &bundle);

    let manifest_path = bundle.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["files"][0]["path"] = json!("../outside.json");
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

    let error = evidence::verify(&bundle).unwrap_err();
    assert!(error.to_string().contains("not a path inside"), "{error}");
}

#[test]
fn an_existing_bundle_is_not_overwritten() {
    let (dir, database, seller, document) = archived();
    let bundle = dir.path().join("bundle");
    export(&database, seller, document, &bundle);

    let error = evidence::export(database.connection(), seller, document, &bundle, "test", NOW).unwrap_err();
    assert!(matches!(error, store::StoreError::Rejected(_)), "{error}");
}

#[test]
fn an_unknown_document_is_refused() {
    let (dir, database, seller, _document) = archived();
    let error = evidence::export(
        database.connection(),
        seller,
        DocumentId::new(),
        dir.path().join("bundle"),
        "test",
        NOW,
    )
    .unwrap_err();
    assert!(matches!(error, store::StoreError::NotFound(_)), "{error}");
}
