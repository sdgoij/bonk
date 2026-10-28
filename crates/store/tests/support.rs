#![allow(clippy::unwrap_used)]
//! Support bundles: useful diagnostics, and not one fact about the business.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocType, DraftLine, InvoiceDraft, InvoiceTotals, LineId, LocalDate,
    Money, Party, Quantity, SellerId, TaxBreakdown, TaxTreatment, UnitCode, VatCategory, VatRate,
};
use store::support::{self, SupportRequest};
use store::{
    CustomerRecord, Database, ImportSource, IssueRequest, KdfParams, KeyStore, ReceivedDocument, SellerRecord,
    insert_customer, insert_draft, issue_draft,
};
use tempfile::TempDir;

const NOW: &str = "2026-10-28T12:00:00Z";

// Strings that must never appear in a bundle. Each is distinctive on purpose.
const SELLER_NAME: &str = "Seller CANARY-7f3a";
const CUSTOMER_NAME: &str = "Customer CANARY-2b8c";
const VAT_ID: &str = "NL111111110B01";
const RECEIVED_NUMBER: &str = "SUPPLIER-CANARY-9911";
const AMOUNT_TEXT: &str = "121.00";

fn party(name: &str) -> Party {
    Party::new(name, Address::new("Amsterdam", CountryCode::NL))
}

fn archived() -> (TempDir, Database, KeyStore, SellerId) {
    let dir = TempDir::new().unwrap();
    let keystore = KeyStore::create(dir.path().join("keystore.bin"), "pass", KdfParams::TEST).unwrap();
    let mut database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();

    let seller_id = SellerId::new();
    let mut seller_party = party(SELLER_NAME);
    seller_party.vat_id = Some(VAT_ID.to_owned());
    let seller = SellerRecord {
        id: seller_id,
        party: seller_party,
        default_currency: Currency::EUR,
        default_locale: "nl-NL".to_owned(),
    };
    store::insert_seller(database.connection(), &seller, NOW).unwrap();

    let customer_id = CustomerId::new();
    let customer = CustomerRecord {
        id: customer_id,
        party: party(CUSTOMER_NAME),
    };
    insert_customer(database.connection(), &customer, NOW).unwrap();

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
    issue_draft(
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

    // A received document, so inbound counts, imports, and blobs are non-zero
    // and a supplier's number and name are present to be excluded.
    let net = Money::from_major_units(100, Currency::EUR).unwrap();
    let tax = Money::from_major_units(21, Currency::EUR).unwrap();
    let totals = InvoiceTotals {
        net,
        tax,
        gross: Money::from_major_units(121, Currency::EUR).unwrap(),
        breakdown: vec![TaxBreakdown {
            category: VatCategory::Standard,
            rate_bp: 2100,
            taxable_base: net,
            tax,
        }],
    };
    let issuer = party(SELLER_NAME);
    let received = ReceivedDocument {
        number: RECEIVED_NUMBER,
        doc_type: DocType::Invoice,
        currency: Currency::EUR,
        issue_date: LocalDate::parse_iso("2026-10-20").unwrap(),
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
    let payload = b"<Invoice>canary payload</Invoice>";
    store::record_received(
        database.connection(),
        seller_id,
        None,
        &received,
        &ImportSource {
            kind: "ubl",
            path: Some("canary.xml"),
            payload_hash: &store::audit::sha256_hex(payload),
            payload,
        },
        None,
        NOW,
    )
    .unwrap();

    (dir, database, keystore, seller_id)
}

#[test]
fn a_bundle_has_counts_and_no_business_facts() {
    let (dir, database, _keystore, _seller) = archived();
    let bundle = dir.path().join("support");

    let summary = support::write(
        &database,
        &SupportRequest {
            database_path: &dir.path().join("bonk.db"),
            keystore_path: &dir.path().join("keystore.bin"),
            app_version: "0.1.0-test",
            directory: &bundle,
            now: NOW,
        },
    )
    .unwrap();

    assert_eq!(summary.counts.sellers, 1);
    assert_eq!(summary.counts.customers, 1);
    assert_eq!(summary.counts.documents, 2);
    assert_eq!(summary.counts.outbound, 1);
    assert_eq!(summary.counts.inbound, 1);
    assert_eq!(summary.counts.imports, 1);
    assert_eq!(summary.integrity.orphaned_blobs, 0);
    assert!(summary.chain.is_some());

    let text = std::fs::read_to_string(bundle.join("summary.txt")).unwrap();
    let json = std::fs::read_to_string(bundle.join("summary.json")).unwrap();
    let document_id: String = database
        .connection()
        .query_row("SELECT id FROM document ORDER BY direction LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    let canaries = [
        SELLER_NAME,
        CUSTOMER_NAME,
        VAT_ID,
        RECEIVED_NUMBER,
        AMOUNT_TEXT,
        document_id.as_str(),
        dir.path().to_str().unwrap(),
        "canary.xml",
    ];
    for canary in canaries {
        assert!(!text.contains(canary), "summary.txt leaks {canary:?}:\n{text}");
        assert!(!json.contains(canary), "summary.json leaks {canary:?}:\n{json}");
    }

    // The bundle still has to be useful: counts, versions, and integrity travel.
    assert!(text.contains("documents:  2 (1 outbound, 1 inbound)"), "{text}");
    assert!(text.contains("issued 1"), "{text}");
    assert!(text.contains("imported 1"), "{text}");
    assert!(text.contains("encryption:"), "{text}");
    assert!(json.contains("\"schema_version\": 3"), "{json}");
    assert!(json.contains("\"format\": \"bonk-support\""), "{json}");
}

#[test]
fn an_existing_bundle_is_not_overwritten() {
    let (dir, database, _keystore, _seller) = archived();
    let bundle = dir.path().join("support");
    let request = SupportRequest {
        database_path: &dir.path().join("bonk.db"),
        keystore_path: &dir.path().join("keystore.bin"),
        app_version: "0.1.0-test",
        directory: &bundle,
        now: NOW,
    };
    support::write(&database, &request).unwrap();

    let error = support::write(&database, &request).unwrap_err();
    assert!(matches!(error, store::StoreError::Rejected(_)), "{error}");
}
