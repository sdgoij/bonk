//! CII D16B: reading a received CrossIndustryInvoice, and writing one.
//!
//! Factur-X and ZUGFeRD carry EN 16931 in CII, and some access points send CII
//! directly. The business terms are the same as in UBL — the BT/BG numbers do
//! not change — so this reader maps element names onto the same
//! [`ImportedInvoice`] the UBL reader produces, and the rest of the pipeline
//! never learns which syntax arrived.
//!
//! The writer is the other direction and produces the payload a Factur-X PDF
//! carries. Like the UBL writer it is a straight-line serialiser with a fixed
//! element order, so the same invoice hashes the same.

use crux::{
    Address, CountryCode, Currency, DocType, FacturXProfile, InvoiceTotals, LocalDate, Money, Party, Quantity,
    TaxBreakdown, TaxTreatment, UnitCode, VatCategory, VatRate,
};

use crate::invoice::{Invoice, InvoiceLine, exemption_reason};
use crate::xml;

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

    let (line_extension_total, totals) = totals(settlement, currency)?;

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
        line_extension_total,
        totals,
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
    // EN 16931 puts the line total inside the line's monetary summation; a
    // hand-written document may put it directly under the settlement, so both are
    // read.
    let line_total = settlement.first("LineTotalAmount").or_else(|| {
        settlement
            .first("SpecifiedTradeSettlementLineMonetarySummation")
            .and_then(|summation| summation.first("LineTotalAmount"))
    });
    let line_net =
        money(line_total, currency, "ram:LineTotalAmount")?.ok_or(ImportError::Missing("ram:LineTotalAmount"))?;

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

fn totals(settlement: &Node, currency: Currency) -> Result<(Money, InvoiceTotals), ImportError> {
    let summation = settlement
        .first("SpecifiedTradeSettlementHeaderMonetarySummation")
        .ok_or(ImportError::Missing(
            "ram:SpecifiedTradeSettlementHeaderMonetarySummation",
        ))?;
    // BT-106, kept apart from BT-109 (`TaxBasisTotalAmount`): a document that
    // does not agree with itself can state them differently.
    let line_total = money(summation.first("LineTotalAmount"), currency, "ram:LineTotalAmount")?
        .ok_or(ImportError::Missing("ram:LineTotalAmount"))?;
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

    Ok((
        line_total,
        InvoiceTotals {
            net,
            tax,
            gross,
            breakdown,
        },
    ))
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
    if let Some(contact) = party.first("DefinedTradeContact") {
        parsed.contact_name = text(contact, "PersonName");
        parsed.phone = contact
            .first("TelephoneUniversalCommunication")
            .and_then(|phone| text(phone, "CompleteNumber"));
        parsed.email = contact
            .first("EmailURIUniversalCommunication")
            .and_then(|email| text(email, "URIID"));
    }
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

/// The EN 16931 guideline identifier written into BT-24 (Factur-X's profile).
pub const EN16931_GUIDELINE: &str = "urn:cen.eu:en16931:2017";

/// Why a document cannot be serialised as CII.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CiiError {
    /// A credit note must name the invoice it corrects (BG-3).
    CreditNoteWithoutPrecedingInvoice,
    /// A Peppol endpoint identifier must look like `EAS:value`.
    MalformedEndpoint,
    /// The level cannot be written without a field the invoice left empty.
    ///
    /// A lower level carries fewer elements, so it depends on the few it keeps:
    /// a MINIMUM invoice is the parties, the amounts, and the dates, and nothing
    /// else can stand in for a missing name.
    MissingField {
        /// The level that was being written.
        profile: FacturXProfile,
        /// The business term, e.g. `BT-27 the seller's name`.
        field: &'static str,
    },
}

impl std::fmt::Display for CiiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreditNoteWithoutPrecedingInvoice => f.write_str("a credit note needs a preceding invoice reference"),
            Self::MalformedEndpoint => f.write_str("a Peppol endpoint identifier must be written as EAS:value"),
            Self::MissingField { profile, field } => {
                write!(f, "a Factur-X {profile} document needs {field}")
            }
        }
    }
}

impl std::error::Error for CiiError {}

/// Serialises an invoice or credit note as CII D16B at the default level.
///
/// The default is the level the archive validates against, and the one every
/// hybrid written before the levels existed claims, so its bytes do not move.
pub fn write_invoice(invoice: &Invoice<'_>) -> Result<String, CiiError> {
    write_invoice_at(invoice, FacturXProfile::DEFAULT)
}

/// Serialises an invoice or credit note as CII D16B at a Factur-X level.
///
/// The level decides what is written, not what is checked: a document is
/// validated against its own profile before it gets here. What this guards is the
/// minimum each level needs to still be that level.
pub fn write_invoice_at(invoice: &Invoice<'_>, profile: FacturXProfile) -> Result<String, CiiError> {
    let credit_note = invoice.doc_type.is_credit_note();
    if credit_note && invoice.preceding_invoice.is_none() {
        return Err(CiiError::CreditNoteWithoutPrecedingInvoice);
    }
    require(invoice, profile)?;

    let mut out = String::with_capacity(8192);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<rsm:CrossIndustryInvoice");
    out.push_str(" xmlns:rsm=\"urn:un:unece:uncefact:data:standard:CrossIndustryInvoice:100\"");
    out.push_str(" xmlns:ram=\"urn:un:unece:uncefact:data:standard:ReusableAggregateBusinessInformationEntity:100\"");
    out.push_str(" xmlns:udt=\"urn:un:unece:uncefact:data:standard:UnqualifiedDataType:100\">\n");

    xml::open(&mut out, 1, "rsm:ExchangedDocumentContext");
    xml::open(&mut out, 2, "ram:GuidelineSpecifiedDocumentContextParameter");
    xml::text(&mut out, 3, "ram:ID", profile.guideline());
    xml::close(&mut out, 2, "ram:GuidelineSpecifiedDocumentContextParameter");
    xml::close(&mut out, 1, "rsm:ExchangedDocumentContext");

    xml::open(&mut out, 1, "rsm:ExchangedDocument");
    xml::text(&mut out, 2, "ram:ID", invoice.number);
    xml::text(&mut out, 2, "ram:TypeCode", invoice.doc_type.code());
    xml::open(&mut out, 2, "ram:IssueDateTime");
    cii_date(&mut out, 3, invoice.issue_date);
    xml::close(&mut out, 2, "ram:IssueDateTime");
    if profile.carries_note()
        && let Some(note) = invoice.note
    {
        xml::open(&mut out, 2, "ram:IncludedNote");
        xml::text(&mut out, 3, "ram:Content", note);
        xml::close(&mut out, 2, "ram:IncludedNote");
    }
    xml::close(&mut out, 1, "rsm:ExchangedDocument");

    xml::open(&mut out, 1, "rsm:SupplyChainTradeTransaction");
    if profile.carries_lines() {
        for line in invoice.lines {
            line_item(&mut out, line);
        }
    }
    agreement(&mut out, invoice, profile)?;
    delivery(&mut out, invoice, profile);
    settlement(&mut out, invoice, profile);
    xml::close(&mut out, 1, "rsm:SupplyChainTradeTransaction");

    out.push_str("</rsm:CrossIndustryInvoice>\n");
    Ok(out)
}

/// The fields a level cannot do without.
///
/// Everything else in the model can be absent from a valid document; these are
/// the ones whose absence would make the file unreadable as the level it claims.
fn require(invoice: &Invoice<'_>, profile: FacturXProfile) -> Result<(), CiiError> {
    let required = [
        ("BT-1 the invoice number", invoice.number),
        ("BT-27 the seller's name", invoice.seller.legal_name.as_str()),
        ("BT-44 the buyer's name", invoice.customer.legal_name.as_str()),
    ];
    for (field, value) in required {
        if value.trim().is_empty() {
            return Err(CiiError::MissingField { profile, field });
        }
    }
    if profile.requires_seller_vat() && invoice.seller.vat_id.as_deref().is_none_or(|vat| vat.trim().is_empty()) {
        return Err(CiiError::MissingField {
            profile,
            field: "BT-31 the seller's VAT identifier",
        });
    }
    Ok(())
}

/// One line, in the element order the CII schema declares.
fn line_item(out: &mut String, line: &InvoiceLine<'_>) {
    xml::open(out, 2, "ram:IncludedSupplyChainTradeLineItem");
    xml::open(out, 3, "ram:AssociatedDocumentLineDocument");
    xml::text(out, 4, "ram:LineID", &line.id.to_string());
    xml::close(out, 3, "ram:AssociatedDocumentLineDocument");

    xml::open(out, 3, "ram:SpecifiedTradeProduct");
    xml::text(out, 4, "ram:Name", line.name);
    if let Some(description) = line.description {
        xml::text(out, 4, "ram:Description", description);
    }
    xml::close(out, 3, "ram:SpecifiedTradeProduct");

    xml::open(out, 3, "ram:SpecifiedLineTradeAgreement");
    xml::open(out, 4, "ram:NetPriceProductTradePrice");
    xml::money(out, 5, "ram:ChargeAmount", line.unit_price);
    xml::text_attrs(
        out,
        5,
        "ram:BasisQuantity",
        &[("unitCode", line.unit.as_str())],
        &line.base_quantity.to_trimmed_string(),
    );
    xml::close(out, 4, "ram:NetPriceProductTradePrice");
    xml::close(out, 3, "ram:SpecifiedLineTradeAgreement");

    xml::open(out, 3, "ram:SpecifiedLineTradeDelivery");
    xml::text_attrs(
        out,
        4,
        "ram:BilledQuantity",
        &[("unitCode", line.unit.as_str())],
        &line.quantity.to_trimmed_string(),
    );
    xml::close(out, 3, "ram:SpecifiedLineTradeDelivery");

    xml::open(out, 3, "ram:SpecifiedLineTradeSettlement");
    xml::open(out, 4, "ram:ApplicableTradeTax");
    xml::text(out, 5, "ram:TypeCode", "VAT");
    xml::text(out, 5, "ram:CategoryCode", line.treatment.category.code());
    if line.treatment.category != VatCategory::OutsideScope {
        xml::text(
            out,
            5,
            "ram:RateApplicablePercent",
            &xml::rate_basis_points(line.treatment.rate.basis_points()),
        );
    }
    xml::close(out, 4, "ram:ApplicableTradeTax");
    xml::open(out, 4, "ram:SpecifiedTradeSettlementLineMonetarySummation");
    xml::money(out, 5, "ram:LineTotalAmount", line.line_net);
    xml::close(out, 4, "ram:SpecifiedTradeSettlementLineMonetarySummation");
    xml::close(out, 3, "ram:SpecifiedLineTradeSettlement");
    xml::close(out, 2, "ram:IncludedSupplyChainTradeLineItem");
}

fn agreement(out: &mut String, invoice: &Invoice<'_>, profile: FacturXProfile) -> Result<(), CiiError> {
    xml::open(out, 2, "ram:ApplicableHeaderTradeAgreement");
    if profile.carries_references()
        && let Some(reference) = invoice.buyer_reference
    {
        xml::text(out, 3, "ram:BuyerReference", reference);
    }
    trade_party(out, "ram:SellerTradeParty", invoice.seller, true, profile)?;
    trade_party(out, "ram:BuyerTradeParty", invoice.customer, false, profile)?;
    if profile.carries_references()
        && let Some(order) = invoice.order_reference
    {
        xml::open(out, 3, "ram:SellerOrderReferencedDocument");
        xml::text(out, 4, "ram:IssuerAssignedID", order);
        xml::close(out, 3, "ram:SellerOrderReferencedDocument");
    }
    xml::close(out, 2, "ram:ApplicableHeaderTradeAgreement");
    Ok(())
}

/// A party, in the element order `TradePartyType` declares.
///
/// `ram:Name` is the registered name (BT-27/BT-44), because that is what the
/// schema means by it; a trading name is a separate term (BT-28/BT-45) and lives
/// inside the legal organization.
fn trade_party(
    out: &mut String,
    wrapper: &str,
    party: &Party,
    is_seller: bool,
    profile: FacturXProfile,
) -> Result<(), CiiError> {
    let endpoint = match &party.peppol_id {
        Some(_) if profile.carries_endpoint() => Some(party.peppol_endpoint().ok_or(CiiError::MalformedEndpoint)?),
        Some(_) => None,
        None => None,
    };

    xml::open(out, 3, wrapper);
    // BT-29: the seller's identifier, which for a Dutch business is its KvK number.
    if is_seller
        && profile.carries_references()
        && let Some(kvk) = &party.kvk_number
    {
        xml::text_attrs(out, 4, "ram:ID", &[("schemeID", "0106")], kvk);
    }
    xml::text(out, 4, "ram:Name", &party.legal_name);

    if profile.carries_references() && ((is_seller && party.kvk_number.is_some()) || party.trading_name.is_some()) {
        xml::open(out, 4, "ram:SpecifiedLegalOrganization");
        if is_seller && let Some(kvk) = &party.kvk_number {
            xml::text_attrs(out, 5, "ram:ID", &[("schemeID", "0106")], kvk);
        }
        if let Some(trading) = &party.trading_name {
            xml::text(out, 5, "ram:TradingBusinessName", trading);
        }
        xml::close(out, 4, "ram:SpecifiedLegalOrganization");
    }

    if profile.carries_contact() && (party.contact_name.is_some() || party.phone.is_some() || party.email.is_some()) {
        xml::open(out, 4, "ram:DefinedTradeContact");
        if let Some(contact) = &party.contact_name {
            xml::text(out, 5, "ram:PersonName", contact);
        }
        if let Some(phone) = &party.phone {
            xml::open(out, 5, "ram:TelephoneUniversalCommunication");
            xml::text(out, 6, "ram:CompleteNumber", phone);
            xml::close(out, 5, "ram:TelephoneUniversalCommunication");
        }
        if let Some(email) = &party.email {
            xml::open(out, 5, "ram:EmailURIUniversalCommunication");
            xml::text(out, 6, "ram:URIID", email);
            xml::close(out, 5, "ram:EmailURIUniversalCommunication");
        }
        xml::close(out, 4, "ram:DefinedTradeContact");
    }

    // MINIMUM keeps the country and drops the rest: a system that only posts the
    // amount needs to know where the VAT came from, not where the letter goes.
    xml::open(out, 4, "ram:PostalTradeAddress");
    if profile.carries_address() {
        if let Some(postal) = &party.address.postal_code {
            xml::text(out, 5, "ram:PostcodeCode", postal);
        }
        if let Some(street) = &party.address.street {
            xml::text(out, 5, "ram:LineOne", street);
        }
        xml::text(out, 5, "ram:CityName", &party.address.city);
    }
    xml::text(out, 5, "ram:CountryID", party.address.country.as_str());
    xml::close(out, 4, "ram:PostalTradeAddress");

    if let Some((scheme, value)) = endpoint {
        xml::open(out, 4, "ram:URIUniversalCommunication");
        xml::text_attrs(out, 5, "ram:URIID", &[("schemeID", scheme)], value);
        xml::close(out, 4, "ram:URIUniversalCommunication");
    }
    if let Some(vat_id) = &party.vat_id {
        xml::open(out, 4, "ram:SpecifiedTaxRegistration");
        xml::text_attrs(out, 5, "ram:ID", &[("schemeID", "VA")], vat_id);
        xml::close(out, 4, "ram:SpecifiedTaxRegistration");
    }

    xml::close(out, 3, wrapper);
    Ok(())
}

fn delivery(out: &mut String, invoice: &Invoice<'_>, profile: FacturXProfile) {
    xml::open(out, 2, "ram:ApplicableHeaderTradeDelivery");
    // The element itself is mandatory in D16B at every level; only the date
    // inside it belongs to BASIC and above.
    if profile.carries_delivery()
        && let Some(delivered) = invoice.delivery_date
    {
        xml::open(out, 3, "ram:ActualDeliverySupplyChainEvent");
        xml::open(out, 4, "ram:OccurrenceDateTime");
        cii_date(out, 5, delivered);
        xml::close(out, 4, "ram:OccurrenceDateTime");
        xml::close(out, 3, "ram:ActualDeliverySupplyChainEvent");
    }
    xml::close(out, 2, "ram:ApplicableHeaderTradeDelivery");
}

fn settlement(out: &mut String, invoice: &Invoice<'_>, profile: FacturXProfile) {
    let currency = invoice.currency.as_str();
    xml::open(out, 2, "ram:ApplicableHeaderTradeSettlement");
    xml::text(out, 3, "ram:InvoiceCurrencyCode", currency);

    if profile.carries_vat_breakdown() {
        for row in &invoice.totals.breakdown {
            let exemption = exemption_reason(invoice, row);
            xml::open(out, 3, "ram:ApplicableTradeTax");
            xml::money(out, 4, "ram:CalculatedAmount", row.tax);
            xml::text(out, 4, "ram:TypeCode", "VAT");
            if let Some((_, reason)) = exemption
                && let Some(reason) = reason
            {
                xml::text(out, 4, "ram:ExemptionReason", reason);
            }
            xml::money(out, 4, "ram:BasisAmount", row.taxable_base);
            xml::text(out, 4, "ram:CategoryCode", row.category.code());
            if let Some((code, _)) = exemption
                && let Some(code) = code
            {
                xml::text(out, 4, "ram:ExemptionReasonCode", code);
            }
            if row.category != VatCategory::OutsideScope {
                xml::text(
                    out,
                    4,
                    "ram:RateApplicablePercent",
                    &xml::rate_basis_points(row.rate_bp),
                );
            }
            xml::close(out, 3, "ram:ApplicableTradeTax");
        }
    }

    if profile.carries_payment_terms()
        && let Some(due) = invoice.due_date
    {
        xml::open(out, 3, "ram:SpecifiedTradePaymentTerms");
        xml::open(out, 4, "ram:DueDateDateTime");
        cii_date(out, 5, due);
        xml::close(out, 4, "ram:DueDateDateTime");
        xml::close(out, 3, "ram:SpecifiedTradePaymentTerms");
    }

    xml::open(out, 3, "ram:SpecifiedTradeSettlementHeaderMonetarySummation");
    xml::money(out, 4, "ram:LineTotalAmount", invoice.totals.net);
    xml::money(out, 4, "ram:TaxBasisTotalAmount", invoice.totals.net);
    xml::amount(out, 4, "ram:TaxTotalAmount", currency, invoice.totals.tax);
    xml::money(out, 4, "ram:GrandTotalAmount", invoice.totals.gross);
    xml::money(out, 4, "ram:DuePayableAmount", invoice.totals.gross);
    xml::close(out, 3, "ram:SpecifiedTradeSettlementHeaderMonetarySummation");

    // BG-3 comes after the summation, which is where the schema puts it.
    if profile.carries_references()
        && let Some(preceding) = &invoice.preceding_invoice
    {
        xml::open(out, 3, "ram:InvoiceReferencedDocument");
        xml::text(out, 4, "ram:IssuerAssignedID", preceding.number);
        xml::open(out, 4, "ram:FormattedIssueDateTime");
        cii_date(out, 5, preceding.issue_date);
        xml::close(out, 4, "ram:FormattedIssueDateTime");
        xml::close(out, 3, "ram:InvoiceReferencedDocument");
    }

    xml::close(out, 2, "ram:ApplicableHeaderTradeSettlement");
}

/// A CII date: `CCYYMMDD` under `format="102"`.
fn cii_date(out: &mut String, depth: usize, value: LocalDate) {
    let compact: String = value
        .to_iso_string()
        .chars()
        .filter(|character| *character != '-')
        .collect();
    xml::text_attrs(out, depth, "udt:DateTimeString", &[("format", "102")], &compact);
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

    /// The view the writer tests serialise: one line, one VAT row, every optional
    /// field the writer can carry filled in.
    struct Parts {
        seller: Party,
        customer: Party,
        treatment: TaxTreatment,
        totals: InvoiceTotals,
    }

    impl Parts {
        fn new() -> Self {
            let mut seller = Party::new(
                "Seller BV",
                Address {
                    street: Some("Main 1".to_owned()),
                    city: "Amsterdam".to_owned(),
                    postal_code: Some("1000 AA".to_owned()),
                    country: CountryCode::NL,
                },
            );
            seller.vat_id = Some("NL123456782B01".to_owned());
            seller.kvk_number = Some("12345678".to_owned());
            seller.peppol_id = Some("0106:12345678".to_owned());
            let customer = Party::new(
                "Customer BV",
                Address {
                    street: Some("Buyer 1".to_owned()),
                    city: "Rotterdam".to_owned(),
                    postal_code: Some("3000 AA".to_owned()),
                    country: CountryCode::NL,
                },
            );
            Self {
                seller,
                customer,
                treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
                totals: InvoiceTotals {
                    net: Money::from_major_units(100, Currency::EUR).unwrap(),
                    tax: Money::from_major_units(21, Currency::EUR).unwrap(),
                    gross: Money::from_major_units(121, Currency::EUR).unwrap(),
                    breakdown: vec![TaxBreakdown {
                        category: VatCategory::Standard,
                        rate_bp: 2100,
                        taxable_base: Money::from_major_units(100, Currency::EUR).unwrap(),
                        tax: Money::from_major_units(21, Currency::EUR).unwrap(),
                    }],
                },
            }
        }

        fn write(&self, profile: FacturXProfile) -> Result<String, CiiError> {
            let line = InvoiceLine {
                id: 1,
                name: "Consulting",
                description: Some("Advice"),
                quantity: Quantity::from_units(2).unwrap(),
                unit: UnitCode::PIECE,
                unit_price: Money::from_major_units(50, Currency::EUR).unwrap(),
                base_quantity: Quantity::ONE,
                line_net: Money::from_major_units(100, Currency::EUR).unwrap(),
                treatment: &self.treatment,
            };
            let lines = [line];
            let invoice = Invoice {
                number: "2026-0001",
                issue_date: LocalDate::parse_iso("2026-10-27").unwrap(),
                due_date: Some(LocalDate::parse_iso("2026-11-26").unwrap()),
                delivery_date: Some(LocalDate::parse_iso("2026-10-25").unwrap()),
                doc_type: DocType::Invoice,
                currency: Currency::EUR,
                buyer_reference: Some("PO-1"),
                order_reference: Some("ORDER-7"),
                note: Some("Thank you"),
                preceding_invoice: None,
                seller: &self.seller,
                customer: &self.customer,
                lines: &lines,
                totals: &self.totals,
            };
            write_invoice_at(&invoice, profile)
        }

        fn xml(&self, profile: FacturXProfile) -> String {
            self.write(profile).expect("the writer accepts its own view")
        }
    }

    fn round_trip() -> (String, ImportedInvoice) {
        let parts = Parts::new();
        let xml = parts.xml(FacturXProfile::DEFAULT);
        let imported = read_document(&xml).expect("the writer's output reads back").invoice;
        (xml, imported)
    }

    #[test]
    fn every_level_writes_its_own_guideline() {
        let parts = Parts::new();
        for profile in FacturXProfile::ALL {
            let xml = parts.xml(profile);
            assert!(
                xml.contains(&format!("<ram:ID>{}</ram:ID>", profile.guideline())),
                "{profile} does not name itself: {xml}"
            );
        }
    }

    #[test]
    fn the_lower_levels_leave_out_what_they_do_not_carry() {
        let parts = Parts::new();

        let minimum = parts.xml(FacturXProfile::Minimum);
        assert!(!minimum.contains("IncludedSupplyChainTradeLineItem"), "{minimum}");
        assert!(!minimum.contains("ram:ApplicableTradeTax"), "{minimum}");
        assert!(!minimum.contains("ram:CityName"), "{minimum}");
        assert!(!minimum.contains("ram:SpecifiedTradePaymentTerms"), "{minimum}");
        assert!(!minimum.contains("ram:BuyerReference"), "{minimum}");
        // What it keeps is what a posting system needs: who, how much, when.
        assert!(minimum.contains("<ram:CountryID>NL</ram:CountryID>"), "{minimum}");
        assert!(
            minimum.contains("<ram:GrandTotalAmount>121.00</ram:GrandTotalAmount>"),
            "{minimum}"
        );
        assert!(
            minimum.contains(r#"<ram:TaxTotalAmount currencyID="EUR">21.00</ram:TaxTotalAmount>"#),
            "{minimum}"
        );

        let basic_wl = parts.xml(FacturXProfile::BasicWl);
        assert!(!basic_wl.contains("IncludedSupplyChainTradeLineItem"), "{basic_wl}");
        assert!(
            basic_wl.contains("<ram:BasisAmount>100.00</ram:BasisAmount>"),
            "{basic_wl}"
        );
        assert!(
            basic_wl.contains("<ram:CityName>Amsterdam</ram:CityName>"),
            "{basic_wl}"
        );
        assert!(basic_wl.contains("<ram:SpecifiedTradePaymentTerms>"), "{basic_wl}");

        let basic = parts.xml(FacturXProfile::Basic);
        assert!(basic.contains("<ram:IncludedSupplyChainTradeLineItem>"), "{basic}");
        assert!(basic.contains("<ram:ActualDeliverySupplyChainEvent>"), "{basic}");
    }

    #[test]
    fn a_minimum_document_reads_back_as_its_amounts() {
        let parts = Parts::new();
        let xml = parts.xml(FacturXProfile::Minimum);
        let read = read_document(&xml)
            .expect("a minimum document is still a document")
            .invoice;
        assert_eq!(read.number, "2026-0001");
        assert!(read.lines.is_empty(), "the level carries no lines");
        assert_eq!(read.totals.net.micros(), 100_000_000);
        assert_eq!(read.totals.tax.micros(), 21_000_000);
        assert_eq!(read.totals.gross.micros(), 121_000_000);
        assert!(read.totals.breakdown.is_empty(), "no breakdown below BASIC WL");
        assert_eq!(read.seller.legal_name, "Seller BV");
        assert_eq!(read.seller.address.country, CountryCode::NL);
        assert_eq!(read.profile.as_deref(), Some(FacturXProfile::Minimum.guideline()));
    }

    #[test]
    fn a_level_that_needs_the_seller_vat_refuses_without_it() {
        let mut parts = Parts::new();
        parts.seller.vat_id = None;
        assert_eq!(
            parts.write(FacturXProfile::Minimum),
            Err(CiiError::MissingField {
                profile: FacturXProfile::Minimum,
                field: "BT-31 the seller's VAT identifier",
            })
        );
        // The higher levels keep the lines and the breakdown, which identify the
        // document without a tax registration.
        assert!(parts.write(FacturXProfile::En16931).is_ok());
    }

    #[test]
    fn a_blank_party_name_is_refused_at_every_level() {
        let mut parts = Parts::new();
        parts.customer.legal_name = "  ".to_owned();
        for profile in FacturXProfile::ALL {
            assert_eq!(
                parts.write(profile),
                Err(CiiError::MissingField {
                    profile,
                    field: "BT-44 the buyer's name",
                }),
                "{profile}"
            );
        }
    }

    #[test]
    fn what_the_writer_emits_is_what_the_reader_reads() {
        let (_, invoice) = round_trip();

        assert_eq!(invoice.doc_type, DocType::Invoice);
        assert_eq!(invoice.number, "2026-0001");
        assert_eq!(invoice.issue_date, LocalDate::parse_iso("2026-10-27").unwrap());
        assert_eq!(invoice.due_date, Some(LocalDate::parse_iso("2026-11-26").unwrap()));
        assert_eq!(invoice.delivery_date, Some(LocalDate::parse_iso("2026-10-25").unwrap()));
        assert_eq!(invoice.currency, Currency::EUR);
        assert_eq!(invoice.buyer_reference.as_deref(), Some("PO-1"));
        assert_eq!(invoice.order_reference.as_deref(), Some("ORDER-7"));
        assert_eq!(invoice.note.as_deref(), Some("Thank you"));
        assert_eq!(invoice.profile.as_deref(), Some(EN16931_GUIDELINE));

        let seller = &invoice.seller;
        assert_eq!(seller.legal_name, "Seller BV");
        assert_eq!(seller.vat_id.as_deref(), Some("NL123456782B01"));
        assert_eq!(seller.kvk_number.as_deref(), Some("12345678"));
        assert_eq!(seller.peppol_id.as_deref(), Some("0106:12345678"));
        assert_eq!(seller.address.street.as_deref(), Some("Main 1"));
        assert_eq!(seller.address.city, "Amsterdam");
        assert_eq!(seller.address.postal_code.as_deref(), Some("1000 AA"));
        assert_eq!(seller.address.country, CountryCode::NL);
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
    }

    #[test]
    fn the_writer_is_byte_stable() {
        let (first, _) = round_trip();
        let (second, _) = round_trip();
        assert_eq!(first, second, "the same invoice must serialise to the same bytes");
    }

    #[test]
    fn the_document_has_the_shape_en_16931_declares() {
        let (xml, _) = round_trip();
        assert!(xml.contains("<ram:ID>urn:cen.eu:en16931:2017</ram:ID>"), "{xml}");
        assert!(
            xml.contains("<ram:SpecifiedTradeSettlementLineMonetarySummation>"),
            "the line total sits in the summation the schema declares: {xml}"
        );
        assert!(
            xml.contains(r#"<ram:BilledQuantity unitCode="C62">2</ram:BilledQuantity>"#),
            "{xml}"
        );
        assert!(
            xml.contains(r#"<ram:TaxTotalAmount currencyID="EUR">21.00</ram:TaxTotalAmount>"#),
            "{xml}"
        );
        assert!(
            xml.contains(r#"<ram:ID schemeID="0106">12345678</ram:ID>"#),
            "the KvK carries its scheme: {xml}"
        );
    }

    #[test]
    fn a_credit_note_needs_and_carries_its_preceding_invoice() {
        // The writer refuses BG-3 before it writes anything.
        let seller = Party::new("Seller BV", Address::new("Amsterdam", CountryCode::NL));
        let customer = Party::new("Customer BV", Address::new("Rotterdam", CountryCode::NL));
        let treatment = TaxTreatment::standard(VatRate::STANDARD_NL);
        let line = InvoiceLine {
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
        let totals = InvoiceTotals {
            net: Money::from_major_units(100, Currency::EUR).unwrap(),
            tax: Money::from_major_units(21, Currency::EUR).unwrap(),
            gross: Money::from_major_units(121, Currency::EUR).unwrap(),
            breakdown: vec![TaxBreakdown {
                category: VatCategory::Standard,
                rate_bp: 2100,
                taxable_base: Money::from_major_units(100, Currency::EUR).unwrap(),
                tax: Money::from_major_units(21, Currency::EUR).unwrap(),
            }],
        };
        let mut invoice = Invoice {
            number: "2026-0002",
            issue_date: LocalDate::parse_iso("2026-10-27").unwrap(),
            due_date: None,
            delivery_date: None,
            doc_type: DocType::CreditNote,
            currency: Currency::EUR,
            buyer_reference: None,
            order_reference: None,
            note: None,
            preceding_invoice: None,
            seller: &seller,
            customer: &customer,
            lines: &lines,
            totals: &totals,
        };
        assert_eq!(
            write_invoice(&invoice),
            Err(CiiError::CreditNoteWithoutPrecedingInvoice)
        );

        invoice.preceding_invoice = Some(crate::InvoiceReference {
            number: "2026-0001",
            issue_date: LocalDate::parse_iso("2026-09-30").unwrap(),
        });
        let xml = write_invoice(&invoice).unwrap();
        let read = read_document(&xml).expect("the credit note reads back").invoice;
        assert_eq!(read.doc_type, DocType::CreditNote);
        let preceding = read.preceding.expect("BG-3");
        assert_eq!(preceding.number, "2026-0001");
        assert_eq!(preceding.issue_date, Some(LocalDate::parse_iso("2026-09-30").unwrap()));
    }

    #[test]
    fn an_exemption_reason_is_written_on_the_breakdown_row() {
        let seller = Party::new("Seller BV", Address::new("Amsterdam", CountryCode::NL));
        let customer = Party::new("Customer BV", Address::new("Rotterdam", CountryCode::NL));
        let treatment = TaxTreatment::zero(VatCategory::Exempt).with_exemption_reason("Exempt supply");
        let line = InvoiceLine {
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
        let totals = InvoiceTotals {
            net: Money::from_major_units(100, Currency::EUR).unwrap(),
            tax: Money::zero(Currency::EUR),
            gross: Money::from_major_units(100, Currency::EUR).unwrap(),
            breakdown: vec![TaxBreakdown {
                category: VatCategory::Exempt,
                rate_bp: 0,
                taxable_base: Money::from_major_units(100, Currency::EUR).unwrap(),
                tax: Money::zero(Currency::EUR),
            }],
        };
        let invoice = Invoice {
            number: "2026-0003",
            issue_date: LocalDate::parse_iso("2026-10-27").unwrap(),
            due_date: None,
            delivery_date: None,
            doc_type: DocType::Invoice,
            currency: Currency::EUR,
            buyer_reference: None,
            order_reference: None,
            note: None,
            preceding_invoice: None,
            seller: &seller,
            customer: &customer,
            lines: &lines,
            totals: &totals,
        };

        let xml = write_invoice(&invoice).unwrap();
        assert!(
            xml.contains("<ram:ExemptionReason>Exempt supply</ram:ExemptionReason>"),
            "{xml}"
        );
        assert!(xml.contains("<ram:CategoryCode>E</ram:CategoryCode>"), "{xml}");
        let read = read_document(&xml).expect("the exempt invoice reads back").invoice;
        assert_eq!(read.totals.breakdown[0].category, VatCategory::Exempt);
        assert_eq!(read.totals.tax.micros(), 0);
    }

    #[test]
    fn an_endpoint_without_a_scheme_is_refused() {
        let (_, _) = round_trip();
        let mut seller = Party::new("Seller BV", Address::new("Amsterdam", CountryCode::NL));
        seller.peppol_id = Some("12345678".to_owned());
        let customer = Party::new("Customer BV", Address::new("Rotterdam", CountryCode::NL));
        let treatment = TaxTreatment::standard(VatRate::STANDARD_NL);
        let line = InvoiceLine {
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
        let totals = InvoiceTotals {
            net: Money::from_major_units(100, Currency::EUR).unwrap(),
            tax: Money::from_major_units(21, Currency::EUR).unwrap(),
            gross: Money::from_major_units(121, Currency::EUR).unwrap(),
            breakdown: vec![TaxBreakdown {
                category: VatCategory::Standard,
                rate_bp: 2100,
                taxable_base: Money::from_major_units(100, Currency::EUR).unwrap(),
                tax: Money::from_major_units(21, Currency::EUR).unwrap(),
            }],
        };
        let invoice = Invoice {
            number: "2026-0004",
            issue_date: LocalDate::parse_iso("2026-10-27").unwrap(),
            due_date: None,
            delivery_date: None,
            doc_type: DocType::Invoice,
            currency: Currency::EUR,
            buyer_reference: None,
            order_reference: None,
            note: None,
            preceding_invoice: None,
            seller: &seller,
            customer: &customer,
            lines: &lines,
            totals: &totals,
        };
        assert_eq!(write_invoice(&invoice), Err(CiiError::MalformedEndpoint));
    }
}
