//! CII D16B reader: a received CrossIndustryInvoice becomes the terms we model.
//!
//! Factur-X and ZUGFeRD carry EN 16931 in CII, and some access points send CII
//! directly. The business terms are the same as in UBL — the BT/BG numbers do
//! not change — so this reader maps element names onto the same
//! [`ImportedInvoice`] the UBL reader produces, and the rest of the pipeline
//! never learns which syntax arrived.

use crux::{
    Address, CountryCode, Currency, DocType, InvoiceTotals, LocalDate, Party, Quantity, TaxBreakdown, TaxTreatment,
    UnitCode, VatCategory, VatRate,
};

use crate::dom::{Node, money, namespace, text};
use crate::read::{ImportError, ImportedInvoice, ImportedLine, ImportedReference};

/// UN/CEFACT Cross Industry Invoice D16B.
pub(crate) const CII_NAMESPACE: &str = "urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100";

/// Invoice, corrected invoice, and credit note (BT-3).
const TYPE_INVOICE: &str = "380";
const TYPE_CREDIT_NOTE: &str = "381";
const TYPE_CORRECTED_INVOICE: &str = "384";

pub(crate) fn from_root(root: &Node) -> Result<ImportedInvoice, ImportError> {
    match namespace(root) {
        Some(namespace) if namespace == CII_NAMESPACE => {}
        other => {
            return Err(ImportError::NotCii(match other {
                Some(namespace) => format!("{} in {namespace}", root.name),
                None => root.name.clone(),
            }));
        }
    }

    let document = root
        .first("ExchangedDocument")
        .ok_or(ImportError::Missing("rsm:ExchangedDocument"))?;
    let transaction = root
        .first("SupplyChainTradeTransaction")
        .ok_or(ImportError::Missing("rsm:SupplyChainTradeTransaction"))?;
    let agreement = transaction
        .first("ApplicableHeaderTradeAgreement")
        .ok_or(ImportError::Missing("ram:ApplicableHeaderTradeAgreement"))?;
    let settlement = transaction
        .first("ApplicableHeaderTradeSettlement")
        .ok_or(ImportError::Missing("ram:ApplicableHeaderTradeSettlement"))?;

    let doc_type = match text(document, "TypeCode").as_deref() {
        Some(TYPE_CREDIT_NOTE) => DocType::CreditNote,
        Some(TYPE_INVOICE | TYPE_CORRECTED_INVOICE) => DocType::Invoice,
        Some(other) => {
            return Err(ImportError::Invalid {
                field: "ram:TypeCode",
                value: other.to_owned(),
            });
        }
        None => return Err(ImportError::Missing("ram:TypeCode")),
    };

    let number = required(document, "ID", "ram:ID")?;
    let issue_date = date(
        document
            .first("IssueDateTime")
            .ok_or(ImportError::Missing("ram:IssueDateTime"))?,
    )?;
    let currency = Currency::parse(&required(settlement, "InvoiceCurrencyCode", "ram:InvoiceCurrencyCode")?)
        .map_err(|error| invalid("ram:InvoiceCurrencyCode", error.to_string()))?;

    let seller = party(agreement, "SellerTradeParty").ok_or(ImportError::Missing("ram:SellerTradeParty/ram:Name"))?;
    let customer = party(agreement, "BuyerTradeParty").ok_or(ImportError::Missing("ram:BuyerTradeParty/ram:Name"))?;

    let mut lines = Vec::new();
    for (index, item) in transaction.children("IncludedSupplyChainTradeLineItem").enumerate() {
        lines.push(line(item, currency, index)?);
    }

    let delivery_date = transaction
        .first("ApplicableHeaderTradeDelivery")
        .and_then(|delivery| delivery.first("ActualDeliverySupplyChainEvent"))
        .and_then(|event| event.first("OccurrenceDateTime"))
        .map(date)
        .transpose()?;

    let due_date = settlement
        .first("SpecifiedTradePaymentTerms")
        .and_then(|terms| terms.first("DueDateDateTime"))
        .map(date)
        .transpose()?;

    Ok(ImportedInvoice {
        doc_type,
        number,
        issue_date,
        due_date,
        delivery_date,
        currency,
        buyer_reference: text(agreement, "BuyerReference"),
        order_reference: agreement
            .first("SellerOrderReferencedDocument")
            .and_then(|order| text(order, "IssuerAssignedID")),
        note: document.first("IncludedNote").and_then(|note| text(note, "Content")),
        profile: root
            .first("ExchangedDocumentContext")
            .and_then(|context| context.first("GuidelineSpecifiedDocumentContextParameter"))
            .and_then(|parameter| text(parameter, "ID")),
        preceding: preceding(settlement)?,
        seller,
        customer,
        lines,
        totals: totals(settlement, currency)?,
    })
}

fn line(item: &Node, currency: Currency, index: usize) -> Result<ImportedLine, ImportError> {
    let position = item
        .first("AssociatedDocumentLineDocument")
        .and_then(|document| text(document, "LineID"))
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_else(|| u32::try_from(index + 1).unwrap_or(u32::MAX));

    let quantity_element = item
        .first("SpecifiedLineTradeDelivery")
        .and_then(|delivery| delivery.first("BilledQuantity"))
        .ok_or(ImportError::Missing("ram:BilledQuantity"))?;
    let quantity =
        Quantity::parse(&quantity_element.text).map_err(|error| invalid("ram:BilledQuantity", error.to_string()))?;
    let unit = UnitCode::parse(
        &quantity_element
            .attribute("unitCode")
            .unwrap_or_else(|| "C62".to_owned()),
    )
    .map_err(|error| invalid("ram:BilledQuantity/@unitCode", error.to_string()))?;

    let price = item
        .first("SpecifiedLineTradeAgreement")
        .and_then(|agreement| agreement.first("NetPriceProductTradePrice"));
    let unit_price = money(
        price.and_then(|price| price.first("ChargeAmount")),
        currency,
        "ram:ChargeAmount",
    )?
    .ok_or(ImportError::Missing("ram:NetPriceProductTradePrice/ram:ChargeAmount"))?;
    let base_quantity = price
        .and_then(|price| price.first("BasisQuantity"))
        .map(|node| Quantity::parse(&node.text))
        .transpose()
        .map_err(|error| invalid("ram:BasisQuantity", error.to_string()))?
        .unwrap_or(Quantity::ONE);

    let product = item.first("SpecifiedTradeProduct");
    let name = product
        .and_then(|product| text(product, "Name"))
        .or_else(|| product.and_then(|product| text(product, "Description")))
        .ok_or(ImportError::Missing("ram:SpecifiedTradeProduct/ram:Name"))?;
    let description = product.and_then(|product| text(product, "Description"));

    let settlement = item
        .first("SpecifiedLineTradeSettlement")
        .ok_or(ImportError::Missing("ram:SpecifiedLineTradeSettlement"))?;
    let line_net = money(settlement.first("LineTotalAmount"), currency, "ram:LineTotalAmount")?
        .ok_or(ImportError::Missing("ram:LineTotalAmount"))?;

    let tax = settlement.first("ApplicableTradeTax");
    let vat_category = tax
        .and_then(|tax| text(tax, "CategoryCode"))
        .map(|code| VatCategory::parse(&code))
        .transpose()
        .map_err(|error| invalid("ram:CategoryCode", error.to_string()))?
        .unwrap_or(VatCategory::Standard);
    let rate = tax
        .and_then(|tax| text(tax, "RateApplicablePercent"))
        .map(|percent| VatRate::parse_percent(&percent))
        .transpose()
        .map_err(|error| invalid("ram:RateApplicablePercent", error.to_string()))?
        .unwrap_or(VatRate::ZERO);

    Ok(ImportedLine {
        position,
        name,
        description,
        quantity,
        unit,
        unit_price,
        base_quantity,
        line_net,
        treatment: TaxTreatment {
            category: vat_category,
            rate,
            exemption_reason: tax.and_then(|tax| text(tax, "ExemptionReason")),
            exemption_reason_code: tax.and_then(|tax| text(tax, "ExemptionReasonCode")),
        },
    })
}

fn totals(settlement: &Node, currency: Currency) -> Result<InvoiceTotals, ImportError> {
    let summation = settlement
        .first("SpecifiedTradeSettlementHeaderMonetarySummation")
        .ok_or(ImportError::Missing(
            "ram:SpecifiedTradeSettlementHeaderMonetarySummation",
        ))?;
    let net = money(
        summation.first("TaxBasisTotalAmount"),
        currency,
        "ram:TaxBasisTotalAmount",
    )?
    .ok_or(ImportError::Missing("ram:TaxBasisTotalAmount"))?;
    let tax = money(summation.first("TaxTotalAmount"), currency, "ram:TaxTotalAmount")?
        .ok_or(ImportError::Missing("ram:TaxTotalAmount"))?;
    let gross = money(summation.first("GrandTotalAmount"), currency, "ram:GrandTotalAmount")?
        .ok_or(ImportError::Missing("ram:GrandTotalAmount"))?;

    let mut breakdown = Vec::new();
    for row in settlement.children("ApplicableTradeTax") {
        let vat_category = text(row, "CategoryCode")
            .map(|code| VatCategory::parse(&code))
            .transpose()
            .map_err(|error| invalid("ram:CategoryCode", error.to_string()))?
            .unwrap_or(VatCategory::Standard);
        let rate_bp = text(row, "RateApplicablePercent")
            .map(|percent| VatRate::parse_percent(&percent))
            .transpose()
            .map_err(|error| invalid("ram:RateApplicablePercent", error.to_string()))?
            .unwrap_or(VatRate::ZERO)
            .basis_points();
        breakdown.push(TaxBreakdown {
            category: vat_category,
            rate_bp,
            taxable_base: money(row.first("BasisAmount"), currency, "ram:BasisAmount")?
                .ok_or(ImportError::Missing("ram:ApplicableTradeTax/ram:BasisAmount"))?,
            tax: money(row.first("CalculatedAmount"), currency, "ram:CalculatedAmount")?
                .ok_or(ImportError::Missing("ram:ApplicableTradeTax/ram:CalculatedAmount"))?,
        });
    }

    Ok(InvoiceTotals {
        net,
        tax,
        gross,
        breakdown,
    })
}

fn preceding(settlement: &Node) -> Result<Option<ImportedReference>, ImportError> {
    let Some(reference) = settlement.first("InvoiceReferencedDocument") else {
        return Ok(None);
    };
    let Some(number) = text(reference, "IssuerAssignedID") else {
        return Ok(None);
    };
    let issue_date = reference.first("FormattedIssueDateTime").map(date).transpose()?;
    Ok(Some(ImportedReference { number, issue_date }))
}

fn party(agreement: &Node, wrapper: &str) -> Option<Party> {
    let party = agreement.first(wrapper)?;
    let name = text(party, "Name")?;

    let postal = party.first("PostalTradeAddress");
    let country = postal
        .and_then(|postal| text(postal, "CountryID"))
        .and_then(|code| CountryCode::parse(&code).ok())?;

    let mut parsed = Party::new(
        name,
        Address {
            street: postal.and_then(|postal| text(postal, "LineOne")),
            city: postal.and_then(|postal| text(postal, "CityName")).unwrap_or_default(),
            postal_code: postal.and_then(|postal| text(postal, "PostcodeCode")),
            country,
        },
    );

    parsed.trading_name = party
        .first("SpecifiedLegalOrganization")
        .and_then(|legal| text(legal, "TradingBusinessName"));
    parsed.vat_id = party
        .children("SpecifiedTaxRegistration")
        .find(|registration| {
            registration
                .first("ID")
                .and_then(|id| id.attribute("schemeID"))
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("VA"))
        })
        .and_then(|registration| text(registration, "ID"));
    parsed.kvk_number = party
        .first("SpecifiedLegalOrganization")
        .and_then(|legal| legal.first("ID"))
        .filter(|id| id.attribute("schemeID").is_none_or(|scheme| scheme == "0106"))
        .map(|id| id.text.trim().to_owned())
        .filter(|value| !value.is_empty());
    parsed.peppol_id = party
        .first("URIUniversalCommunication")
        .and_then(|communication| communication.first("URIID"))
        .map(|uri| {
            let value = uri.text.trim().to_owned();
            match uri.attribute("schemeID") {
                Some(scheme) if !scheme.is_empty() => format!("{scheme}:{value}"),
                _ => value,
            }
        });
    Some(parsed)
}

fn required(node: &Node, name: &str, field: &'static str) -> Result<String, ImportError> {
    text(node, name).ok_or(ImportError::Missing(field))
}

/// Reads a CII date, which is written as `CCYYMMDD` under `format="102"`.
fn date(node: &Node) -> Result<LocalDate, ImportError> {
    let string = node
        .first("DateTimeString")
        .ok_or(ImportError::Missing("udt:DateTimeString"))?;
    let format = string.attribute("format").unwrap_or_else(|| "102".to_owned());
    if format != "102" {
        return Err(ImportError::Invalid {
            field: "udt:DateTimeString/@format",
            value: format,
        });
    }
    let value = string.text.trim();
    let iso = compact_to_iso(value).ok_or_else(|| ImportError::Invalid {
        field: "udt:DateTimeString",
        value: value.to_owned(),
    })?;
    LocalDate::parse_iso(&iso).map_err(|error| ImportError::Invalid {
        field: "udt:DateTimeString",
        value: format!("{value} ({error})"),
    })
}

fn compact_to_iso(value: &str) -> Option<String> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..]))
}

fn invalid(field: &'static str, detail: String) -> ImportError {
    ImportError::Invalid { field, value: detail }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::read_document;

    /// A Peppol BIS 3.0 invoice in CII, as an access point would send it.
    fn written(number: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<rsm:CrossIndustryInvoice
  xmlns:rsm="urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100"
  xmlns:ram="urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100"
  xmlns:udt="urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100">
  <rsm:ExchangedDocumentContext>
    <ram:GuidelineSpecifiedDocumentContextParameter>
      <ram:ID>urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0</ram:ID>
    </ram:GuidelineSpecifiedDocumentContextParameter>
  </rsm:ExchangedDocumentContext>
  <rsm:ExchangedDocument>
    <ram:ID>{number}</ram:ID>
    <ram:TypeCode>380</ram:TypeCode>
    <ram:IssueDateTime>
      <udt:DateTimeString format="102">20261027</udt:DateTimeString>
    </ram:IssueDateTime>
    <ram:IncludedNote>
      <ram:Content>Thank you</ram:Content>
    </ram:IncludedNote>
  </rsm:ExchangedDocument>
  <rsm:SupplyChainTradeTransaction>
    <ram:IncludedSupplyChainTradeLineItem>
      <ram:AssociatedDocumentLineDocument>
        <ram:LineID>1</ram:LineID>
      </ram:AssociatedDocumentLineDocument>
      <ram:SpecifiedTradeProduct>
        <ram:Name>Consulting</ram:Name>
        <ram:Description>Advice</ram:Description>
      </ram:SpecifiedTradeProduct>
      <ram:SpecifiedLineTradeAgreement>
        <ram:NetPriceProductTradePrice>
          <ram:ChargeAmount>50.00</ram:ChargeAmount>
          <ram:BasisQuantity unitCode="C62">1</ram:BasisQuantity>
        </ram:NetPriceProductTradePrice>
      </ram:SpecifiedLineTradeAgreement>
      <ram:SpecifiedLineTradeDelivery>
        <ram:BilledQuantity unitCode="C62">2</ram:BilledQuantity>
      </ram:SpecifiedLineTradeDelivery>
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
        <ram:ID schemeID="0106">12345678</ram:ID>
        <ram:Name>Seller BV</ram:Name>
        <ram:SpecifiedLegalOrganization>
          <ram:ID schemeID="0106">12345678</ram:ID>
          <ram:TradingBusinessName>Seller Trading</ram:TradingBusinessName>
        </ram:SpecifiedLegalOrganization>
        <ram:PostalTradeAddress>
          <ram:PostcodeCode>1000 AA</ram:PostcodeCode>
          <ram:LineOne>Main 1</ram:LineOne>
          <ram:CityName>Amsterdam</ram:CityName>
          <ram:CountryID>NL</ram:CountryID>
        </ram:PostalTradeAddress>
        <ram:URIUniversalCommunication>
          <ram:URIID schemeID="0106">12345678</ram:URIID>
        </ram:URIUniversalCommunication>
        <ram:SpecifiedTaxRegistration>
          <ram:ID schemeID="VA">NL123456782B01</ram:ID>
        </ram:SpecifiedTaxRegistration>
      </ram:SellerTradeParty>
      <ram:BuyerTradeParty>
        <ram:Name>Customer BV</ram:Name>
        <ram:PostalTradeAddress>
          <ram:PostcodeCode>3000 AA</ram:PostcodeCode>
          <ram:LineOne>Buyer 1</ram:LineOne>
          <ram:CityName>Rotterdam</ram:CityName>
          <ram:CountryID>NL</ram:CountryID>
        </ram:PostalTradeAddress>
      </ram:BuyerTradeParty>
      <ram:SellerOrderReferencedDocument>
        <ram:IssuerAssignedID>ORDER-7</ram:IssuerAssignedID>
      </ram:SellerOrderReferencedDocument>
    </ram:ApplicableHeaderTradeAgreement>
    <ram:ApplicableHeaderTradeDelivery>
      <ram:ActualDeliverySupplyChainEvent>
        <ram:OccurrenceDateTime>
          <udt:DateTimeString format="102">20261025</udt:DateTimeString>
        </ram:OccurrenceDateTime>
      </ram:ActualDeliverySupplyChainEvent>
    </ram:ApplicableHeaderTradeDelivery>
    <ram:ApplicableHeaderTradeSettlement>
      <ram:InvoiceCurrencyCode>EUR</ram:InvoiceCurrencyCode>
      <ram:ApplicableTradeTax>
        <ram:CalculatedAmount>21.00</ram:CalculatedAmount>
        <ram:TypeCode>VAT</ram:TypeCode>
        <ram:BasisAmount>100.00</ram:BasisAmount>
        <ram:CategoryCode>S</ram:CategoryCode>
        <ram:RateApplicablePercent>21</ram:RateApplicablePercent>
      </ram:ApplicableTradeTax>
      <ram:SpecifiedTradePaymentTerms>
        <ram:DueDateDateTime>
          <udt:DateTimeString format="102">20261126</udt:DateTimeString>
        </ram:DueDateDateTime>
      </ram:SpecifiedTradePaymentTerms>
      <ram:SpecifiedTradeSettlementHeaderMonetarySummation>
        <ram:LineTotalAmount>100.00</ram:LineTotalAmount>
        <ram:TaxBasisTotalAmount>100.00</ram:TaxBasisTotalAmount>
        <ram:TaxTotalAmount currencyID="EUR">21.00</ram:TaxTotalAmount>
        <ram:GrandTotalAmount>121.00</ram:GrandTotalAmount>
        <ram:DuePayableAmount>121.00</ram:DuePayableAmount>
      </ram:SpecifiedTradeSettlementHeaderMonetarySummation>
      <ram:InvoiceReferencedDocument>
        <ram:IssuerAssignedID>2026-0007</ram:IssuerAssignedID>
        <ram:FormattedIssueDateTime>
          <udt:DateTimeString format="102">20260930</udt:DateTimeString>
        </ram:FormattedIssueDateTime>
      </ram:InvoiceReferencedDocument>
    </ram:ApplicableHeaderTradeSettlement>
  </rsm:SupplyChainTradeTransaction>
</rsm:CrossIndustryInvoice>"#
        )
    }

    #[test]
    fn reads_a_peppol_cii_invoice() {
        let document = read_document(&written("SUP-1")).expect("import");
        assert_eq!(document.syntax, crate::Syntax::CiiD16B);
        let invoice = document.invoice;

        assert_eq!(invoice.doc_type, DocType::Invoice);
        assert_eq!(invoice.number, "SUP-1");
        assert_eq!(invoice.issue_date, LocalDate::parse_iso("2026-10-27").unwrap());
        assert_eq!(invoice.due_date, Some(LocalDate::parse_iso("2026-11-26").unwrap()));
        assert_eq!(invoice.delivery_date, Some(LocalDate::parse_iso("2026-10-25").unwrap()));
        assert_eq!(invoice.currency, Currency::EUR);
        assert_eq!(invoice.buyer_reference.as_deref(), Some("PO-1"));
        assert_eq!(invoice.order_reference.as_deref(), Some("ORDER-7"));
        assert_eq!(invoice.note.as_deref(), Some("Thank you"));
        assert_eq!(
            invoice.profile.as_deref(),
            Some("urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0")
        );

        let seller = &invoice.seller;
        assert_eq!(seller.legal_name, "Seller BV");
        assert_eq!(seller.trading_name.as_deref(), Some("Seller Trading"));
        assert_eq!(seller.vat_id.as_deref(), Some("NL123456782B01"));
        assert_eq!(seller.kvk_number.as_deref(), Some("12345678"));
        assert_eq!(seller.peppol_id.as_deref(), Some("0106:12345678"));
        assert_eq!(seller.address.country, CountryCode::NL);
        assert_eq!(seller.address.street.as_deref(), Some("Main 1"));
        assert_eq!(seller.address.city, "Amsterdam");
        assert_eq!(seller.address.postal_code.as_deref(), Some("1000 AA"));
        assert_eq!(invoice.customer.legal_name, "Customer BV");
        assert_eq!(invoice.customer.address.city, "Rotterdam");

        assert_eq!(invoice.lines.len(), 1);
        let line = &invoice.lines[0];
        assert_eq!(line.position, 1);
        assert_eq!(line.name, "Consulting");
        assert_eq!(line.description.as_deref(), Some("Advice"));
        assert_eq!(line.quantity, Quantity::from_units(2).unwrap());
        assert_eq!(line.unit, UnitCode::PIECE);
        assert_eq!(line.unit_price.micros(), 50_000_000);
        assert_eq!(line.line_net.micros(), 100_000_000);
        assert_eq!(line.treatment.category, VatCategory::Standard);
        assert_eq!(line.treatment.rate, VatRate::STANDARD_NL);

        assert_eq!(invoice.totals.net.micros(), 100_000_000);
        assert_eq!(invoice.totals.tax.micros(), 21_000_000);
        assert_eq!(invoice.totals.gross.micros(), 121_000_000);
        assert_eq!(invoice.totals.breakdown.len(), 1);
        assert_eq!(invoice.totals.breakdown[0].rate_bp, 2100);
        assert_eq!(invoice.totals.breakdown[0].tax.micros(), 21_000_000);
    }

    #[test]
    fn reads_a_credit_note_with_its_preceding_invoice() {
        let xml = written("SUP-C1").replace("<ram:TypeCode>380</ram:TypeCode>", "<ram:TypeCode>381</ram:TypeCode>");
        let invoice = read_document(&xml).expect("import").invoice;
        assert_eq!(invoice.doc_type, DocType::CreditNote);
        let preceding = invoice.preceding.expect("BG-3");
        assert_eq!(preceding.number, "2026-0007");
        assert_eq!(preceding.issue_date, Some(LocalDate::parse_iso("2026-09-30").unwrap()));
    }

    #[test]
    fn a_corrected_invoice_is_an_invoice() {
        let xml = written("SUP-2").replace("<ram:TypeCode>380</ram:TypeCode>", "<ram:TypeCode>384</ram:TypeCode>");
        assert_eq!(read_document(&xml).expect("import").invoice.doc_type, DocType::Invoice);
    }

    #[test]
    fn an_unknown_type_code_is_reported() {
        let xml = written("SUP-3").replace("<ram:TypeCode>380</ram:TypeCode>", "<ram:TypeCode>999</ram:TypeCode>");
        assert!(matches!(
            read_document(&xml),
            Err(ImportError::Invalid {
                field: "ram:TypeCode",
                ..
            })
        ));
    }

    #[test]
    fn a_cii_document_in_another_namespace_is_rejected() {
        let xml = written("SUP-4").replace(
            "urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100",
            "urn:example:not-cii",
        );
        assert!(matches!(read_document(&xml), Err(ImportError::NotCii(_))));
    }

    #[test]
    fn a_non_102_date_format_is_reported() {
        let xml = written("SUP-5").replace(
            r#"<udt:DateTimeString format="102">20261027</udt:DateTimeString>"#,
            r#"<udt:DateTimeString format="610">202610</udt:DateTimeString>"#,
        );
        assert!(matches!(
            read_document(&xml),
            Err(ImportError::Invalid {
                field: "udt:DateTimeString/@format",
                ..
            })
        ));
    }

    #[test]
    fn a_compact_date_that_is_not_a_date_is_reported() {
        let xml = written("SUP-6").replace(">20261027<", ">20261345<");
        assert!(matches!(
            read_document(&xml),
            Err(ImportError::Invalid {
                field: "udt:DateTimeString",
                ..
            })
        ));
    }

    #[test]
    fn a_missing_seller_is_reported() {
        let xml = written("SUP-7").replace("<ram:Name>Seller BV</ram:Name>", "");
        assert_eq!(
            read_document(&xml),
            Err(ImportError::Missing("ram:SellerTradeParty/ram:Name"))
        );
    }

    #[test]
    fn a_missing_line_total_is_reported() {
        let xml = written("SUP-8").replacen("<ram:LineTotalAmount>100.00</ram:LineTotalAmount>", "", 1);
        assert_eq!(read_document(&xml), Err(ImportError::Missing("ram:LineTotalAmount")));
    }

    #[test]
    fn a_dtd_in_cii_is_still_rejected() {
        let xml = "<?xml version=\"1.0\"?><!DOCTYPE foo [<!ENTITY x \"y\">]><rsm:CrossIndustryInvoice/>";
        assert_eq!(read_document(xml), Err(ImportError::Doctype));
    }
}
