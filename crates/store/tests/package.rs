#![allow(clippy::unwrap_used)]
//! The accountant handoff package: reports, returns, listings, documents, and a
//! manifest that makes them checkable.

use crux::{
    Address, CountryCode, Currency, CustomerId, DocumentId, DraftLine, InvoiceDraft, LineId, LocalDate, Money, Party,
    Quantity, SellerId, TaxTreatment, UnitCode, VatCategory, VatRate,
};
use store::package;
use store::{
    CustomerRecord, Database, IssueRequest, KdfParams, KeyStore, SellerRecord, insert_customer, insert_draft,
    insert_seller, issue_draft, post_outbound_document, seed_starter_chart,
};
use tempfile::TempDir;

const NOW: &str = "2026-11-06T16:00:00.123456Z";

fn date(raw: &str) -> LocalDate {
    LocalDate::parse_iso(raw).unwrap()
}

fn euros(units: i64) -> Money {
    Money::from_major_units(units, Currency::EUR).unwrap()
}

fn country(code: &str) -> CountryCode {
    CountryCode::parse(code).unwrap()
}

struct Books {
    _dir: TempDir,
    database: Database,
    seller: SellerId,
    domestic: CustomerId,
    german: CustomerId,
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

        let domestic = add_customer(&database, "Klant BV", "NL", None);
        let german = add_customer(&database, "Kunde GmbH", "DE", Some("DE123456789"));
        Self {
            _dir: dir,
            database,
            seller,
            domestic,
            german,
        }
    }

    fn issue(&mut self, customer: CustomerId, treatment: TaxTreatment, net_units: i64, on: &str) -> DocumentId {
        let mut draft = InvoiceDraft::new(self.seller, customer, Currency::EUR, date(on));
        // Intra-community supplies need a delivery date; giving every sale one is
        // harmless and keeps the fixture from tripping BR-IC-11.
        draft.delivery_date = Some(date(on));
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
}

fn add_customer(database: &Database, name: &str, country_code: &str, vat_id: Option<&str>) -> CustomerId {
    let id = CustomerId::new();
    let mut party = Party::new(name, Address::new("Stad", country(country_code)));
    party.vat_id = vat_id.map(str::to_owned);
    insert_customer(database.connection(), &CustomerRecord { id, party }, NOW).unwrap();
    id
}

#[test]
fn the_package_carries_the_reports_returns_listings_and_documents() {
    let mut books = Books::open();
    books.issue(
        books.domestic,
        TaxTreatment::standard(VatRate::STANDARD_NL),
        1_000,
        "2026-01-10",
    );
    books.issue(
        books.german,
        TaxTreatment::zero(VatCategory::IntraCommunity),
        500,
        "2026-01-12",
    );
    store::vat::prepare(books.database.connection(), books.seller, 2026, 1, "ada", NOW).unwrap();

    let directory = books._dir.path().join("package");
    let report = package::export(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-01-31"),
        &directory,
        "ada",
        NOW,
    )
    .unwrap();

    assert_eq!(report.documents, 2);
    assert_eq!(report.journal_entries, 2);
    assert_eq!(report.vat_returns, 1);
    assert_eq!(report.icp_months, 1);
    assert!(report.files >= 10);
    assert!(report.head.is_some(), "the archive has an audit chain");

    let read = |path: &str| std::fs::read_to_string(directory.join(path)).unwrap();
    let manifest_text = read("manifest.json");
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
    assert_eq!(manifest["format"], package::FORMAT);
    assert_eq!(manifest["version"], package::VERSION);
    assert_eq!(manifest["counts"]["documents"], 2);
    assert_eq!(manifest["counts"]["vat_returns"], 1);

    // Every listed file is there and hashes to what the manifest says.
    let files = manifest["files"].as_array().unwrap();
    assert!(files.len() >= 10);
    for file in files {
        let path = file["path"].as_str().unwrap();
        let bytes = std::fs::read(directory.join(path)).unwrap();
        assert_eq!(
            store::audit::sha256_hex(&bytes),
            file["sha256"].as_str().unwrap(),
            "{path}"
        );
        assert_eq!(bytes.len() as u64, file["size"].as_u64().unwrap(), "{path}");
    }

    let trial = read("trial-balance.csv");
    assert!(
        trial
            .lines()
            .any(|line| line.starts_with("1300,") && line.contains(",asset,1710.00,")),
        "{trial}"
    );
    let profit = read("profit-and-loss.csv");
    assert!(profit.lines().any(|line| line.starts_with("revenue,8000,")), "{profit}");
    assert!(profit.lines().any(|line| line.starts_with("revenue,8030,")), "{profit}");
    assert!(
        read("balance-sheet.csv")
            .lines()
            .any(|line| line.starts_with("asset,1300,"))
    );

    let journal = read("journal.csv");
    assert!(journal.contains("VK,2026-0001"), "{journal}");
    assert!(journal.contains("VK,2026-0002"), "{journal}");

    let open = read("open-items.csv");
    assert!(open.contains("receivable,2026-0001"), "{open}");
    assert!(open.contains("receivable,2026-0002"), "{open}");
    assert!(open.contains("Kunde GmbH"), "{open}");

    let return_json: serde_json::Value = serde_json::from_str(&read("vat/vat-return-2026-01.json")).unwrap();
    let lines = return_json["lines"].as_array().unwrap();
    assert!(
        lines
            .iter()
            .any(|line| line["rubriek"] == "1a" && line["base"] == "1000.00")
    );
    assert!(
        lines
            .iter()
            .any(|line| line["rubriek"] == "3a" && line["base"] == "500.00")
    );
    assert_eq!(return_json["payable"], "210.00");

    let icp = read("icp/icp-2026-01.csv");
    assert!(icp.contains("DE123456789,Kunde GmbH,500.00"), "{icp}");

    assert!(
        directory.join("documents/outbound-2026-0001-ubl.xml").exists(),
        "the UBL artefact travels with the package"
    );
    assert!(directory.join("documents/outbound-2026-0002-ubl.xml").exists());
    assert!(read("README.txt").contains("open-items.csv"));

    // A second export of the same window writes the same files, in the same order.
    let again = books._dir.path().join("again");
    package::export(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-01-31"),
        &again,
        "ada",
        NOW,
    )
    .unwrap();
    let manifest_again: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(again.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["files"], manifest_again["files"]);
    assert_eq!(
        std::fs::read_to_string(directory.join("journal.csv")).unwrap(),
        std::fs::read_to_string(again.join("journal.csv")).unwrap()
    );
}

#[test]
fn a_package_refuses_while_an_icp_line_has_no_vat_id() {
    let mut books = Books::open();
    // Reverse-charged services go to 3a, and this customer has no VAT identifier.
    books.issue(
        books.domestic,
        TaxTreatment::zero(VatCategory::ReverseCharge),
        400,
        "2026-01-20",
    );

    let directory = books._dir.path().join("package");
    let error = package::export(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-01-31"),
        &directory,
        "ada",
        NOW,
    )
    .unwrap_err();

    assert!(error.to_string().contains("ICP listing"), "{error}");
    assert!(error.to_string().contains("2026-0001"), "{error}");
    assert!(
        !directory.join("manifest.json").exists(),
        "a refused package leaves no manifest behind"
    );
}

#[test]
fn a_package_is_not_written_over() {
    let mut books = Books::open();
    books.issue(
        books.domestic,
        TaxTreatment::standard(VatRate::STANDARD_NL),
        1_000,
        "2026-01-10",
    );
    let directory = books._dir.path().join("package");
    package::export(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-01-31"),
        &directory,
        "ada",
        NOW,
    )
    .unwrap();

    let error = package::export(
        books.database.connection(),
        books.seller,
        date("2026-01-01"),
        date("2026-01-31"),
        &directory,
        "ada",
        NOW,
    )
    .unwrap_err();
    assert!(error.to_string().contains("already contains a package"), "{error}");
}
