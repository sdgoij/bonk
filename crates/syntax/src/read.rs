//! UBL 2.1 reader: a received invoice becomes the terms we model.
//!
//! The reader is deliberately narrow. It extracts what the domain and the
//! payable register need, keeps the document's own amounts as sent, and refuses
//! anything it cannot represent rather than guessing. It never decides business
//! rules; `conformance` does that on the resulting view.
//!
//! Safety: documents come from third parties. DTDs are rejected outright (so no
//! entity expansion), nesting depth and size are bounded, and a document is only
//! accepted when its root element is in the UBL Invoice or CreditNote namespace.

use crux::{
    Address, CountryCode, Currency, DocType, InvoiceTotals, LocalDate, Money, Party, Quantity, TaxBreakdown,
    TaxTreatment, UnitCode, VatCategory, VatRate,
};
use quick_xml::Reader;
use quick_xml::events::Event;

/// Largest document the reader accepts.
pub const MAX_DOCUMENT_BYTES: usize = 10 * 1024 * 1024;

/// Deepest element nesting the reader accepts.
pub const MAX_DEPTH: usize = 64;

const INVOICE_NAMESPACE: &str = "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2";
const CREDIT_NOTE_NAMESPACE: &str = "urn:oasis:names:specification:ubl:schema:xsd:CreditNote-2";

/// The invoice a received credit note corrects (BG-3), as the sender named it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImportedReference {
    /// Preceding invoice number (BT-25).
    pub number: String,
    /// Preceding invoice issue date (BT-26).
    pub issue_date: Option<LocalDate>,
}

/// One line of a received document.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImportedLine {
    /// 1-based position, from `cbc:ID` when present.
    pub position: u32,
    /// Item name (BT-153).
    pub name: String,
    /// Item description (BT-154).
    pub description: Option<String>,
    /// Quantity (BT-129), as sent.
    pub quantity: Quantity,
    /// Unit of measure (BT-130).
    pub unit: UnitCode,
    /// Item net price (BT-146).
    pub unit_price: Money,
    /// Price base quantity (BT-149).
    pub base_quantity: Quantity,
    /// Line net amount (BT-131), as sent.
    pub line_net: Money,
    /// VAT treatment (BT-151, BT-152, BT-120, BT-121).
    pub treatment: TaxTreatment,
}

/// A received invoice or credit note.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImportedInvoice {
    /// Invoice or credit note (BT-3).
    pub doc_type: DocType,
    /// Document number as the sender wrote it (BT-1).
    pub number: String,
    /// Issue date (BT-2).
    pub issue_date: LocalDate,
    /// Due date (BT-9).
    pub due_date: Option<LocalDate>,
    /// Actual delivery date (BT-72).
    pub delivery_date: Option<LocalDate>,
    /// Document currency (BT-5).
    pub currency: Currency,
    /// Buyer reference (BT-10).
    pub buyer_reference: Option<String>,
    /// Order reference (BT-13).
    pub order_reference: Option<String>,
    /// Free-text note (BT-22).
    pub note: Option<String>,
    /// Specification identifier the sender claims (BT-24).
    pub profile: Option<String>,
    /// The invoice a credit note corrects.
    pub preceding: Option<ImportedReference>,
    /// Issuer of the document (BT-27 and friends).
    pub seller: Party,
    /// Recipient (BT-44 and friends).
    pub customer: Party,
    /// Lines (BG-25).
    pub lines: Vec<ImportedLine>,
    /// Totals as sent (BG-22, BG-23).
    pub totals: InvoiceTotals,
}

/// Why a document was not accepted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ImportError {
    /// The document exceeds [`MAX_DOCUMENT_BYTES`].
    TooLarge {
        /// Size that was offered.
        bytes: usize,
    },
    /// The bytes are not UTF-8 XML, or the XML is malformed.
    NotXml(String),
    /// The document carries a DTD.
    Doctype,
    /// Element nesting exceeds [`MAX_DEPTH`].
    TooDeep,
    /// The root element is not UBL 2.1 Invoice or CreditNote.
    NotUbl(String),
    /// A required element is absent or empty.
    Missing(&'static str),
    /// A required element has a value we cannot represent.
    Invalid {
        /// Element that failed, e.g. `cbc:IssueDate`.
        field: &'static str,
        /// The value as sent.
        value: String,
    },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(f, "the document is {bytes} bytes; the limit is {MAX_DOCUMENT_BYTES}"),
            Self::NotXml(detail) => write!(f, "the document is not well-formed XML: {detail}"),
            Self::Doctype => f.write_str("documents with a DTD are not accepted"),
            Self::TooDeep => write!(f, "the document nests deeper than {MAX_DEPTH} elements"),
            Self::NotUbl(root) => write!(
                f,
                "{root:?} is not a UBL 2.1 Invoice or CreditNote (unexpected root or namespace)"
            ),
            Self::Missing(field) => write!(f, "{field} is required but missing or empty"),
            Self::Invalid { field, value } => write!(f, "{field} has an unsupported value {value:?}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Reads a UBL 2.1 invoice or credit note.
pub fn read_invoice(xml: &str) -> Result<ImportedInvoice, ImportError> {
    if xml.len() > MAX_DOCUMENT_BYTES {
        return Err(ImportError::TooLarge { bytes: xml.len() });
    }
    let root = parse(xml)?;
    from_root(&root)
}

fn from_root(root: &Node) -> Result<ImportedInvoice, ImportError> {
    let doc_type = match (root.name.as_str(), namespace(root).as_deref()) {
        ("Invoice", Some(INVOICE_NAMESPACE)) | ("CreditNote", Some(CREDIT_NOTE_NAMESPACE)) => {
            if root.name == "Invoice" {
                DocType::Invoice
            } else {
                DocType::CreditNote
            }
        }
        (name, namespace) => {
            return Err(ImportError::NotUbl(match namespace {
                Some(namespace) => format!("{name} in {namespace}"),
                None => name.to_owned(),
            }));
        }
    };

    let currency = Currency::parse(&required(root, "DocumentCurrencyCode")?)
        .map_err(|error| invalid("cbc:DocumentCurrencyCode", error.to_string()))?;
    let number = text(root, "ID").ok_or(ImportError::Missing("cbc:ID"))?;
    let issue_date = date(root, "IssueDate")?.ok_or(ImportError::Missing("cbc:IssueDate"))?;

    let seller =
        party(root, "AccountingSupplierParty").ok_or(ImportError::Missing("cac:AccountingSupplierParty/cac:Party"))?;
    let customer =
        party(root, "AccountingCustomerParty").ok_or(ImportError::Missing("cac:AccountingCustomerParty/cac:Party"))?;

    let mut lines = Vec::new();
    for (index, element) in root
        .children("InvoiceLine")
        .chain(root.children("CreditNoteLine"))
        .enumerate()
    {
        lines.push(line(element, currency, index)?);
    }

    Ok(ImportedInvoice {
        doc_type,
        number,
        issue_date,
        due_date: date(root, "DueDate")?,
        delivery_date: match root.first("Delivery") {
            Some(delivery) => date(delivery, "ActualDeliveryDate")?,
            None => None,
        },
        currency,
        buyer_reference: text(root, "BuyerReference"),
        order_reference: root.first("OrderReference").and_then(|order| text(order, "ID")),
        note: text(root, "Note"),
        profile: text(root, "CustomizationID"),
        preceding: preceding(root)?,
        seller,
        customer,
        lines,
        totals: totals(root, currency)?,
    })
}

fn line(element: &Node, currency: Currency, index: usize) -> Result<ImportedLine, ImportError> {
    let position = text(element, "ID")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_else(|| u32::try_from(index + 1).unwrap_or(u32::MAX));

    let quantity_element = element
        .first("InvoicedQuantity")
        .or_else(|| element.first("CreditedQuantity"))
        .ok_or(ImportError::Missing("cbc:InvoicedQuantity"))?;
    let quantity =
        Quantity::parse(&quantity_element.text).map_err(|error| invalid("cbc:InvoicedQuantity", error.to_string()))?;
    let unit = UnitCode::parse(
        &quantity_element
            .attribute("unitCode")
            .unwrap_or_else(|| "C62".to_owned()),
    )
    .map_err(|error| invalid("cbc:InvoicedQuantity/@unitCode", error.to_string()))?;

    let price = element.first("Price");
    let unit_price = money(
        price.and_then(|price| price.first("PriceAmount")),
        currency,
        "cbc:PriceAmount",
    )?
    .ok_or(ImportError::Missing("cac:Price/cbc:PriceAmount"))?;
    let base_quantity = price
        .and_then(|price| price.first("BaseQuantity"))
        .map(|node| Quantity::parse(&node.text))
        .transpose()
        .map_err(|error| invalid("cac:Price/cbc:BaseQuantity", error.to_string()))?
        .unwrap_or(Quantity::ONE);

    let item = element.first("Item");
    let name = item
        .and_then(|item| text(item, "Name"))
        .or_else(|| item.and_then(|item| item.first("Description").map(|node| node.text.clone())))
        .ok_or(ImportError::Missing("cac:Item/cbc:Name"))?;
    let description = item.and_then(|item| text(item, "Description"));

    let category = item.and_then(|item| item.first("ClassifiedTaxCategory"));
    let vat_category = category
        .and_then(|category| text(category, "ID"))
        .map(|code| VatCategory::parse(&code))
        .transpose()
        .map_err(|error| invalid("cac:ClassifiedTaxCategory/cbc:ID", error.to_string()))?
        .unwrap_or(VatCategory::Standard);
    let rate = category
        .and_then(|category| text(category, "Percent"))
        .map(|percent| VatRate::parse_percent(&percent))
        .transpose()
        .map_err(|error| invalid("cac:ClassifiedTaxCategory/cbc:Percent", error.to_string()))?
        .unwrap_or(VatRate::ZERO);

    let line_net = money(
        element.first("LineExtensionAmount"),
        currency,
        "cbc:LineExtensionAmount",
    )?
    .ok_or(ImportError::Missing("cbc:LineExtensionAmount"))?;

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
            exemption_reason: category.and_then(|category| text(category, "TaxExemptionReason")),
            exemption_reason_code: category.and_then(|category| text(category, "TaxExemptionReasonCode")),
        },
    })
}

fn totals(root: &Node, currency: Currency) -> Result<InvoiceTotals, ImportError> {
    let legal = root.first("LegalMonetaryTotal");
    let net = money(
        legal.and_then(|node| node.first("TaxExclusiveAmount")),
        currency,
        "BT-109",
    )?
    .ok_or(ImportError::Missing("cac:LegalMonetaryTotal/cbc:TaxExclusiveAmount"))?;
    let gross = money(
        legal.and_then(|node| node.first("TaxInclusiveAmount")),
        currency,
        "BT-112",
    )?
    .ok_or(ImportError::Missing("cac:LegalMonetaryTotal/cbc:TaxInclusiveAmount"))?;

    let tax_total = root.first("TaxTotal");
    let tax = money(tax_total.and_then(|node| node.first("TaxAmount")), currency, "BT-110")?
        .ok_or(ImportError::Missing("cac:TaxTotal/cbc:TaxAmount"))?;

    let mut breakdown = Vec::new();
    if let Some(tax_total) = tax_total {
        for row in tax_total.children("TaxSubtotal") {
            let category = row.first("TaxCategory");
            let vat_category = category
                .and_then(|category| text(category, "ID"))
                .map(|code| VatCategory::parse(&code))
                .transpose()
                .map_err(|error| invalid("cac:TaxCategory/cbc:ID", error.to_string()))?
                .unwrap_or(VatCategory::Standard);
            let rate_bp = category
                .and_then(|category| text(category, "Percent"))
                .map(|percent| VatRate::parse_percent(&percent))
                .transpose()
                .map_err(|error| invalid("cac:TaxCategory/cbc:Percent", error.to_string()))?
                .unwrap_or(VatRate::ZERO)
                .basis_points();
            breakdown.push(TaxBreakdown {
                category: vat_category,
                rate_bp,
                taxable_base: money(row.first("TaxableAmount"), currency, "BT-116")?
                    .ok_or(ImportError::Missing("cac:TaxSubtotal/cbc:TaxableAmount"))?,
                tax: money(row.first("TaxAmount"), currency, "BT-117")?
                    .ok_or(ImportError::Missing("cac:TaxSubtotal/cbc:TaxAmount"))?,
            });
        }
    }

    Ok(InvoiceTotals {
        net,
        tax,
        gross,
        breakdown,
    })
}

fn preceding(root: &Node) -> Result<Option<ImportedReference>, ImportError> {
    let Some(reference) = root
        .first("BillingReference")
        .and_then(|billing| billing.first("InvoiceDocumentReference"))
    else {
        return Ok(None);
    };
    let Some(number) = text(reference, "ID") else {
        return Ok(None);
    };
    Ok(Some(ImportedReference {
        number,
        issue_date: date(reference, "IssueDate")?,
    }))
}

fn party(root: &Node, wrapper: &str) -> Option<Party> {
    let party = root.first(wrapper)?.first("Party")?;
    let name = party
        .first("PartyName")
        .and_then(|name| text(name, "Name"))
        .or_else(|| {
            party
                .first("PartyLegalEntity")
                .and_then(|legal| text(legal, "RegistrationName"))
        })?;

    let postal = party.first("PostalAddress");
    let country = postal
        .and_then(|postal| postal.first("Country"))
        .and_then(|country| text(country, "IdentificationCode"))
        .and_then(|code| CountryCode::parse(&code).ok())?;

    let mut parsed = Party::new(
        name,
        Address {
            street: postal.and_then(|postal| text(postal, "StreetName")),
            city: postal.and_then(|postal| text(postal, "CityName")).unwrap_or_default(),
            postal_code: postal.and_then(|postal| text(postal, "PostalZone")),
            country,
        },
    );
    parsed.vat_id = party
        .children("PartyTaxScheme")
        .find(|scheme| {
            scheme
                .first("TaxScheme")
                .and_then(|scheme| text(scheme, "ID"))
                .is_some_and(|id| id.eq_ignore_ascii_case("VAT"))
        })
        .and_then(|scheme| text(scheme, "CompanyID"));
    parsed.kvk_number = party
        .first("PartyLegalEntity")
        .and_then(|legal| legal.first("CompanyID"))
        .filter(|company| company.attribute("schemeID").is_none_or(|scheme| scheme == "0106"))
        .map(|company| company.text.clone());
    parsed.peppol_id = party.first("EndpointID").map(|endpoint| {
        let value = endpoint.text.clone();
        match endpoint.attribute("schemeID") {
            Some(scheme) if !scheme.is_empty() => format!("{scheme}:{value}"),
            _ => value,
        }
    });
    Some(parsed)
}

fn required(node: &Node, name: &str) -> Result<String, ImportError> {
    text(node, name).ok_or(ImportError::Missing("cbc:DocumentCurrencyCode"))
}

fn text(node: &Node, name: &str) -> Option<String> {
    node.first(name)
        .map(|child| child.text.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn date(node: &Node, name: &str) -> Result<Option<LocalDate>, ImportError> {
    text(node, name)
        .map(|value| {
            LocalDate::parse_iso(&value).map_err(|error| ImportError::Invalid {
                field: "cbc:Date",
                value: format!("{value} ({error})"),
            })
        })
        .transpose()
}

fn money(node: Option<&Node>, currency: Currency, field: &'static str) -> Result<Option<Money>, ImportError> {
    let Some(node) = node else {
        return Ok(None);
    };
    let value = node.text.trim();
    Money::parse_fixed(value, currency)
        .map(Some)
        .map_err(|error| ImportError::Invalid {
            field,
            value: format!("{value} ({error})"),
        })
}

fn invalid(field: &'static str, detail: String) -> ImportError {
    ImportError::Invalid { field, value: detail }
}

// ---------------------------------------------------------------- minimal DOM

struct Node {
    name: String,
    raw_name: String,
    attributes: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

impl Node {
    fn first(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |child| child.name == name)
    }

    fn attribute(&self, name: &str) -> Option<String> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    }
}

fn namespace(root: &Node) -> Option<String> {
    match root.raw_name.split_once(':') {
        Some((prefix, _)) => root.attribute(&format!("xmlns:{prefix}")),
        None => root.attribute("xmlns"),
    }
}

/// Parses well-formed XML into a bounded tree of local names.
fn parse(xml: &str) -> Result<Node, ImportError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut stack: Vec<Node> = Vec::new();
    let mut root: Option<Node> = None;

    loop {
        let event = reader
            .read_event()
            .map_err(|error| ImportError::NotXml(error.to_string()))?;
        match event {
            Event::Start(element) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(ImportError::TooDeep);
                }
                let raw_name = element.name().as_ref().to_owned();
                stack.push(Node {
                    name: local(element.name().as_ref()),
                    raw_name,
                    attributes: attributes(&element),
                    children: Vec::new(),
                    text: String::new(),
                });
            }
            Event::Empty(element) => {
                let raw_name = element.name().as_ref().to_owned();
                let node = Node {
                    name: local(element.name().as_ref()),
                    raw_name,
                    attributes: attributes(&element),
                    children: Vec::new(),
                    text: String::new(),
                };
                attach(&mut stack, &mut root, node)?;
            }
            Event::End(_) => {
                let node = stack
                    .pop()
                    .ok_or_else(|| ImportError::NotXml("unbalanced element".to_owned()))?;
                attach(&mut stack, &mut root, node)?;
            }
            Event::Text(text) => {
                if let Some(current) = stack.last_mut() {
                    let value = text.into_inner().into_owned();
                    push_text(&mut current.text, &value);
                }
            }
            Event::CData(data) => {
                if let Some(current) = stack.last_mut() {
                    let value: &str = data.as_ref();
                    push_text(&mut current.text, value);
                }
            }
            Event::DocType(_) => return Err(ImportError::Doctype),
            Event::Eof => break,
            _ => {}
        }
    }

    if !stack.is_empty() {
        return Err(ImportError::NotXml("unclosed elements".to_owned()));
    }
    root.ok_or_else(|| ImportError::NotXml("empty document".to_owned()))
}

fn attach(stack: &mut [Node], root: &mut Option<Node>, node: Node) -> Result<(), ImportError> {
    match stack.last_mut() {
        Some(parent) => {
            parent.children.push(node);
            Ok(())
        }
        None if root.is_none() => {
            *root = Some(node);
            Ok(())
        }
        None => Err(ImportError::NotXml("more than one root element".to_owned())),
    }
}

fn push_text(target: &mut String, value: &str) {
    if !target.is_empty() {
        target.push(' ');
    }
    target.push_str(value);
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or(name).to_owned()
}

fn attributes(element: &quick_xml::events::BytesStart<'_>) -> Vec<(String, String)> {
    element
        .attributes()
        .flatten()
        .map(|attribute| {
            // Attribute names keep their prefix: namespace declarations are
            // looked up as `xmlns` / `xmlns:prefix`.
            (attribute.key.as_ref().to_owned(), attribute.value.into_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crux::{TaxBreakdown, VatRate};

    use super::*;
    use crate::ubl::{UblInvoice, UblLine, UblPrecedingInvoice, write_invoice};

    fn endpoint_party(name: &str, endpoint: &str) -> Party {
        let mut party = Party::new(
            name,
            Address {
                street: Some("Main 1".to_owned()),
                city: "Amsterdam".to_owned(),
                postal_code: Some("1000 AA".to_owned()),
                country: CountryCode::NL,
            },
        );
        party.vat_id = Some("NL123456782B01".to_owned());
        party.kvk_number = Some("12345678".to_owned());
        party.peppol_id = Some(endpoint.to_owned());
        party
    }

    fn treatment() -> TaxTreatment {
        TaxTreatment::standard(VatRate::STANDARD_NL)
    }

    fn totals() -> InvoiceTotals {
        let net = Money::from_major_units(100, Currency::EUR).expect("net");
        let tax = Money::from_major_units(21, Currency::EUR).expect("tax");
        InvoiceTotals {
            net,
            tax,
            gross: Money::from_major_units(121, Currency::EUR).expect("gross"),
            breakdown: vec![TaxBreakdown {
                category: VatCategory::Standard,
                rate_bp: 2100,
                taxable_base: net,
                tax,
            }],
        }
    }

    fn written(doc_type: DocType) -> String {
        let treatment = treatment();
        let line = UblLine {
            id: 1,
            name: "Consulting",
            description: Some("Advice"),
            quantity: Quantity::from_units(2).expect("quantity"),
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(50, Currency::EUR).expect("price"),
            base_quantity: Quantity::ONE,
            line_net: Money::from_major_units(100, Currency::EUR).expect("line net"),
            treatment: &treatment,
        };
        let lines = [line];
        let totals = totals();
        let seller = endpoint_party("Seller BV", "0106:12345678");
        let customer = endpoint_party("Customer BV", "0106:87654321");
        let invoice = UblInvoice {
            number: if doc_type == DocType::Invoice {
                "2026-0001"
            } else {
                "2026-C0001"
            },
            issue_date: LocalDate::parse_iso("2026-10-27").expect("date"),
            due_date: Some(LocalDate::parse_iso("2026-11-26").expect("due date")),
            delivery_date: None,
            doc_type,
            currency: Currency::EUR,
            buyer_reference: Some("PO-1"),
            order_reference: None,
            note: None,
            preceding_invoice: (doc_type == DocType::CreditNote).then_some(UblPrecedingInvoice {
                number: "2026-0007",
                issue_date: LocalDate::parse_iso("2026-09-30").expect("preceding date"),
            }),
            seller: &seller,
            customer: &customer,
            lines: &lines,
            totals: &totals,
        };
        write_invoice(&invoice).expect("writer accepts its own view")
    }

    #[test]
    fn round_trips_a_written_invoice() {
        let xml = written(DocType::Invoice);
        let imported = read_invoice(&xml).expect("import");

        assert_eq!(imported.doc_type, DocType::Invoice);
        assert_eq!(imported.number, "2026-0001");
        assert_eq!(imported.issue_date, LocalDate::parse_iso("2026-10-27").unwrap());
        assert_eq!(imported.due_date, Some(LocalDate::parse_iso("2026-11-26").unwrap()));
        assert_eq!(imported.currency, Currency::EUR);
        assert_eq!(imported.buyer_reference.as_deref(), Some("PO-1"));
        assert!(imported.preceding.is_none());

        assert_eq!(imported.seller.legal_name, "Seller BV");
        assert_eq!(imported.seller.vat_id.as_deref(), Some("NL123456782B01"));
        assert_eq!(imported.seller.kvk_number.as_deref(), Some("12345678"));
        assert_eq!(imported.seller.peppol_id.as_deref(), Some("0106:12345678"));
        assert_eq!(imported.seller.address.country, CountryCode::NL);
        assert_eq!(imported.seller.address.street.as_deref(), Some("Main 1"));

        assert_eq!(imported.lines.len(), 1);
        let line = &imported.lines[0];
        assert_eq!(line.position, 1);
        assert_eq!(line.name, "Consulting");
        assert_eq!(line.quantity, Quantity::from_units(2).unwrap());
        assert_eq!(line.unit, UnitCode::PIECE);
        assert_eq!(line.unit_price.micros(), 50_000_000);
        assert_eq!(line.line_net.micros(), 100_000_000);
        assert_eq!(line.treatment.rate, VatRate::STANDARD_NL);

        assert_eq!(imported.totals.net.micros(), 100_000_000);
        assert_eq!(imported.totals.tax.micros(), 21_000_000);
        assert_eq!(imported.totals.gross.micros(), 121_000_000);
        assert_eq!(imported.totals.breakdown.len(), 1);
        assert_eq!(imported.totals.breakdown[0].rate_bp, 2100);
    }

    #[test]
    fn round_trips_a_written_credit_note() {
        let imported = read_invoice(&written(DocType::CreditNote)).expect("import");
        assert_eq!(imported.doc_type, DocType::CreditNote);
        assert_eq!(imported.number, "2026-C0001");
        let preceding = imported.preceding.expect("BG-3");
        assert_eq!(preceding.number, "2026-0007");
        assert_eq!(preceding.issue_date, Some(LocalDate::parse_iso("2026-09-30").unwrap()));
    }

    #[test]
    fn a_dtd_is_rejected() {
        let xml = "<?xml version=\"1.0\"?><!DOCTYPE foo [<!ENTITY x \"y\">]><Invoice/>";
        assert_eq!(read_invoice(xml), Err(ImportError::Doctype));
    }

    #[test]
    fn a_non_ubl_root_is_rejected() {
        let xml = r#"<Invoice xmlns="urn:example:not-ubl"><cbc:ID>x</cbc:ID></Invoice>"#;
        assert!(matches!(read_invoice(xml), Err(ImportError::NotUbl(_))));
    }

    #[test]
    fn a_missing_number_is_reported() {
        let xml = written(DocType::Invoice).replace("<cbc:ID>2026-0001</cbc:ID>", "");
        assert_eq!(read_invoice(&xml), Err(ImportError::Missing("cbc:ID")));
    }

    #[test]
    fn a_malformed_date_is_reported() {
        let xml = written(DocType::Invoice).replace("2026-10-27", "27-10-2026");
        assert!(matches!(
            read_invoice(&xml),
            Err(ImportError::Invalid { field: "cbc:Date", .. })
        ));
    }

    #[test]
    fn too_deep_nesting_is_rejected() {
        let mut xml = String::from("<Invoice>");
        for _ in 0..(MAX_DEPTH + 1) {
            xml.push_str("<a>");
        }
        assert_eq!(read_invoice(&xml), Err(ImportError::TooDeep));
    }

    #[test]
    fn an_oversized_document_is_rejected() {
        let xml = "x".repeat(MAX_DOCUMENT_BYTES + 1);
        assert!(matches!(read_invoice(&xml), Err(ImportError::TooLarge { .. })));
    }

    #[test]
    fn a_document_with_a_prefixed_root_uses_its_own_namespace() {
        let xml = written(DocType::Invoice).replace(
            "<Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"",
            "<ubl:Invoice xmlns:ubl=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\"",
        );
        let xml = xml.replace("</Invoice>", "</ubl:Invoice>");
        assert!(read_invoice(&xml).is_ok(), "a prefixed root is still UBL");
    }
}
