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
    insert_seller, issue_draft, post_outbound_document, seed_starter_chart,
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

fn sell(database: &mut Database, seller: SellerId, on: &str) -> DocumentId {
    let customer_id = CustomerId::new();
    insert_customer(
        database.connection(),
        &CustomerRecord {
            id: customer_id,
            party: party("Second Customer BV"),
        },
        NOW,
    )
    .unwrap();

    let mut draft = InvoiceDraft::new(seller, customer_id, Currency::EUR, LocalDate::parse_iso(on).unwrap());
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
    issued.id
}

/// The archive from [`archived`], posted to the ledger, with October's return prepared.
fn posted() -> (TempDir, Database, SellerId, DocumentId) {
    let (dir, database, seller, document) = archived();
    seed_starter_chart(database.connection(), NOW).unwrap();
    post_outbound_document(database.connection(), document, "test", NOW).unwrap();
    store::vat::prepare(database.connection(), seller, 2026, 10, "test", NOW).unwrap();
    (dir, database, seller, document)
}

fn ledger_content(bundle: &std::path::Path, id: &str) -> String {
    std::fs::read_to_string(bundle.join("ledger.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|record| record["id"] == json!(id))
        .unwrap_or_else(|| panic!("the ledger does not carry {id}"))["content_hash"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Rewrites a file in the bundle and makes the manifest agree, the way a forger
/// with write access to the directory would.
fn replace_and_rehash(bundle: &std::path::Path, relative: &str, bytes: &[u8]) -> String {
    std::fs::write(bundle.join(relative), bytes).unwrap();
    let hash = audit::sha256_hex(bytes);
    patch_manifest(bundle, |manifest| {
        for file in manifest["files"].as_array_mut().unwrap() {
            if file["path"] == json!(relative) {
                file["sha256"] = json!(hash);
                file["size"] = json!(bytes.len());
            }
        }
    });
    hash
}

fn patch_manifest(bundle: &std::path::Path, patch: impl FnOnce(&mut Value)) {
    let path = bundle.join("manifest.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    patch(&mut manifest);
    std::fs::write(&path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
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
    assert_eq!(report.ledger_entries, 0, "an unposted document has no books to carry");
    assert_eq!(report.vat_returns, 0);

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

#[test]
fn a_correction_of_the_posting_travels_with_the_document() {
    let (dir, database, seller, document) = archived();
    seed_starter_chart(database.connection(), NOW).unwrap();
    let posted = post_outbound_document(database.connection(), document, "test", NOW).unwrap();
    store::reverse_entry(
        database.connection(),
        posted.id,
        "wrong customer",
        LocalDate::parse_iso("2026-11-02").unwrap(),
        "test",
        NOW,
    )
    .unwrap();

    let bundle = dir.path().join("bundle");
    let report = export(&database, seller, document, &bundle);
    assert_eq!(
        report.ledger_entries, 2,
        "the reversal is part of what happened to this document"
    );
    assert_eq!(evidence::verify(&bundle).unwrap(), report);
}

#[test]
fn a_bundle_carries_the_posting_and_the_return_it_explains() {
    let (dir, database, seller, document) = posted();
    let bundle = dir.path().join("bundle");

    let report = export(&database, seller, document, &bundle);
    assert_eq!(report.ledger_entries, 1, "the sale is in the bundle");
    assert_eq!(report.vat_returns, 1, "October's return travels with it");
    assert!(bundle.join("ledger.jsonl").exists());

    let manifest: Value = serde_json::from_slice(&std::fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["ledger"]["entries"], 1);
    assert_eq!(manifest["ledger"]["vat_returns"][0]["period"], 10);
    assert_eq!(manifest["ledger"]["vat_returns"][0]["payable_micros"], 21_000_000);

    let ledger = std::fs::read_to_string(bundle.join("ledger.jsonl")).unwrap();
    let record: Value = serde_json::from_str(ledger.lines().next().unwrap()).unwrap();
    assert_eq!(record["journal"], "VK");
    assert_eq!(record["source_id"], document.to_string());
    assert_eq!(record["lines"].as_array().unwrap().len(), 3, "debtors, revenue, VAT");

    assert_eq!(evidence::verify(&bundle).unwrap(), report);
}

#[test]
fn a_return_brings_the_whole_basis_it_names() {
    let (dir, mut database, seller, document) = archived();
    seed_starter_chart(database.connection(), NOW).unwrap();
    post_outbound_document(database.connection(), document, "test", NOW).unwrap();
    let other = sell(&mut database, seller, "2026-10-29");
    post_outbound_document(database.connection(), other, "test", NOW).unwrap();
    store::vat::prepare(database.connection(), seller, 2026, 10, "test", NOW).unwrap();

    let bundle = dir.path().join("bundle");
    let report = export(&database, seller, document, &bundle);
    assert_eq!(
        report.ledger_entries, 2,
        "the return cannot be explained by one of its two sales"
    );
    assert_eq!(report.vat_returns, 1);
    assert_eq!(evidence::verify(&bundle).unwrap(), report);
}

#[test]
fn a_forged_ledger_line_is_caught_even_with_a_matching_manifest() {
    let (dir, database, seller, document) = posted();
    let bundle = dir.path().join("bundle");
    export(&database, seller, document, &bundle);

    let mut records: Vec<Value> = std::fs::read_to_string(bundle.join("ledger.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    records[0]["description"] = json!("Consulting, restated");
    let body = records
        .iter()
        .map(|record| serde_json::to_string(record).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let hash = replace_and_rehash(&bundle, "ledger.jsonl", body.as_bytes());
    patch_manifest(&bundle, |manifest| {
        manifest["ledger"]["sha256"] = json!(hash);
    });

    let error = evidence::verify(&bundle).unwrap_err();
    assert!(
        error.to_string().contains("does not hash to what it records"),
        "{error}"
    );
}

#[test]
fn a_return_that_names_an_absent_entry_is_caught() {
    let (dir, database, seller, document) = posted();
    let bundle = dir.path().join("bundle");
    export(&database, seller, document, &bundle);

    let manifest: Value = serde_json::from_slice(&std::fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
    let return_file = manifest["ledger"]["vat_returns"][0]["file"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut stored: Value = serde_json::from_slice(&std::fs::read(bundle.join(&return_file)).unwrap()).unwrap();

    // Add an entry to the basis and rebuild the basis hash over the forged list, so
    // the file hash and the manifest agree and only the membership check is left.
    let forged = "00000000-0000-0000-0000-000000000000";
    let mut canonical = String::new();
    for entry in stored["basis"].as_array().unwrap() {
        let id = entry.as_str().unwrap();
        canonical.push_str(id);
        canonical.push(':');
        canonical.push_str(&ledger_content(&bundle, id));
        canonical.push('\n');
    }
    canonical.push_str(forged);
    canonical.push(':');
    canonical.push_str(&"0".repeat(64));
    canonical.push('\n');
    let basis_hash = audit::sha256_hex(canonical.as_bytes());
    stored["basis"].as_array_mut().unwrap().push(json!(forged));
    stored["basis_hash"] = json!(basis_hash);
    let bytes = serde_json::to_vec_pretty(&stored).unwrap();
    let hash = replace_and_rehash(&bundle, &return_file, &bytes);
    patch_manifest(&bundle, |manifest| {
        manifest["ledger"]["vat_returns"][0]["basis_hash"] = json!(basis_hash);
        manifest["ledger"]["vat_returns"][0]["sha256"] = json!(hash);
    });

    let error = evidence::verify(&bundle).unwrap_err();
    assert!(error.to_string().contains("which the bundle does not carry"), "{error}");
}

#[test]
fn a_bundle_written_before_the_ledger_still_verifies() {
    let (dir, database, seller, document) = posted();
    let bundle = dir.path().join("bundle");
    let report = export(&database, seller, document, &bundle);

    let manifest: Value = serde_json::from_slice(&std::fs::read(bundle.join("manifest.json")).unwrap()).unwrap();
    let ledger_file = manifest["ledger"]["file"].as_str().unwrap().to_owned();
    let return_file = manifest["ledger"]["vat_returns"][0]["file"]
        .as_str()
        .unwrap()
        .to_owned();
    patch_manifest(&bundle, |manifest| {
        manifest.as_object_mut().unwrap().remove("ledger");
        let kept: Vec<Value> = manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|file| file["path"] != json!(ledger_file) && file["path"] != json!(return_file))
            .cloned()
            .collect();
        manifest["files"] = json!(kept);
    });
    std::fs::remove_file(bundle.join(&ledger_file)).unwrap();
    std::fs::remove_dir_all(bundle.join("vat-returns")).unwrap();

    let verified = evidence::verify(&bundle).unwrap();
    assert_eq!(verified.ledger_entries, 0);
    assert_eq!(verified.vat_returns, 0);
    assert_eq!(verified.number, report.number);
    assert_eq!(verified.files, report.files - 2, "the ledger and the return are gone");
}
