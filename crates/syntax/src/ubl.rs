//! Deterministic UBL 2.1 writer (Peppol BIS Billing 3.0 profile).
//!
//! The writer is intentionally dumb: it serialises a validated view and never
//! decides business rules. Output is byte-stable for the same input (fixed
//! element order, no timestamps, no pretty-print variation), because artefacts
//! are hashed and archived.
//!
//! Credit notes share the writer: the root element, the type code element, the
//! line wrapper, and the quantity element differ, and BG-3 is emitted.

use crux::{Currency, DocType, InvoiceTotals, LocalDate, Money, Party, Quantity, TaxTreatment, UnitCode, VatCategory};

/// A document ready to be serialised.
pub struct UblInvoice<'a> {
    /// Human-readable invoice number (BT-1).
    pub number: &'a str,
    /// Issue date (BT-2).
    pub issue_date: LocalDate,
    /// Due date (BT-9).
    pub due_date: Option<LocalDate>,
    /// Actual delivery date (BT-72).
    pub delivery_date: Option<LocalDate>,
    /// Document type.
    pub doc_type: DocType,
    /// Document currency (BT-5).
    pub currency: Currency,
    /// Buyer reference (BT-10).
    pub buyer_reference: Option<&'a str>,
    /// Purchase order reference (BT-13).
    pub order_reference: Option<&'a str>,
    /// Free-text note (BT-22).
    pub note: Option<&'a str>,
    /// The invoice this credit note corrects (BG-3).
    pub preceding_invoice: Option<UblPrecedingInvoice<'a>>,
    /// Seller.
    pub seller: &'a Party,
    /// Customer.
    pub customer: &'a Party,
    /// Lines.
    pub lines: &'a [UblLine<'a>],
    /// Totals and VAT breakdown.
    pub totals: &'a InvoiceTotals,
}

/// The invoice a credit note corrects (BG-3).
pub struct UblPrecedingInvoice<'a> {
    /// Preceding invoice number (BT-25).
    pub number: &'a str,
    /// Preceding invoice issue date (BT-26).
    pub issue_date: LocalDate,
}

/// One invoice line.
pub struct UblLine<'a> {
    /// 1-based position.
    pub id: u32,
    /// Item name (BT-153).
    pub name: &'a str,
    /// Item description (BT-154).
    pub description: Option<&'a str>,
    /// Quantity (BT-129).
    pub quantity: Quantity,
    /// Unit (BT-130).
    pub unit: UnitCode,
    /// Unit price (BT-146).
    pub unit_price: Money,
    /// Price base quantity (BT-149).
    pub base_quantity: Quantity,
    /// Line net amount (BT-131), already rounded.
    pub line_net: Money,
    /// VAT treatment.
    pub treatment: &'a TaxTreatment,
}

/// Problems that prevent serialisation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UblError {
    /// A credit note must name the invoice it corrects (BG-3).
    CreditNoteWithoutPrecedingInvoice,
    /// A Peppol endpoint identifier must look like `EAS:value`.
    MalformedEndpoint,
}

impl std::fmt::Display for UblError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreditNoteWithoutPrecedingInvoice => f.write_str("a credit note needs a preceding invoice reference"),
            Self::MalformedEndpoint => f.write_str("a Peppol endpoint identifier must be written as EAS:value"),
        }
    }
}

impl std::error::Error for UblError {}

/// Serialises an invoice or credit note as UBL 2.1.
pub fn write_invoice(invoice: &UblInvoice<'_>) -> Result<String, UblError> {
    let credit_note = invoice.doc_type.is_credit_note();
    if credit_note && invoice.preceding_invoice.is_none() {
        return Err(UblError::CreditNoteWithoutPrecedingInvoice);
    }

    let mut out = String::with_capacity(4096);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(if credit_note {
        "<CreditNote xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2\""
    } else {
        "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\""
    });
    out.push_str(" xmlns:cac=\"urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2\"");
    out.push_str(" xmlns:cbc=\"urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2\">\n");

    text(
        &mut out,
        1,
        "cbc:CustomizationID",
        "urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0",
    );
    text(
        &mut out,
        1,
        "cbc:ProfileID",
        "urn:fdc:peppol.eu:2017:poacc:billing:01:1.0",
    );
    text(&mut out, 1, "cbc:ID", invoice.number);
    text(&mut out, 1, "cbc:IssueDate", &invoice.issue_date.to_iso_string());
    if let Some(due) = invoice.due_date
        && !credit_note
    {
        text(&mut out, 1, "cbc:DueDate", &due.to_iso_string());
    }
    text(
        &mut out,
        1,
        if credit_note {
            "cbc:CreditNoteTypeCode"
        } else {
            "cbc:InvoiceTypeCode"
        },
        invoice.doc_type.code(),
    );
    if let Some(note) = invoice.note {
        text(&mut out, 1, "cbc:Note", note);
    }
    text(&mut out, 1, "cbc:DocumentCurrencyCode", invoice.currency.as_str());
    if let Some(reference) = invoice.buyer_reference {
        text(&mut out, 1, "cbc:BuyerReference", reference);
    }
    if let Some(order) = invoice.order_reference {
        open(&mut out, 1, "cac:OrderReference");
        text(&mut out, 2, "cbc:ID", order);
        close(&mut out, 1, "cac:OrderReference");
    }
    if let Some(preceding) = &invoice.preceding_invoice {
        open(&mut out, 1, "cac:BillingReference");
        open(&mut out, 2, "cac:InvoiceDocumentReference");
        text(&mut out, 3, "cbc:ID", preceding.number);
        text(&mut out, 3, "cbc:IssueDate", &preceding.issue_date.to_iso_string());
        close(&mut out, 2, "cac:InvoiceDocumentReference");
        close(&mut out, 1, "cac:BillingReference");
    }

    party(&mut out, "cac:AccountingSupplierParty", invoice.seller, true)?;
    party(&mut out, "cac:AccountingCustomerParty", invoice.customer, false)?;

    let needs_delivery = invoice.delivery_date.is_some()
        || invoice
            .lines
            .iter()
            .any(|line| line.treatment.category == VatCategory::IntraCommunity);
    if needs_delivery {
        delivery(&mut out, invoice);
    }

    tax_total(&mut out, invoice);
    legal_total(&mut out, invoice);

    for line in invoice.lines {
        document_line(&mut out, invoice.currency, line, credit_note);
    }

    out.push_str(if credit_note { "</CreditNote>\n" } else { "</Invoice>\n" });
    Ok(out)
}

/// Splits a Peppol endpoint identifier into its EAS scheme and value.
///
/// An identifier without a non-empty scheme and value cannot be written: BT-34
/// and BT-49 require the scheme, and dropping the element silently would leave
/// the recipient unable to route the document.
fn party(out: &mut String, wrapper: &str, party: &Party, is_seller: bool) -> Result<(), UblError> {
    open(out, 1, wrapper);
    open(out, 2, "cac:Party");

    if party.peppol_id.is_some() {
        let Some((scheme, value)) = party.peppol_endpoint() else {
            return Err(UblError::MalformedEndpoint);
        };
        text_attrs(out, 3, "cbc:EndpointID", &[("schemeID", scheme)], value);
    }

    let name = party.trading_name.as_deref().unwrap_or(&party.legal_name);
    open(out, 3, "cac:PartyName");
    text(out, 4, "cbc:Name", name);
    close(out, 3, "cac:PartyName");

    open(out, 3, "cac:PostalAddress");
    if let Some(street) = &party.address.street {
        text(out, 4, "cbc:StreetName", street);
    }
    text(out, 4, "cbc:CityName", &party.address.city);
    if let Some(postal) = &party.address.postal_code {
        text(out, 4, "cbc:PostalZone", postal);
    }
    open(out, 4, "cac:Country");
    text(out, 5, "cbc:IdentificationCode", party.address.country.as_str());
    close(out, 4, "cac:Country");
    close(out, 3, "cac:PostalAddress");

    if let Some(vat_id) = &party.vat_id {
        open(out, 3, "cac:PartyTaxScheme");
        text(out, 4, "cbc:CompanyID", vat_id);
        open(out, 4, "cac:TaxScheme");
        text(out, 5, "cbc:ID", "VAT");
        close(out, 4, "cac:TaxScheme");
        close(out, 3, "cac:PartyTaxScheme");
    }

    open(out, 3, "cac:PartyLegalEntity");
    text(out, 4, "cbc:RegistrationName", &party.legal_name);
    if is_seller && let Some(kvk) = &party.kvk_number {
        // NL-R-003: a Dutch legal registration identifier carries its scheme.
        // 0106 is the EAS scheme for a Dutch chamber of commerce number.
        text_attrs(out, 4, "cbc:CompanyID", &[("schemeID", "0106")], kvk);
    }
    close(out, 3, "cac:PartyLegalEntity");

    if party.contact_name.is_some() || party.email.is_some() || party.phone.is_some() {
        open(out, 3, "cac:Contact");
        if let Some(contact) = &party.contact_name {
            text(out, 4, "cbc:Name", contact);
        }
        if let Some(phone) = &party.phone {
            text(out, 4, "cbc:Telephone", phone);
        }
        if let Some(email) = &party.email {
            text(out, 4, "cbc:ElectronicMail", email);
        }
        close(out, 3, "cac:Contact");
    }

    close(out, 2, "cac:Party");
    close(out, 1, wrapper);
    Ok(())
}

fn delivery(out: &mut String, invoice: &UblInvoice<'_>) {
    open(out, 1, "cac:Delivery");
    if let Some(date) = invoice.delivery_date {
        text(out, 2, "cbc:ActualDeliveryDate", &date.to_iso_string());
    }
    open(out, 2, "cac:DeliveryLocation");
    open(out, 3, "cac:Address");
    open(out, 4, "cac:Country");
    text(
        out,
        5,
        "cbc:IdentificationCode",
        invoice.customer.address.country.as_str(),
    );
    close(out, 4, "cac:Country");
    close(out, 3, "cac:Address");
    close(out, 2, "cac:DeliveryLocation");
    close(out, 1, "cac:Delivery");
}

fn tax_total(out: &mut String, invoice: &UblInvoice<'_>) {
    let currency = invoice.currency.as_str();
    open(out, 1, "cac:TaxTotal");
    amount(out, 2, "cbc:TaxAmount", currency, invoice.totals.tax);

    for row in &invoice.totals.breakdown {
        open(out, 2, "cac:TaxSubtotal");
        amount(out, 3, "cbc:TaxableAmount", currency, row.taxable_base);
        amount(out, 3, "cbc:TaxAmount", currency, row.tax);
        open(out, 3, "cac:TaxCategory");
        text(out, 4, "cbc:ID", row.category.code());
        if row.category != VatCategory::OutsideScope {
            text(out, 4, "cbc:Percent", &rate_basis_points(row.rate_bp));
        }
        if let Some((code, reason)) = exemption_reason(invoice, row) {
            if let Some(code) = code {
                text(out, 4, "cbc:TaxExemptionReasonCode", code);
            }
            if let Some(reason) = reason {
                text(out, 4, "cbc:TaxExemptionReason", reason);
            }
        }
        open(out, 4, "cac:TaxScheme");
        text(out, 5, "cbc:ID", "VAT");
        close(out, 4, "cac:TaxScheme");
        close(out, 3, "cac:TaxCategory");
        close(out, 2, "cac:TaxSubtotal");
    }

    close(out, 1, "cac:TaxTotal");
}

fn exemption_reason<'a>(
    invoice: &UblInvoice<'a>,
    row: &crux::TaxBreakdown,
) -> Option<(Option<&'a str>, Option<&'a str>)> {
    invoice
        .lines
        .iter()
        .find(|line| {
            line.treatment.category == row.category
                && (row.category != VatCategory::Standard || line.treatment.rate.basis_points() == row.rate_bp)
                && (line
                    .treatment
                    .exemption_reason_code
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    || line
                        .treatment
                        .exemption_reason
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()))
        })
        .map(|line| {
            let code = line
                .treatment
                .exemption_reason_code
                .as_deref()
                .filter(|value| !value.trim().is_empty());
            let reason = line
                .treatment
                .exemption_reason
                .as_deref()
                .filter(|value| !value.trim().is_empty());
            (code, reason)
        })
}

fn legal_total(out: &mut String, invoice: &UblInvoice<'_>) {
    let currency = invoice.currency.as_str();
    open(out, 1, "cac:LegalMonetaryTotal");
    amount(out, 2, "cbc:LineExtensionAmount", currency, invoice.totals.net);
    amount(out, 2, "cbc:TaxExclusiveAmount", currency, invoice.totals.net);
    amount(out, 2, "cbc:TaxInclusiveAmount", currency, invoice.totals.gross);
    amount(out, 2, "cbc:PayableAmount", currency, invoice.totals.gross);
    close(out, 1, "cac:LegalMonetaryTotal");
}

fn document_line(out: &mut String, currency: Currency, line: &UblLine<'_>, credit_note: bool) {
    let code = currency.as_str();
    let (wrapper, quantity) = if credit_note {
        ("cac:CreditNoteLine", "cbc:CreditedQuantity")
    } else {
        ("cac:InvoiceLine", "cbc:InvoicedQuantity")
    };
    open(out, 1, wrapper);
    text(out, 2, "cbc:ID", &line.id.to_string());
    text_attrs(
        out,
        2,
        quantity,
        &[("unitCode", line.unit.as_str())],
        &line.quantity.to_trimmed_string(),
    );
    amount(out, 2, "cbc:LineExtensionAmount", code, line.line_net);

    open(out, 2, "cac:Item");
    text(out, 3, "cbc:Name", line.name);
    if let Some(description) = line.description {
        text(out, 3, "cbc:Description", description);
    }
    open(out, 3, "cac:ClassifiedTaxCategory");
    text(out, 4, "cbc:ID", line.treatment.category.code());
    if line.treatment.category != VatCategory::OutsideScope {
        text(
            out,
            4,
            "cbc:Percent",
            &rate_basis_points(line.treatment.rate.basis_points()),
        );
    }
    open(out, 4, "cac:TaxScheme");
    text(out, 5, "cbc:ID", "VAT");
    close(out, 4, "cac:TaxScheme");
    close(out, 3, "cac:ClassifiedTaxCategory");
    close(out, 2, "cac:Item");

    open(out, 2, "cac:Price");
    text_attrs(
        out,
        3,
        "cbc:PriceAmount",
        &[("currencyID", code)],
        &line.unit_price.to_trimmed_string(),
    );
    text_attrs(
        out,
        3,
        "cbc:BaseQuantity",
        &[("unitCode", line.unit.as_str())],
        &line.base_quantity.to_trimmed_string(),
    );
    close(out, 2, "cac:Price");
    close(out, 1, wrapper);
}

fn rate_basis_points(basis_points: u16) -> String {
    let whole = basis_points / 100;
    let fraction = basis_points % 100;
    if fraction == 0 {
        whole.to_string()
    } else if fraction.is_multiple_of(10) {
        format!("{whole}.{}", fraction / 10)
    } else {
        format!("{whole}.{fraction:02}")
    }
}

fn open(out: &mut String, depth: usize, tag: &str) {
    indent(out, depth);
    out.push('<');
    out.push_str(tag);
    out.push_str(">\n");
}

fn close(out: &mut String, depth: usize, tag: &str) {
    indent(out, depth);
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

fn text(out: &mut String, depth: usize, tag: &str, value: &str) {
    indent(out, depth);
    out.push('<');
    out.push_str(tag);
    out.push('>');
    escape(out, value);
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

fn text_attrs(out: &mut String, depth: usize, tag: &str, attrs: &[(&str, &str)], value: &str) {
    indent(out, depth);
    out.push('<');
    out.push_str(tag);
    for (name, attribute) in attrs {
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        escape(out, attribute);
        out.push('"');
    }
    out.push('>');
    escape(out, value);
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

fn amount(out: &mut String, depth: usize, tag: &str, currency: &str, value: Money) {
    let rendered = value.to_fixed_string(2).unwrap_or_else(|_| "0.00".to_owned());
    text_attrs(out, depth, tag, &[("currencyID", currency)], &rendered);
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn escape(out: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(character),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crux::{Address, CountryCode, Money, Quantity, TaxBreakdown, TaxTreatment, VatCategory, VatRate};

    fn seller() -> Party {
        let mut party = Party::new(
            "Seller BV",
            Address {
                street: Some("Main 1".to_owned()),
                city: "Amsterdam".to_owned(),
                postal_code: Some("1000 AA".to_owned()),
                country: CountryCode::NL,
            },
        );
        party.vat_id = Some("NL123456782B01".to_owned());
        party.kvk_number = Some("12345678".to_owned());
        party.peppol_id = Some("0106:12345678".to_owned());
        party
    }

    fn customer() -> Party {
        Party::new("Customer BV", Address::new("Rotterdam", CountryCode::NL))
    }

    fn totals() -> InvoiceTotals {
        let net = Money::from_major_units(100, Currency::EUR).unwrap();
        let tax = Money::from_major_units(21, Currency::EUR).unwrap();
        InvoiceTotals {
            net,
            tax,
            gross: Money::from_major_units(121, Currency::EUR).unwrap(),
            breakdown: vec![TaxBreakdown {
                category: VatCategory::Standard,
                rate_bp: 2100,
                taxable_base: net,
                tax,
            }],
        }
    }

    fn minimal_invoice<'a>(
        seller: &'a Party,
        customer: &'a Party,
        lines: &'a [UblLine<'a>],
        totals: &'a InvoiceTotals,
    ) -> UblInvoice<'a> {
        UblInvoice {
            number: "2026-0001",
            issue_date: LocalDate::parse_iso("2026-10-27").unwrap(),
            due_date: Some(LocalDate::parse_iso("2026-11-26").unwrap()),
            delivery_date: None,
            doc_type: DocType::Invoice,
            currency: Currency::EUR,
            buyer_reference: Some("PO-1"),
            order_reference: Some("ORD-1"),
            note: None,
            preceding_invoice: None,
            seller,
            customer,
            lines,
            totals,
        }
    }

    #[test]
    fn writes_the_expected_document() {
        let treatment = TaxTreatment::standard(VatRate::STANDARD_NL);
        let line = UblLine {
            id: 1,
            name: "Consulting",
            description: Some("Advice"),
            quantity: Quantity::from_units(2).unwrap(),
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(50, Currency::EUR).unwrap(),
            base_quantity: Quantity::ONE,
            line_net: Money::from_major_units(100, Currency::EUR).unwrap(),
            treatment: &treatment,
        };
        let lines = [line];
        let totals = totals();
        let seller = seller();
        let customer = customer();
        let invoice = minimal_invoice(&seller, &customer, &lines, &totals);

        let xml = write_invoice(&invoice).unwrap();
        let expected = r#"<?xml version="1.0" encoding="UTF-8"?>
<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cac="urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2" xmlns:cbc="urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2">
  <cbc:CustomizationID>urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0</cbc:CustomizationID>
  <cbc:ProfileID>urn:fdc:peppol.eu:2017:poacc:billing:01:1.0</cbc:ProfileID>
  <cbc:ID>2026-0001</cbc:ID>
  <cbc:IssueDate>2026-10-27</cbc:IssueDate>
  <cbc:DueDate>2026-11-26</cbc:DueDate>
  <cbc:InvoiceTypeCode>380</cbc:InvoiceTypeCode>
  <cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>
  <cbc:BuyerReference>PO-1</cbc:BuyerReference>
  <cac:OrderReference>
    <cbc:ID>ORD-1</cbc:ID>
  </cac:OrderReference>
  <cac:AccountingSupplierParty>
    <cac:Party>
      <cbc:EndpointID schemeID="0106">12345678</cbc:EndpointID>
      <cac:PartyName>
        <cbc:Name>Seller BV</cbc:Name>
      </cac:PartyName>
      <cac:PostalAddress>
        <cbc:StreetName>Main 1</cbc:StreetName>
        <cbc:CityName>Amsterdam</cbc:CityName>
        <cbc:PostalZone>1000 AA</cbc:PostalZone>
        <cac:Country>
          <cbc:IdentificationCode>NL</cbc:IdentificationCode>
        </cac:Country>
      </cac:PostalAddress>
      <cac:PartyTaxScheme>
        <cbc:CompanyID>NL123456782B01</cbc:CompanyID>
        <cac:TaxScheme>
          <cbc:ID>VAT</cbc:ID>
        </cac:TaxScheme>
      </cac:PartyTaxScheme>
      <cac:PartyLegalEntity>
        <cbc:RegistrationName>Seller BV</cbc:RegistrationName>
        <cbc:CompanyID schemeID="0106">12345678</cbc:CompanyID>
      </cac:PartyLegalEntity>
    </cac:Party>
  </cac:AccountingSupplierParty>
  <cac:AccountingCustomerParty>
    <cac:Party>
      <cac:PartyName>
        <cbc:Name>Customer BV</cbc:Name>
      </cac:PartyName>
      <cac:PostalAddress>
        <cbc:CityName>Rotterdam</cbc:CityName>
        <cac:Country>
          <cbc:IdentificationCode>NL</cbc:IdentificationCode>
        </cac:Country>
      </cac:PostalAddress>
      <cac:PartyLegalEntity>
        <cbc:RegistrationName>Customer BV</cbc:RegistrationName>
      </cac:PartyLegalEntity>
    </cac:Party>
  </cac:AccountingCustomerParty>
  <cac:TaxTotal>
    <cbc:TaxAmount currencyID="EUR">21.00</cbc:TaxAmount>
    <cac:TaxSubtotal>
      <cbc:TaxableAmount currencyID="EUR">100.00</cbc:TaxableAmount>
      <cbc:TaxAmount currencyID="EUR">21.00</cbc:TaxAmount>
      <cac:TaxCategory>
        <cbc:ID>S</cbc:ID>
        <cbc:Percent>21</cbc:Percent>
        <cac:TaxScheme>
          <cbc:ID>VAT</cbc:ID>
        </cac:TaxScheme>
      </cac:TaxCategory>
    </cac:TaxSubtotal>
  </cac:TaxTotal>
  <cac:LegalMonetaryTotal>
    <cbc:LineExtensionAmount currencyID="EUR">100.00</cbc:LineExtensionAmount>
    <cbc:TaxExclusiveAmount currencyID="EUR">100.00</cbc:TaxExclusiveAmount>
    <cbc:TaxInclusiveAmount currencyID="EUR">121.00</cbc:TaxInclusiveAmount>
    <cbc:PayableAmount currencyID="EUR">121.00</cbc:PayableAmount>
  </cac:LegalMonetaryTotal>
  <cac:InvoiceLine>
    <cbc:ID>1</cbc:ID>
    <cbc:InvoicedQuantity unitCode="C62">2</cbc:InvoicedQuantity>
    <cbc:LineExtensionAmount currencyID="EUR">100.00</cbc:LineExtensionAmount>
    <cac:Item>
      <cbc:Name>Consulting</cbc:Name>
      <cbc:Description>Advice</cbc:Description>
      <cac:ClassifiedTaxCategory>
        <cbc:ID>S</cbc:ID>
        <cbc:Percent>21</cbc:Percent>
        <cac:TaxScheme>
          <cbc:ID>VAT</cbc:ID>
        </cac:TaxScheme>
      </cac:ClassifiedTaxCategory>
    </cac:Item>
    <cac:Price>
      <cbc:PriceAmount currencyID="EUR">50</cbc:PriceAmount>
      <cbc:BaseQuantity unitCode="C62">1</cbc:BaseQuantity>
    </cac:Price>
  </cac:InvoiceLine>
</Invoice>
"#;
        assert_eq!(xml, expected);
    }

    #[test]
    fn output_is_deterministic() {
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
        let lines = [line];
        let totals = totals();
        let seller = seller();
        let customer = customer();
        let invoice = minimal_invoice(&seller, &customer, &lines, &totals);
        assert_eq!(write_invoice(&invoice).unwrap(), write_invoice(&invoice).unwrap());
    }

    #[test]
    fn text_is_escaped() {
        let mut seller = seller();
        seller.legal_name = "A & B <C> \"D\" 'E'".to_owned();
        let treatment = TaxTreatment::standard(VatRate::STANDARD_NL);
        let line = UblLine {
            id: 1,
            name: "x",
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(1, Currency::EUR).unwrap(),
            base_quantity: Quantity::ONE,
            line_net: Money::from_major_units(1, Currency::EUR).unwrap(),
            treatment: &treatment,
        };
        let lines = [line];
        let totals = totals();
        let customer = customer();
        let invoice = minimal_invoice(&seller, &customer, &lines, &totals);
        let xml = write_invoice(&invoice).unwrap();
        assert!(xml.contains("A &amp; B &lt;C&gt; &quot;D&quot; &apos;E&apos;"), "{xml}");
        assert!(!xml.contains("<C>"));
    }

    fn zero_totals(category: VatCategory) -> InvoiceTotals {
        let net = Money::from_major_units(100, Currency::EUR).unwrap();
        InvoiceTotals {
            net,
            tax: Money::zero(Currency::EUR),
            gross: net,
            breakdown: vec![TaxBreakdown {
                category,
                rate_bp: 0,
                taxable_base: net,
                tax: Money::zero(Currency::EUR),
            }],
        }
    }

    #[test]
    fn exemption_reasons_are_written_at_the_tax_subtotal() {
        let treatment = TaxTreatment::zero(VatCategory::Exempt).with_exemption_reason("Exempt supply");
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
        let lines = [line];
        let totals = zero_totals(VatCategory::Exempt);
        let seller = seller();
        let customer = customer();
        let invoice = minimal_invoice(&seller, &customer, &lines, &totals);

        let xml = write_invoice(&invoice).unwrap();
        assert!(xml.contains("<cbc:ID>E</cbc:ID>"), "{xml}");
        assert!(
            xml.contains("<cbc:TaxExemptionReason>Exempt supply</cbc:TaxExemptionReason>"),
            "{xml}"
        );
    }

    #[test]
    fn exemption_reason_codes_are_written_and_outside_scope_omits_the_rate() {
        let mut treatment = TaxTreatment::zero(VatCategory::OutsideScope);
        treatment.exemption_reason_code = Some("VATEX-EU-O".to_owned());
        let line = UblLine {
            id: 1,
            name: "Outside scope",
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(100, Currency::EUR).unwrap(),
            base_quantity: Quantity::ONE,
            line_net: Money::from_major_units(100, Currency::EUR).unwrap(),
            treatment: &treatment,
        };
        let lines = [line];
        let totals = zero_totals(VatCategory::OutsideScope);
        let seller = seller();
        let customer = customer();
        let invoice = minimal_invoice(&seller, &customer, &lines, &totals);

        let xml = write_invoice(&invoice).unwrap();
        assert!(
            xml.contains("<cbc:TaxExemptionReasonCode>VATEX-EU-O</cbc:TaxExemptionReasonCode>"),
            "{xml}"
        );
        assert!(!xml.contains("<cbc:Percent>"), "{xml}");
    }

    #[test]
    fn intra_community_writes_delivery_date_and_country() {
        let treatment = TaxTreatment::zero(VatCategory::IntraCommunity).with_exemption_reason("Intra-community supply");
        let line = UblLine {
            id: 1,
            name: "Intra-community supply",
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(100, Currency::EUR).unwrap(),
            base_quantity: Quantity::ONE,
            line_net: Money::from_major_units(100, Currency::EUR).unwrap(),
            treatment: &treatment,
        };
        let lines = [line];
        let totals = zero_totals(VatCategory::IntraCommunity);
        let seller = seller();
        let customer = customer();
        let mut invoice = minimal_invoice(&seller, &customer, &lines, &totals);
        invoice.delivery_date = Some(LocalDate::parse_iso("2026-10-25").unwrap());

        let xml = write_invoice(&invoice).unwrap();
        assert!(xml.contains("<cac:Delivery>"), "{xml}");
        assert!(
            xml.contains("<cbc:ActualDeliveryDate>2026-10-25</cbc:ActualDeliveryDate>"),
            "{xml}"
        );
        assert!(
            xml.contains("<cbc:IdentificationCode>NL</cbc:IdentificationCode>"),
            "{xml}"
        );
    }

    #[test]
    fn a_dutch_legal_registration_carries_scheme_0106() {
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
        let lines = [line];
        let totals = totals();
        let seller = seller();
        let customer = customer();
        let invoice = minimal_invoice(&seller, &customer, &lines, &totals);

        // Peppol NL-R-003 requires scheme 0106 or 0190 on a Dutch seller's
        // legal registration identifier.
        let xml = write_invoice(&invoice).unwrap();
        assert!(
            xml.contains("<cbc:CompanyID schemeID=\"0106\">12345678</cbc:CompanyID>"),
            "{xml}"
        );
    }

    #[test]
    fn credit_notes_use_the_credit_note_binding() {
        let treatment = TaxTreatment::standard(VatRate::STANDARD_NL);
        let line = UblLine {
            id: 1,
            name: "Correction",
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(100, Currency::EUR).unwrap(),
            base_quantity: Quantity::ONE,
            line_net: Money::from_major_units(100, Currency::EUR).unwrap(),
            treatment: &treatment,
        };
        let lines = [line];
        let totals = totals();
        let seller = seller();
        let customer = customer();
        let mut invoice = minimal_invoice(&seller, &customer, &lines, &totals);
        invoice.doc_type = DocType::CreditNote;
        invoice.preceding_invoice = Some(UblPrecedingInvoice {
            number: "2026-0007",
            issue_date: LocalDate::parse_iso("2026-09-30").unwrap(),
        });

        let xml = write_invoice(&invoice).unwrap();
        assert!(xml.starts_with(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<CreditNote xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2\""
        ), "{xml}");
        assert!(
            xml.contains("<cbc:CreditNoteTypeCode>381</cbc:CreditNoteTypeCode>"),
            "{xml}"
        );
        assert!(!xml.contains("InvoiceTypeCode"), "{xml}");
        assert!(!xml.contains("<cbc:DueDate>"), "a credit note has no due date: {xml}");
        assert!(xml.contains("<cac:BillingReference>"), "{xml}");
        assert!(xml.contains("<cbc:ID>2026-0007</cbc:ID>"), "{xml}");
        assert!(xml.contains("<cbc:IssueDate>2026-09-30</cbc:IssueDate>"), "{xml}");
        assert!(xml.contains("<cac:CreditNoteLine>"), "{xml}");
        assert!(
            xml.contains("<cbc:CreditedQuantity unitCode=\"C62\">1</cbc:CreditedQuantity>"),
            "{xml}"
        );
        assert!(!xml.contains("cac:InvoiceLine"), "{xml}");
        assert!(xml.ends_with("</CreditNote>\n"), "{xml}");
    }

    #[test]
    fn a_malformed_endpoint_identifier_is_rejected() {
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
        let lines = [line];
        let totals = totals();
        let seller = seller();
        let mut customer = customer();
        customer.peppol_id = Some("12345678".to_owned());
        let invoice = minimal_invoice(&seller, &customer, &lines, &totals);

        assert_eq!(write_invoice(&invoice), Err(UblError::MalformedEndpoint));
    }

    #[test]
    fn a_credit_note_without_a_preceding_invoice_is_rejected() {
        let treatment = TaxTreatment::standard(VatRate::STANDARD_NL);
        let line = UblLine {
            id: 1,
            name: "x",
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(1, Currency::EUR).unwrap(),
            base_quantity: Quantity::ONE,
            line_net: Money::from_major_units(1, Currency::EUR).unwrap(),
            treatment: &treatment,
        };
        let lines = [line];
        let totals = totals();
        let seller = seller();
        let customer = customer();
        let mut invoice = minimal_invoice(&seller, &customer, &lines, &totals);
        invoice.doc_type = DocType::CreditNote;

        assert_eq!(
            write_invoice(&invoice),
            Err(UblError::CreditNoteWithoutPrecedingInvoice)
        );
    }
}
