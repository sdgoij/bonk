//! Importing a received UBL document into the archive.

use std::path::Path;

use crux::{CustomerId, DocumentId, Party, SellerId};

use store::{CustomerRecord, ImportSource, ReceivedDocument, ReceivedLine, StoreError};

/// What happened to one file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImportOutcome {
    /// File the document came from.
    pub path: String,
    /// The document in the archive (the existing one, when this was a repeat).
    pub document_id: DocumentId,
    /// Number as the sender wrote it.
    pub number: String,
    /// Supplier, when the party table matched one.
    pub supplier: Option<CustomerId>,
    /// True when nothing new was written because this document was already here.
    pub duplicate: bool,
}

/// Why a file was not imported.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The file could not be read.
    #[error("cannot read {path}: {detail}")]
    Read {
        /// Path that failed.
        path: String,
        /// Operating-system detail.
        detail: String,
    },
    /// The file is not UTF-8 text.
    #[error("the document is not UTF-8: {0}")]
    NotUtf8(String),
    /// The PDF did not contain an invoice we can read.
    #[error(transparent)]
    Pdf(#[from] super::PdfError),
    /// The document is not a UBL invoice or credit note we can represent.
    #[error(transparent)]
    Parse(#[from] syntax::ImportError),
    /// Storage failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The database has no seller to own the document.
    #[error("this database has no seller to own the document")]
    NoSeller,
}

/// Reads and imports one file.
pub fn import_file(
    database: &store::Database,
    owner_id: SellerId,
    path: &Path,
    now: &str,
) -> Result<ImportOutcome, ImportError> {
    let display = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|error| ImportError::Read {
        path: display.clone(),
        detail: error.to_string(),
    })?;
    import_bytes(database, owner_id, Some(&display), &bytes, now)
}

/// Imports a document already in memory.
///
/// `path` is provenance only; pass `None` when the bytes did not come from a
/// file. The same bytes are never recorded twice, and the same supplier number
/// is never recorded twice.
pub fn import_bytes(
    database: &store::Database,
    owner_id: SellerId,
    path: Option<&str>,
    bytes: &[u8],
    now: &str,
) -> Result<ImportOutcome, ImportError> {
    let text = match super::sniff(bytes) {
        super::SniffedKind::Pdf => {
            let extracted = super::pdf::extract_invoice(bytes)?;
            String::from_utf8(extracted.xml).map_err(|error| ImportError::NotUtf8(error.to_string()))?
        }
        _ => {
            let text = std::str::from_utf8(bytes).map_err(|error| ImportError::NotUtf8(error.to_string()))?;
            text.to_owned()
        }
    };
    let read = syntax::read_document(&text)?;
    let invoice = read.invoice;
    let payload_hash = store::audit::sha256_hex(bytes);
    let display = path.unwrap_or("(memory)").to_owned();
    let supplier = match_supplier(database, &invoice.seller)?;

    if let Some(existing) = store::find_import_by_payload(database.connection(), &payload_hash)? {
        return Ok(ImportOutcome {
            path: display,
            document_id: existing,
            number: invoice.number,
            supplier: supplier.map(|(customer_id, _)| customer_id),
            duplicate: true,
        });
    }

    if let Some((customer_id, _)) = supplier
        && let Some(existing) =
            store::find_inbound_number(database.connection(), owner_id, customer_id, &invoice.number)?
    {
        return Ok(ImportOutcome {
            path: display,
            document_id: existing,
            number: invoice.number,
            supplier: Some(customer_id),
            duplicate: true,
        });
    }

    let seller = store::load_seller(database.connection(), owner_id)?;
    let lines: Vec<ReceivedLine> = invoice
        .lines
        .iter()
        .map(|line| ReceivedLine {
            position: line.position,
            name: line.name.clone(),
            description: line.description.clone(),
            quantity: line.quantity,
            unit: line.unit,
            unit_price: line.unit_price,
            price_base_quantity: line.base_quantity,
            line_net: line.line_net,
            treatment: line.treatment.clone(),
        })
        .collect();

    let document = ReceivedDocument {
        number: &invoice.number,
        doc_type: invoice.doc_type,
        currency: invoice.currency,
        issue_date: invoice.issue_date,
        due_date: invoice.due_date,
        delivery_date: invoice.delivery_date,
        buyer_reference: invoice.buyer_reference.as_deref(),
        order_reference: invoice.order_reference.as_deref(),
        note: invoice.note.as_deref(),
        issuer: &invoice.seller,
        recipient: &seller.party,
        lines: &lines,
        totals: &invoice.totals,
        preceding_number: invoice.preceding.as_ref().map(|preceding| preceding.number.as_str()),
        preceding_issue_date: invoice.preceding.as_ref().and_then(|preceding| preceding.issue_date),
    };

    let source = ImportSource {
        kind: match (super::sniff(bytes), read.syntax) {
            (super::SniffedKind::Pdf, _) => "pdf",
            (_, syntax::Syntax::Ubl21) => "ubl",
            (_, syntax::Syntax::CiiD16B) => "cii",
        },
        path,
        payload_hash: &payload_hash,
        payload: bytes,
    };

    let id = store::record_received(
        database.connection(),
        owner_id,
        supplier.map(|(customer_id, _)| customer_id),
        &document,
        &source,
        supplier.map(|(_, confidence)| confidence),
        now,
    )?;

    Ok(ImportOutcome {
        path: display,
        document_id: id,
        number: invoice.number,
        supplier: supplier.map(|(customer_id, _)| customer_id),
        duplicate: false,
    })
}

/// Matches a supplier against the parties on file.
///
/// Suppliers are customers: there is one party table, and a party can be on
/// either side of a document.
pub fn match_supplier(database: &store::Database, supplier: &Party) -> Result<Option<(CustomerId, f64)>, StoreError> {
    let customers = store::list_customers(database.connection())?;
    Ok(best_match(&customers, supplier))
}

/// Picks the best match from `customers`, or `None` when nothing matches.
///
/// Confidence is coarse and honest: 1.0 for an identical VAT identifier, 2/3 for
/// an identical Peppol endpoint, 1/3 for the same legal name and city. Ties keep
/// the first candidate in listing order, which is deterministic.
///
/// A float is allowed here because a confidence never touches money; the
/// workspace lint against float arithmetic exists to protect amounts.
#[allow(clippy::float_arithmetic)]
pub fn best_match(customers: &[CustomerRecord], supplier: &Party) -> Option<(CustomerId, f64)> {
    let mut best: Option<(CustomerId, u8)> = None;
    for customer in customers {
        let score = score(&customer.party, supplier);
        if score == 0 {
            continue;
        }
        match best {
            Some((_, top)) if score <= top => {}
            _ => best = Some((customer.id, score)),
        }
    }
    best.map(|(id, score)| (id, f64::from(score) / 3.0))
}

fn score(customer: &Party, supplier: &Party) -> u8 {
    if same(customer.vat_id.as_deref(), supplier.vat_id.as_deref()) {
        return 3;
    }
    if same(customer.peppol_id.as_deref(), supplier.peppol_id.as_deref()) {
        return 2;
    }
    let same_name = same(Some(&customer.legal_name), Some(&supplier.legal_name));
    let same_city = same(Some(&customer.address.city), Some(&supplier.address.city));
    if same_name && same_city { 1 } else { 0 }
}

fn same(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left = normalise(left);
            !left.is_empty() && left == normalise(right)
        }
        _ => false,
    }
}

fn normalise(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use crux::{Address, CountryCode, Currency};
    use store::{CustomerRecord, Database, KdfParams, KeyStore, SellerRecord, StoreError};

    use super::*;

    fn party(name: &str, vat: Option<&str>, peppol: Option<&str>) -> Party {
        let mut party = Party::new(name, Address::new("Amsterdam", CountryCode::NL));
        party.vat_id = vat.map(str::to_owned);
        party.peppol_id = peppol.map(str::to_owned);
        party
    }

    fn customer(name: &str, vat: Option<&str>, peppol: Option<&str>) -> CustomerRecord {
        CustomerRecord {
            id: CustomerId::new(),
            party: party(name, vat, peppol),
        }
    }

    fn ubl(seller: &str, vat: Option<&str>, number: &str) -> Vec<u8> {
        use crux::{
            InvoiceTotals, LocalDate, Money, Quantity, TaxBreakdown, TaxTreatment, UnitCode, VatCategory, VatRate,
        };
        use syntax::{UblInvoice, UblLine, write_invoice};

        let treatment = TaxTreatment::standard(VatRate::STANDARD_NL);
        let line = UblLine {
            id: 1,
            name: "Consulting",
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(100, Currency::EUR).unwrap(),
            base_quantity: Quantity::ONE,
            line_net: Money::from_major_units(100, Currency::EUR).unwrap(),
            treatment: &treatment,
        };
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
        let lines = [line];
        let issuer = party(seller, vat, Some("0106:99999999"));
        let recipient = party("Us BV", Some("NL123456782B01"), None);
        let invoice = UblInvoice {
            number,
            issue_date: LocalDate::parse_iso("2026-10-27").unwrap(),
            due_date: Some(LocalDate::parse_iso("2026-11-26").unwrap()),
            delivery_date: None,
            doc_type: crux::DocType::Invoice,
            currency: Currency::EUR,
            buyer_reference: Some("PO-1"),
            order_reference: None,
            note: None,
            preceding_invoice: None,
            seller: &issuer,
            customer: &recipient,
            lines: &lines,
            totals: &totals,
        };
        write_invoice(&invoice).unwrap().into_bytes()
    }

    /// The same document in UN/CEFACT CII, as Factur-X and some access points
    /// send it.
    fn cii(seller: &str, vat: Option<&str>, number: &str) -> Vec<u8> {
        let registration = match vat {
            Some(vat) => format!(
                "<ram:SpecifiedTaxRegistration><ram:ID schemeID=\"VA\">{vat}</ram:ID></ram:SpecifiedTaxRegistration>"
            ),
            None => String::new(),
        };
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<rsm:CrossIndustryInvoice
  xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100"
  xmlns:ram="urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100"
  xmlns:udt="urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100">
  <rsm:ExchangedDocument>
    <ram:ID>{number}</ram:ID>
    <ram:TypeCode>380</ram:TypeCode>
    <ram:IssueDateTime><udt:DateTimeString format="102">20261027</udt:DateTimeString></ram:IssueDateTime>
  </rsm:ExchangedDocument>
  <rsm:SupplyChainTradeTransaction>
    <ram:IncludedSupplyChainTradeLineItem>
      <ram:AssociatedDocumentLineDocument><ram:LineID>1</ram:LineID></ram:AssociatedDocumentLineDocument>
      <ram:SpecifiedTradeProduct><ram:Name>Consulting</ram:Name></ram:SpecifiedTradeProduct>
      <ram:SpecifiedLineTradeAgreement>
        <ram:NetPriceProductTradePrice>
          <ram:ChargeAmount>100.00</ram:ChargeAmount>
          <ram:BasisQuantity unitCode="C62">1</ram:BasisQuantity>
        </ram:NetPriceProductTradePrice>
      </ram:SpecifiedLineTradeAgreement>
      <ram:SpecifiedLineTradeDelivery><ram:BilledQuantity unitCode="C62">1</ram:BilledQuantity></ram:SpecifiedLineTradeDelivery>
      <ram:SpecifiedLineTradeSettlement>
        <ram:ApplicableTradeTax>
          <ram:TypeCode>VAT</ram:TypeCode>
          <ram:CategoryCode>S</ram:CategoryCode>
          <ram:RateApplicablePercent>21</ram:RateApplicablePercent>
        </ram:ApplicableTradeTax>
        <ram:LineTotalAmount>100.00</ram:LineTotalAmount>
      </ram:SpecifiedLineTradeSettlement>
    </ram:IncludedSupplyChainTradeLineItem>
    <ram:ApplicableHeaderTradeAgreement>
      <ram:BuyerReference>PO-1</ram:BuyerReference>
      <ram:SellerTradeParty>
        <ram:Name>{seller}</ram:Name>
        {registration}
        <ram:PostalTradeAddress>
          <ram:CityName>Amsterdam</ram:CityName>
          <ram:CountryID>NL</ram:CountryID>
        </ram:PostalTradeAddress>
      </ram:SellerTradeParty>
      <ram:BuyerTradeParty>
        <ram:Name>Us BV</ram:Name>
        <ram:PostalTradeAddress>
          <ram:CityName>Amsterdam</ram:CityName>
          <ram:CountryID>NL</ram:CountryID>
        </ram:PostalTradeAddress>
      </ram:BuyerTradeParty>
    </ram:ApplicableHeaderTradeAgreement>
    <ram:ApplicableHeaderTradeSettlement>
      <ram:InvoiceCurrencyCode>EUR</ram:InvoiceCurrencyCode>
      <ram:ApplicableTradeTax>
        <ram:CalculatedAmount>21.00</ram:CalculatedAmount>
        <ram:TypeCode>VAT</ram:TypeCode>
        <ram:BasisAmount>100.00</ram:BasisAmount>
        <ram:CategoryCode>S</ram:CategoryCode>
        <ram:RateApplicablePercent>21</ram:RateApplicablePercent>
      </ram:ApplicableTradeTax>
      <ram:SpecifiedTradeSettlementHeaderMonetarySummation>
        <ram:LineTotalAmount>100.00</ram:LineTotalAmount>
        <ram:TaxBasisTotalAmount>100.00</ram:TaxBasisTotalAmount>
        <ram:TaxTotalAmount currencyID="EUR">21.00</ram:TaxTotalAmount>
        <ram:GrandTotalAmount>121.00</ram:GrandTotalAmount>
        <ram:DuePayableAmount>121.00</ram:DuePayableAmount>
      </ram:SpecifiedTradeSettlementHeaderMonetarySummation>
    </ram:ApplicableHeaderTradeSettlement>
  </rsm:SupplyChainTradeTransaction>
</rsm:CrossIndustryInvoice>"#
        )
        .into_bytes()
    }

    fn database() -> (tempfile::TempDir, Database, SellerId) {
        let dir = tempfile::TempDir::new().unwrap();
        let keystore = KeyStore::create(dir.path().join("keystore.bin"), "pass", KdfParams::TEST).unwrap();
        let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();
        let seller_id = SellerId::new();
        store::insert_seller(
            database.connection(),
            &SellerRecord {
                id: seller_id,
                party: party("Us BV", Some("NL123456782B01"), None),
                default_currency: Currency::EUR,
                default_locale: "nl-NL".to_owned(),
            },
            "2026-10-27T00:00:00Z",
        )
        .unwrap();
        (dir, database, seller_id)
    }

    #[test]
    fn matches_by_vat_then_endpoint_then_name_and_city() {
        let customers = vec![
            customer("Acme BV", Some("NL111111110B01"), None),
            customer("Other BV", None, Some("0106:22222222")),
            customer("Third BV", None, None),
        ];

        let by_vat = party("Acme BV", Some("nl111111110b01"), None);
        assert_eq!(best_match(&customers, &by_vat).map(|(id, _)| id), Some(customers[0].id));

        let by_endpoint = party("Irrelevant", None, Some("0106:22222222"));
        assert_eq!(
            best_match(&customers, &by_endpoint).map(|(id, _)| id),
            Some(customers[1].id)
        );

        let by_name = party("Third BV", None, None);
        let matched = best_match(&customers, &by_name).expect("name and city match");
        assert_eq!(matched.0, customers[2].id);
        assert!((0.33..0.34).contains(&matched.1), "confidence: {}", matched.1);
    }

    #[test]
    fn an_unknown_supplier_does_not_match() {
        let customers = vec![customer("Acme BV", Some("NL111111110B01"), None)];
        assert!(best_match(&customers, &party("Stranger BV", None, None)).is_none());
    }

    #[test]
    fn importing_stores_the_document_and_matches_the_supplier() {
        let (_dir, database, seller_id) = database();
        let supplier = customer("Acme BV", Some("NL111111110B01"), None);
        store::insert_customer(database.connection(), &supplier, "2026-10-27T00:00:00Z").unwrap();

        let bytes = ubl("Acme BV", Some("NL111111110B01"), "SUP-1001");
        let outcome =
            import_bytes(&database, seller_id, Some("in/1.xml"), &bytes, "2026-10-27T00:00:00Z").expect("import");

        assert!(!outcome.duplicate);
        assert_eq!(outcome.number, "SUP-1001");
        assert_eq!(outcome.supplier, Some(supplier.id));

        let (direction, number, counterparty, series): (String, String, Option<String>, Option<String>) = database
            .connection()
            .query_row(
                "SELECT direction, number, counterparty_id, series FROM document WHERE id = ?1",
                [outcome.document_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(direction, "inbound");
        assert_eq!(number, "SUP-1001");
        assert_eq!(counterparty.as_deref(), Some(supplier.id.to_string().as_str()));
        assert_eq!(series, None, "an inbound document has no series of ours");

        let lines: i64 = database
            .connection()
            .query_row(
                "SELECT count(*) FROM document_line WHERE document_id = ?1",
                [outcome.document_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(lines, 1);

        let status: String = database
            .connection()
            .query_row(
                "SELECT match_status FROM import_record WHERE document_id = ?1",
                [outcome.document_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "matched");

        // The audit chain still verifies with an imported document in it.
        assert_eq!(store::audit::verify_chain(database.connection()).unwrap(), 1);
    }

    #[test]
    fn imports_a_cii_document_as_the_same_kind_of_record() {
        let (_dir, database, seller_id) = database();
        let supplier = customer("Acme BV", Some("NL111111110B01"), None);
        store::insert_customer(database.connection(), &supplier, "2026-10-27T00:00:00Z").unwrap();

        let outcome = import_bytes(
            &database,
            seller_id,
            Some("in/2.xml"),
            &cii("Acme BV", Some("NL111111110B01"), "SUP-2001"),
            "2026-10-27T00:00:00Z",
        )
        .expect("import");

        assert_eq!(outcome.number, "SUP-2001");
        assert_eq!(outcome.supplier, Some(supplier.id));

        let (direction, kind): (String, String) = database
            .connection()
            .query_row(
                "SELECT d.direction, i.source_kind FROM document d
                 JOIN import_record i ON i.document_id = d.id WHERE d.id = ?1",
                [outcome.document_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(direction, "inbound");
        assert_eq!(kind, "cii", "the archive records which syntax arrived");

        let listed = store::list_received(database.connection(), seller_id).unwrap();
        assert_eq!(listed[0].number, "SUP-2001");
        assert_eq!(listed[0].supplier, "Acme BV");
        assert!(listed[0].matched);
    }

    #[test]
    fn imports_a_factur_x_pdf_as_the_same_record() {
        let (_dir, database, seller_id) = database();
        let supplier = customer("Acme BV", Some("NL111111110B01"), None);
        store::insert_customer(database.connection(), &supplier, "2026-10-27T00:00:00Z").unwrap();

        let xml = String::from_utf8(ubl("Acme BV", Some("NL111111110B01"), "SUP-3001")).unwrap();
        let pdf = crate::pdf::fixtures::container(&xml, None);
        let outcome =
            import_bytes(&database, seller_id, Some("in/3.pdf"), &pdf, "2026-10-27T00:00:00Z").expect("import");

        assert_eq!(outcome.number, "SUP-3001");
        assert_eq!(outcome.supplier, Some(supplier.id));

        let (direction, kind, size): (String, String, i64) = database
            .connection()
            .query_row(
                "SELECT d.direction, i.source_kind, b.size_bytes FROM document d
                 JOIN import_record i ON i.document_id = d.id
                 JOIN blob b ON b.sha256 = i.payload_hash
                 WHERE d.id = ?1",
                [outcome.document_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(direction, "inbound");
        assert_eq!(kind, "pdf");
        assert_eq!(size as usize, pdf.len(), "the blob keeps the file as received");

        let listed = store::list_received(database.connection(), seller_id).unwrap();
        assert_eq!(listed[0].number, "SUP-3001");
        assert_eq!(listed[0].supplier, "Acme BV");
    }

    #[test]
    fn the_same_invoice_as_xml_and_as_pdf_is_one_record() {
        let (_dir, database, seller_id) = database();
        let supplier = customer("Acme BV", Some("NL111111110B01"), None);
        store::insert_customer(database.connection(), &supplier, "2026-10-27T00:00:00Z").unwrap();

        let xml = ubl("Acme BV", Some("NL111111110B01"), "SUP-3002");
        let first = import_bytes(&database, seller_id, None, &xml, "2026-10-27T00:00:00Z").unwrap();
        let pdf = crate::pdf::fixtures::container(&String::from_utf8(xml).unwrap(), None);
        let second = import_bytes(&database, seller_id, None, &pdf, "2026-10-27T00:00:01Z").unwrap();

        assert!(
            second.duplicate,
            "the supplier number is the identity, not the container"
        );
        assert_eq!(second.document_id, first.document_id);
    }

    #[test]
    fn an_unmatched_supplier_is_kept_not_invented() {
        let (_dir, database, seller_id) = database();
        let bytes = ubl("Stranger BV", None, "X-1");
        let outcome = import_bytes(&database, seller_id, None, &bytes, "2026-10-27T00:00:00Z").expect("import");

        assert_eq!(outcome.supplier, None);
        let counterparty: Option<String> = database
            .connection()
            .query_row(
                "SELECT counterparty_id FROM document WHERE id = ?1",
                [outcome.document_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(counterparty, None);

        let (status, confidence): (String, Option<f64>) = database
            .connection()
            .query_row(
                "SELECT match_status, match_confidence FROM import_record WHERE document_id = ?1",
                [outcome.document_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "unmatched");
        assert_eq!(confidence, None);

        let customers: i64 = database
            .connection()
            .query_row("SELECT count(*) FROM customer", [], |row| row.get(0))
            .unwrap();
        assert_eq!(customers, 0, "no party is invented");
    }

    #[test]
    fn importing_the_same_bytes_twice_writes_one_document() {
        let (_dir, database, seller_id) = database();
        let bytes = ubl("Stranger BV", None, "X-1");
        let first = import_bytes(&database, seller_id, None, &bytes, "2026-10-27T00:00:00Z").unwrap();
        let again = import_bytes(&database, seller_id, None, &bytes, "2026-10-27T00:00:01Z").unwrap();

        assert!(again.duplicate);
        assert_eq!(again.document_id, first.document_id);
        let documents: i64 = database
            .connection()
            .query_row("SELECT count(*) FROM document", [], |row| row.get(0))
            .unwrap();
        assert_eq!(documents, 1);
    }

    #[test]
    fn the_same_supplier_number_twice_is_a_duplicate() {
        let (_dir, database, seller_id) = database();
        let supplier = customer("Acme BV", Some("NL111111110B01"), None);
        store::insert_customer(database.connection(), &supplier, "2026-10-27T00:00:00Z").unwrap();

        let first = import_bytes(
            &database,
            seller_id,
            None,
            &ubl("Acme BV", Some("NL111111110B01"), "SUP-1"),
            "2026-10-27T00:00:00Z",
        )
        .unwrap();
        // Same number, different bytes: a re-send that changed the line name.
        let re_send = String::from_utf8(ubl("Acme BV", Some("NL111111110B01"), "SUP-1"))
            .unwrap()
            .replace("Consulting", "Consultancy")
            .into_bytes();
        let again = import_bytes(&database, seller_id, None, &re_send, "2026-10-27T00:00:01Z").unwrap();

        assert!(again.duplicate, "the supplier number is the identity, not the bytes");
        assert_eq!(again.document_id, first.document_id);
        assert_eq!(again.supplier, Some(supplier.id));
    }

    #[test]
    fn the_payable_register_only_moves_forward() {
        use store::DocumentStatus;

        let (_dir, database, seller_id) = database();
        let outcome = import_bytes(
            &database,
            seller_id,
            None,
            &ubl("Acme BV", None, "X-7"),
            "2026-10-27T00:00:00Z",
        )
        .unwrap();
        let id = outcome.document_id;
        let connection = database.connection();

        let listed = store::list_received(connection, seller_id).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].status, DocumentStatus::Received);
        assert_eq!(listed[0].number, "X-7");
        assert!(!listed[0].matched);

        assert_eq!(
            store::advance_status(connection, id, DocumentStatus::Approved, "test", "2026-10-28T00:00:00Z").unwrap(),
            DocumentStatus::Approved
        );
        assert!(
            store::advance_status(connection, id, DocumentStatus::Approved, "test", "2026-10-28T00:00:01Z").is_err(),
            "a repeated status is refused"
        );
        assert!(
            store::advance_status(connection, id, DocumentStatus::Received, "test", "2026-10-28T00:00:02Z").is_err(),
            "statuses do not move backwards"
        );

        store::advance_status(connection, id, DocumentStatus::Paid, "test", "2026-10-29T00:00:00Z").unwrap();
        assert_eq!(
            store::list_received(connection, seller_id).unwrap()[0].status,
            DocumentStatus::Paid
        );

        // Append-only: the events that explain the status cannot be rewritten.
        let update = connection.execute("UPDATE document_event SET kind = 'archived'", []);
        assert!(update.is_err(), "events are append-only");
        let delete = connection.execute("DELETE FROM document_event", []);
        assert!(delete.is_err(), "events are append-only");
    }

    #[test]
    fn the_register_only_covers_received_documents() {
        use store::DocumentStatus;

        let (_dir, mut database, seller_id) = database();
        let customer = customer("Acme BV", None, None);
        store::insert_customer(database.connection(), &customer, "2026-10-27T00:00:00Z").unwrap();

        let mut draft = crux::InvoiceDraft::new(
            seller_id,
            customer.id,
            Currency::EUR,
            crux::LocalDate::parse_iso("2026-10-27").unwrap(),
        );
        draft.due_date = Some(crux::LocalDate::parse_iso("2026-11-26").unwrap());
        draft.lines.push(crux::DraftLine {
            id: crux::LineId::new(),
            position: 1,
            name: "Consulting".to_owned(),
            description: None,
            quantity: crux::Quantity::ONE,
            unit: crux::UnitCode::PIECE,
            unit_price: crux::Money::from_major_units(100, Currency::EUR).unwrap(),
            price_base_quantity: crux::Quantity::ONE,
            treatment: crux::TaxTreatment::standard(crux::VatRate::STANDARD_NL),
        });
        store::insert_draft(database.connection(), &draft, "2026-10-27T00:00:00Z").unwrap();
        let issued = store::issue_draft(
            database.connection_mut(),
            store::IssueRequest {
                draft_id: draft.id,
                actor: "test",
                device: "test",
                renderer_version: "test",
                now: "2026-10-27T00:00:00Z",
            },
            |_| Ok(b"<Invoice/>".to_vec()),
        )
        .unwrap();

        assert!(
            store::list_received(database.connection(), seller_id)
                .unwrap()
                .is_empty()
        );
        assert!(
            store::advance_status(
                database.connection(),
                issued.id,
                DocumentStatus::Paid,
                "test",
                "2026-10-28T00:00:00Z"
            )
            .is_err(),
            "an issued document is not in the payable register"
        );
        assert!(
            store::set_supplier_match(
                database.connection(),
                issued.id,
                customer.id,
                "test",
                "2026-10-28T00:00:00Z"
            )
            .is_err(),
            "an issued document has no supplier to match"
        );
    }

    #[test]
    fn a_supplier_can_be_matched_after_import() {
        let (_dir, database, seller_id) = database();
        let outcome = import_bytes(
            &database,
            seller_id,
            None,
            &ubl("Stranger BV", None, "X-9"),
            "2026-10-27T00:00:00Z",
        )
        .unwrap();
        let customer = customer("Stranger Holding BV", None, None);
        store::insert_customer(database.connection(), &customer, "2026-10-27T00:00:00Z").unwrap();

        let before = store::list_received(database.connection(), seller_id).unwrap();
        assert!(!before[0].matched);
        assert_eq!(before[0].supplier, "Stranger BV", "the name as sent");

        store::set_supplier_match(
            database.connection(),
            outcome.document_id,
            customer.id,
            "test",
            "2026-10-27T00:00:01Z",
        )
        .unwrap();

        let after = store::list_received(database.connection(), seller_id).unwrap();
        assert!(after[0].matched);
        assert_eq!(
            after[0].supplier, "Stranger Holding BV",
            "the party on file, not the sender's spelling"
        );

        // The decision is recorded as an event on the chain.
        assert_eq!(store::audit::verify_chain(database.connection()).unwrap(), 2);
    }

    #[test]
    fn search_follows_the_supplier_decision() {
        use store::{Direction, DocumentStatus, SearchRequest};

        let (_dir, database, seller_id) = database();
        let outcome = import_bytes(
            &database,
            seller_id,
            None,
            &ubl("Stranger BV", None, "X-11"),
            "2026-10-27T00:00:00Z",
        )
        .unwrap();

        let by_sender = store::search(
            database.connection(),
            seller_id,
            &SearchRequest {
                query: Some("stranger"),
                ..SearchRequest::default()
            },
        )
        .unwrap();
        assert_eq!(by_sender.len(), 1);
        assert_eq!(by_sender[0].id, outcome.document_id);
        assert_eq!(by_sender[0].party, "Stranger BV", "the name as sent");
        assert_eq!(by_sender[0].direction, Direction::Inbound);

        let customer = customer("Stranger Holding BV", None, None);
        store::insert_customer(database.connection(), &customer, "2026-10-27T00:00:00Z").unwrap();
        store::set_supplier_match(
            database.connection(),
            outcome.document_id,
            customer.id,
            "test",
            "2026-10-27T00:00:01Z",
        )
        .unwrap();

        let by_party = store::search(
            database.connection(),
            seller_id,
            &SearchRequest {
                query: Some("holding"),
                ..SearchRequest::default()
            },
        )
        .unwrap();
        assert_eq!(by_party.len(), 1, "the index followed the decision");
        assert_eq!(by_party[0].party, "Stranger Holding BV");

        // Status is a filter over the events, not an FTS column.
        store::advance_status(
            database.connection(),
            outcome.document_id,
            DocumentStatus::Approved,
            "test",
            "2026-10-28T00:00:00Z",
        )
        .unwrap();
        let approved = store::search(
            database.connection(),
            seller_id,
            &SearchRequest {
                query: Some("stranger"),
                status: Some(DocumentStatus::Approved),
                ..SearchRequest::default()
            },
        )
        .unwrap();
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].status, DocumentStatus::Approved);
        let received = store::search(
            database.connection(),
            seller_id,
            &SearchRequest {
                query: Some("stranger"),
                status: Some(DocumentStatus::Received),
                ..SearchRequest::default()
            },
        )
        .unwrap();
        assert!(received.is_empty(), "the document moved on");
    }

    #[test]
    fn an_imported_payload_is_covered_by_the_integrity_audit() {
        let (_dir, database, seller_id) = database();
        let bytes = ubl("Stranger BV", None, "X-12");
        import_bytes(&database, seller_id, None, &bytes, "2026-10-27T00:00:00Z").unwrap();

        let report = store::integrity::audit(database.connection()).unwrap();
        assert_eq!(report.imports, 1);
        assert_eq!(report.blobs, 1);
        assert_eq!(
            report.orphaned_blobs, 0,
            "the payload is linked by the hash the import recorded"
        );
    }

    #[test]
    fn an_imported_document_bundles_its_received_bytes() {
        let (dir, database, seller_id) = database();
        let outcome = import_bytes(
            &database,
            seller_id,
            None,
            &ubl("Acme BV", None, "X-21"),
            "2026-10-27T00:00:00Z",
        )
        .unwrap();

        let bundle = dir.path().join("evidence");
        let report = store::evidence::export(
            database.connection(),
            seller_id,
            outcome.document_id,
            &bundle,
            "test",
            "2026-10-27T00:00:01Z",
        )
        .unwrap();

        assert_eq!(report.direction, "inbound");
        assert!(
            bundle.join("artefacts/X-21-received.xml").exists(),
            "the bytes as received travel with the bundle"
        );
        assert_eq!(store::evidence::verify(&bundle).unwrap(), report);
    }

    #[test]
    fn a_non_ubl_file_is_reported_with_its_reason() {
        let (_dir, database, seller_id) = database();
        let error = import_bytes(
            &database,
            seller_id,
            Some("notes.csv"),
            b"a,b,c",
            "2026-10-27T00:00:00Z",
        )
        .unwrap_err();
        assert!(matches!(error, ImportError::Parse(_)), "{error}");
    }

    #[test]
    fn an_import_without_a_seller_is_reported() {
        let dir = tempfile::TempDir::new().unwrap();
        let keystore = KeyStore::create(dir.path().join("keystore.bin"), "pass", KdfParams::TEST).unwrap();
        let database = Database::create(dir.path().join("bonk.db"), &keystore).unwrap();
        let error = import_bytes(
            &database,
            SellerId::new(),
            None,
            &ubl("Acme BV", None, "X-1"),
            "2026-10-27T00:00:00Z",
        )
        .unwrap_err();
        assert!(matches!(error, ImportError::Store(StoreError::NotFound(_))), "{error}");
    }
}
