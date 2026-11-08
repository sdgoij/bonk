//! Plain data crossed to a front end.
//!
//! These are deliberately not the domain types: no lifetimes, no database
//! records, and no rendering types. The rules for anything added here are:
//!
//! - money is integer micros plus an ISO currency (or a pre-formatted string
//!   when the front end only displays it);
//! - dates are ISO strings;
//! - identifiers are strings;
//! - findings and outcomes are data, not errors, so a UI can show blocking
//!   findings without parsing a message.
//!
//! The FFI crate mirrors these shapes in language-neutral records, which keeps
//! `uniffi` out of this crate.

use crux::{
    CountryCode, Currency, CustomerId, DocType, DocumentId, DomainError, DraftLine, Finding, InvoiceDraft,
    InvoiceTotals, LineId, LocalDate, Money, Party, PhoneNumber, PrecedingInvoice, Quantity, SellerId, Severity,
    TaxTreatment, UnitCode, VatCategory, VatRate,
};
use store::{CustomerRecord, DocumentRecord, DocumentStatus, SearchHit, SellerRecord, archive::DocumentView};

/// What the dashboard shows for an open archive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Summary {
    /// Number of seller records.
    pub sellers: i64,
    /// Number of drafts.
    pub drafts: i64,
    /// Applied schema version.
    pub schema_version: i64,
}

/// The result of a customer CSV import.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CustomerImportDto {
    /// Customers written to the archive.
    pub imported: i64,
    /// Rows that were already on file.
    pub skipped: i64,
    /// Rows that could not be read, with the line and the reason.
    pub errors: Vec<String>,
}

/// What became of one imported invoice.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImportDto {
    /// Number as the sender wrote it.
    pub number: String,
    /// Document identity in the archive.
    pub document_id: String,
    /// Supplier name, when the party table matched one.
    pub supplier: Option<String>,
    /// True when the archive already held this document, so nothing was added.
    pub duplicate: bool,
}

/// A customer as a front end edits it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CustomerDto {
    /// Customer identity. Empty when the customer has not been saved yet.
    pub id: String,
    /// Legal name.
    pub name: String,
    /// Street and number.
    pub street: Option<String>,
    /// Postal code.
    pub postal: Option<String>,
    /// City.
    pub city: String,
    /// ISO 3166-1 alpha-2 country.
    pub country: String,
    /// VAT identifier.
    pub vat: Option<String>,
    /// Chamber of commerce number.
    pub kvk: Option<String>,
    /// Peppol endpoint, `EAS:value`.
    pub peppol: Option<String>,
    /// Contact person.
    pub contact: Option<String>,
    /// Contact email.
    pub email: Option<String>,
    /// Contact phone.
    pub phone: Option<String>,
}

impl CustomerDto {
    /// Converts a stored customer.
    pub fn from_record(record: &CustomerRecord) -> Self {
        let party = &record.party;
        Self {
            id: record.id.to_string(),
            name: party.legal_name.clone(),
            street: party.address.street.clone(),
            postal: party.address.postal_code.clone(),
            city: party.address.city.clone(),
            country: party.address.country.to_string(),
            vat: party.vat_id.clone(),
            kvk: party.kvk_number.clone(),
            peppol: party.peppol_id.clone(),
            contact: party.contact_name.clone(),
            email: party.email.clone(),
            phone: party.phone.clone(),
        }
    }

    /// Rebuilds the stored customer, normalising the phone like the CSV import.
    pub fn into_record(self) -> Result<CustomerRecord, DomainError> {
        let country = CountryCode::parse(&self.country)?;
        let mut party = Party::new(
            self.name,
            crux::Address {
                street: self.street,
                city: self.city,
                postal_code: self.postal,
                country,
            },
        );
        party.vat_id = self.vat.map(|value| value.to_ascii_uppercase());
        party.kvk_number = self.kvk;
        party.peppol_id = self.peppol;
        party.contact_name = self.contact;
        party.email = self.email;
        party.phone = match self.phone.as_deref().map(str::trim) {
            Some(raw) if !raw.is_empty() => Some(PhoneNumber::parse(raw, country)?.as_str().to_owned()),
            _ => None,
        };
        Ok(CustomerRecord {
            id: CustomerId::parse_str(&self.id)?,
            party,
        })
    }
}

/// The invoice a credit note corrects.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PrecedingInvoiceDto {
    /// Preceding invoice number.
    pub number: String,
    /// Preceding invoice issue date, ISO `YYYY-MM-DD`.
    pub issue_date: String,
}

/// A seller as the settings screen edits it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SellerDto {
    /// Seller identity.
    pub id: String,
    /// Registered name.
    pub name: String,
    /// Street and number.
    pub street: Option<String>,
    /// Postal code.
    pub postal: Option<String>,
    /// City.
    pub city: String,
    /// ISO 3166-1 alpha-2 country.
    pub country: String,
    /// VAT identifier.
    pub vat: Option<String>,
    /// Chamber of commerce number.
    pub kvk: Option<String>,
    /// Peppol endpoint, `EAS:value`.
    pub peppol: Option<String>,
    /// Contact person.
    pub contact: Option<String>,
    /// Contact email.
    pub email: Option<String>,
    /// Contact phone.
    pub phone: Option<String>,
    /// ISO currency this seller issues in.
    pub default_currency: String,
    /// Locale this seller prefers.
    pub default_locale: String,
}

impl SellerDto {
    /// Converts a stored seller.
    pub fn from_record(record: &SellerRecord) -> Self {
        let party = &record.party;
        Self {
            id: record.id.to_string(),
            name: party.legal_name.clone(),
            street: party.address.street.clone(),
            postal: party.address.postal_code.clone(),
            city: party.address.city.clone(),
            country: party.address.country.to_string(),
            vat: party.vat_id.clone(),
            kvk: party.kvk_number.clone(),
            peppol: party.peppol_id.clone(),
            contact: party.contact_name.clone(),
            email: party.email.clone(),
            phone: party.phone.clone(),
            default_currency: record.default_currency.as_str().to_owned(),
            default_locale: record.default_locale.clone(),
        }
    }

    /// Rebuilds the stored seller, normalising the phone like a customer's.
    pub fn into_record(self) -> Result<SellerRecord, DomainError> {
        let country = CountryCode::parse(&self.country)?;
        let mut party = Party::new(
            self.name,
            crux::Address {
                street: self.street,
                city: self.city,
                postal_code: self.postal,
                country,
            },
        );
        party.vat_id = self.vat.map(|value| value.to_ascii_uppercase());
        party.kvk_number = self.kvk;
        party.peppol_id = self.peppol;
        party.contact_name = self.contact;
        party.email = self.email;
        party.phone = match self.phone.as_deref().map(str::trim) {
            Some(raw) if !raw.is_empty() => Some(PhoneNumber::parse(raw, country)?.as_str().to_owned()),
            _ => None,
        };
        Ok(SellerRecord {
            id: SellerId::parse_str(&self.id)?,
            party,
            default_currency: Currency::parse(&self.default_currency)?,
            default_locale: self.default_locale,
        })
    }
}

/// One line of a draft as a front end edits it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DraftLineDto {
    /// Stable line identity.
    pub id: String,
    /// 1-based position.
    pub position: u32,
    /// Item name.
    pub name: String,
    /// Optional longer description.
    pub description: Option<String>,
    /// Quantity in micro-units.
    pub quantity_micros: i64,
    /// Three-character unit code.
    pub unit: String,
    /// Unit price in micro-units.
    pub unit_price_micros: i64,
    /// Price base quantity in micro-units.
    pub price_base_quantity_micros: i64,
    /// EN 16931 VAT category code, such as `S` or `AE`.
    pub tax_category: String,
    /// VAT rate in basis points.
    pub tax_rate_bp: u16,
    /// Line net in micro-units, rounded to the cent.
    pub line_net_micros: i64,
    /// Exemption reason, when the category needs one.
    pub exemption_reason: Option<String>,
    /// Exemption reason code, when the category needs one.
    pub exemption_reason_code: Option<String>,
}

impl DraftLineDto {
    /// Converts a domain line, calculating its rounded net once.
    pub fn from_line(line: &DraftLine) -> Result<Self, DomainError> {
        Ok(Self {
            id: line.id.to_string(),
            position: line.position,
            name: line.name.clone(),
            description: line.description.clone(),
            quantity_micros: line.quantity.micros(),
            unit: line.unit.as_str().to_owned(),
            unit_price_micros: line.unit_price.micros(),
            price_base_quantity_micros: line.price_base_quantity.micros(),
            tax_category: line.treatment.category.code().to_owned(),
            tax_rate_bp: line.treatment.rate.basis_points(),
            line_net_micros: line.net()?.micros(),
            exemption_reason: line.treatment.exemption_reason.clone(),
            exemption_reason_code: line.treatment.exemption_reason_code.clone(),
        })
    }
}

/// A draft as a front end edits it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DraftDto {
    /// Draft identity.
    pub id: String,
    /// Seller identity.
    pub seller_id: String,
    /// Customer identity.
    pub customer_id: String,
    /// Document type code: `380` for an invoice, `381` for a credit note.
    pub doc_type: String,
    /// ISO currency code.
    pub currency: String,
    /// Issue date, ISO `YYYY-MM-DD`.
    pub issue_date: String,
    /// Due date, ISO `YYYY-MM-DD`.
    pub due_date: Option<String>,
    /// Delivery date, ISO `YYYY-MM-DD`.
    pub delivery_date: Option<String>,
    /// Buyer reference.
    pub buyer_reference: Option<String>,
    /// Order reference.
    pub order_reference: Option<String>,
    /// Free-text note.
    pub note: Option<String>,
    /// Template file name, or `None` for the archive default.
    pub template: Option<String>,
    /// The invoice a credit note corrects.
    pub preceding_invoice: Option<PrecedingInvoiceDto>,
    /// Lines.
    pub lines: Vec<DraftLineDto>,
}

impl DraftDto {
    /// Converts a draft, keeping dates ISO and money in micro-units.
    pub fn from_draft(draft: &InvoiceDraft) -> Result<Self, DomainError> {
        Ok(Self {
            id: draft.id.to_string(),
            seller_id: draft.seller_id.to_string(),
            customer_id: draft.customer_id.to_string(),
            doc_type: draft.doc_type.code().to_owned(),
            currency: draft.currency.as_str().to_owned(),
            issue_date: draft.issue_date.to_iso_string(),
            due_date: draft.due_date.map(crux::LocalDate::to_iso_string),
            delivery_date: draft.delivery_date.map(crux::LocalDate::to_iso_string),
            buyer_reference: draft.buyer_reference.clone(),
            order_reference: draft.order_reference.clone(),
            note: draft.note.clone(),
            template: draft.template.clone(),
            preceding_invoice: draft.preceding_invoice.as_ref().map(|preceding| PrecedingInvoiceDto {
                number: preceding.number.clone(),
                issue_date: preceding.issue_date.to_iso_string(),
            }),
            lines: draft
                .lines
                .iter()
                .map(DraftLineDto::from_line)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    /// Rebuilds the domain draft, parsing every crossed field.
    pub fn into_draft(self) -> Result<InvoiceDraft, DomainError> {
        let currency = Currency::parse(&self.currency)?;
        let doc_type = match self.doc_type.as_str() {
            "380" => DocType::Invoice,
            "381" => DocType::CreditNote,
            other => return Err(DomainError::InvalidDocType(other.to_owned())),
        };
        let preceding_invoice = match self.preceding_invoice {
            Some(preceding) => Some(PrecedingInvoice {
                // The DTO does not carry the preceding document id; a credit
                // note saved from the editor will need it before it can link.
                id: DocumentId::new(),
                number: preceding.number,
                issue_date: LocalDate::parse_iso(&preceding.issue_date)?,
            }),
            None => None,
        };
        let mut lines = Vec::with_capacity(self.lines.len());
        for line in self.lines {
            lines.push(DraftLine {
                id: LineId::parse_str(&line.id)?,
                position: line.position,
                name: line.name,
                description: line.description,
                quantity: Quantity::from_micros(line.quantity_micros),
                unit: UnitCode::parse(&line.unit)?,
                unit_price: Money::from_micros(line.unit_price_micros, currency),
                price_base_quantity: Quantity::from_micros(line.price_base_quantity_micros),
                treatment: TaxTreatment {
                    category: VatCategory::parse(&line.tax_category)?,
                    rate: VatRate::from_basis_points(line.tax_rate_bp)?,
                    exemption_reason: line.exemption_reason,
                    exemption_reason_code: line.exemption_reason_code,
                },
            });
        }
        Ok(InvoiceDraft {
            id: DocumentId::parse_str(&self.id)?,
            seller_id: SellerId::parse_str(&self.seller_id)?,
            customer_id: CustomerId::parse_str(&self.customer_id)?,
            doc_type,
            currency,
            issue_date: LocalDate::parse_iso(&self.issue_date)?,
            due_date: self.due_date.as_deref().map(LocalDate::parse_iso).transpose()?,
            delivery_date: self.delivery_date.as_deref().map(LocalDate::parse_iso).transpose()?,
            buyer_reference: self.buyer_reference,
            order_reference: self.order_reference,
            note: self.note,
            template: self.template,
            preceding_invoice,
            lines,
        })
    }
}

/// Totals for a draft preview, in micro-units.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DraftTotalsDto {
    /// Sum of line nets (BT-106).
    pub net_micros: i64,
    /// Sum of tax per category (BT-110).
    pub tax_micros: i64,
    /// Net plus tax (BT-112).
    pub gross_micros: i64,
}

impl From<&InvoiceTotals> for DraftTotalsDto {
    fn from(totals: &InvoiceTotals) -> Self {
        Self {
            net_micros: totals.net.micros(),
            tax_micros: totals.tax.micros(),
            gross_micros: totals.gross.micros(),
        }
    }
}

/// One validation finding, as data rather than an error string.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FindingDto {
    /// Stable code, such as `BR-16`.
    pub code: String,
    /// `blocking`, `warning`, or `info`.
    pub severity: String,
    /// EN 16931 business term, when one exists.
    pub bt: Option<String>,
    /// Field path the UI focuses.
    pub field: String,
    /// Human-readable fallback message.
    pub message: String,
}

impl From<&Finding> for FindingDto {
    fn from(finding: &Finding) -> Self {
        Self {
            code: finding.code.to_owned(),
            severity: severity_code(finding.severity).to_owned(),
            bt: finding.bt.map(str::to_owned),
            field: finding.field.clone(),
            message: finding.message.clone(),
        }
    }
}

/// The stable word for a severity.
pub const fn severity_code(severity: Severity) -> &'static str {
    match severity {
        Severity::Blocking => "blocking",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

/// An issued document, as much as a list or a detail header needs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocumentSummary {
    /// Document identity.
    pub id: String,
    /// Human number.
    pub number: String,
    /// Issue date, ISO `YYYY-MM-DD`.
    pub issue_date: String,
    /// ISO currency code.
    pub currency: String,
    /// Net in micro-units.
    pub net_micros: i64,
    /// Tax in micro-units.
    pub tax_micros: i64,
    /// Gross in micro-units.
    pub gross_micros: i64,
    /// Stable status code.
    pub status: String,
}

impl DocumentSummary {
    /// Converts an issued record and its current status.
    pub fn from_record(record: &DocumentRecord, status: DocumentStatus) -> Self {
        Self {
            id: record.id.to_string(),
            number: record.number.clone(),
            issue_date: record.issue_date.to_iso_string(),
            currency: record.totals.net.currency().as_str().to_owned(),
            net_micros: record.totals.net.micros(),
            tax_micros: record.totals.tax.micros(),
            gross_micros: record.totals.gross.micros(),
            status: status.code().to_owned(),
        }
    }
}

/// The result of an issue attempt: either a document or the findings that
/// stopped it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct IssueOutcome {
    /// The issued document, when nothing blocked it.
    pub issued: Option<DocumentSummary>,
    /// Every finding, blocking or not.
    pub findings: Vec<FindingDto>,
    /// SHA-256 of the stored UBL artefact, when a document was issued.
    pub ubl_sha256: Option<String>,
    /// SHA-256 of the stored PDF artefact, when one was rendered and filed.
    pub pdf_sha256: Option<String>,
    /// Why the PDF was not filed, when the document was issued anyway.
    pub pdf_error: Option<String>,
}

/// One row in the document register.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocumentHit {
    /// Document identity.
    pub id: String,
    /// Human number; empty for a draft that has none yet.
    pub number: String,
    /// `outbound` or `inbound`.
    pub direction: String,
    /// The party on the other side.
    pub party: String,
    /// Issue date, ISO `YYYY-MM-DD`.
    pub issue_date: String,
    /// ISO currency code.
    pub currency: String,
    /// Gross in micro-units.
    pub gross_micros: i64,
    /// Stable status code.
    pub status: String,
    /// True when the party on the other side is on file.
    pub matched: bool,
}

impl DocumentHit {
    /// Converts one search result.
    pub fn from_hit(hit: &SearchHit) -> Self {
        Self {
            id: hit.id.to_string(),
            number: hit.number.clone(),
            direction: hit.direction.code().to_owned(),
            party: hit.party.clone(),
            issue_date: hit.issue_date.to_iso_string(),
            currency: hit.currency.as_str().to_owned(),
            gross_micros: hit.gross.micros(),
            status: hit.status.code().to_owned(),
            matched: hit.matched,
        }
    }
}

/// A stored document, as the detail screen shows it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocumentDetail {
    /// Document identity.
    pub id: String,
    /// Human number.
    pub number: String,
    /// `outbound` or `inbound`.
    pub direction: String,
    /// UN/CEFACT type, `380` or `381`.
    pub doc_type: String,
    /// ISO currency code.
    pub currency: String,
    /// Issue date, ISO `YYYY-MM-DD`.
    pub issue_date: String,
    /// Due date, when the document has one.
    pub due_date: Option<String>,
    /// Delivery date, when the document has one.
    pub delivery_date: Option<String>,
    /// Buyer reference (BT-10).
    pub buyer_reference: Option<String>,
    /// Order reference (BT-13).
    pub order_reference: Option<String>,
    /// Free-text note (BT-22).
    pub note: Option<String>,
    /// Issuer as it was on the day.
    pub seller: String,
    /// Buyer as it was on the day.
    pub buyer: String,
    /// The customer a received document's supplier resolves to.
    ///
    /// `None` for outbound documents and for imports whose issuer is not on
    /// file, which is what the match action explains.
    pub supplier_id: Option<String>,
    /// Net in micro-units.
    pub net_micros: i64,
    /// Tax in micro-units.
    pub tax_micros: i64,
    /// Gross in micro-units.
    pub gross_micros: i64,
    /// Stable status code.
    pub status: String,
    /// When the archive recorded it, RFC 3339.
    pub recorded_at: String,
    /// Lines, in position order.
    pub lines: Vec<DocumentLine>,
    /// Artefacts with their hashes, oldest first.
    pub artefacts: Vec<DocumentArtefact>,
    /// Status events, oldest first.
    pub events: Vec<DocumentEvent>,
}

impl DocumentDetail {
    /// Converts a stored document.
    pub fn from_view(view: &DocumentView) -> Self {
        Self {
            id: view.id.to_string(),
            number: view.number.clone(),
            direction: view.direction.code().to_owned(),
            doc_type: view.doc_type.code().to_owned(),
            currency: view.currency.as_str().to_owned(),
            issue_date: view.issue_date.to_iso_string(),
            due_date: view.due_date.map(LocalDate::to_iso_string),
            delivery_date: view.delivery_date.map(LocalDate::to_iso_string),
            buyer_reference: view.buyer_reference.clone(),
            order_reference: view.order_reference.clone(),
            note: view.note.clone(),
            seller: view.seller.legal_name.clone(),
            buyer: view.buyer.legal_name.clone(),
            supplier_id: view.supplier_match.map(|id| id.to_string()),
            net_micros: view.totals.net.micros(),
            tax_micros: view.totals.tax.micros(),
            gross_micros: view.totals.gross.micros(),
            status: view.status.code().to_owned(),
            recorded_at: view.recorded_at.clone(),
            lines: view.lines.iter().map(DocumentLine::from_stored).collect(),
            artefacts: view.artefacts.iter().map(DocumentArtefact::from_info).collect(),
            events: view.events.iter().map(DocumentEvent::from_stored).collect(),
        }
    }
}

/// One line of a stored document, as recorded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocumentLine {
    /// Position on the document.
    pub position: i64,
    /// Item name.
    pub name: String,
    /// Optional longer description.
    pub description: Option<String>,
    /// Quantity, trimmed (for example `3` or `1.5`).
    pub quantity: String,
    /// Unit of measure code.
    pub unit: String,
    /// Unit price in micro-units.
    pub unit_price_micros: i64,
    /// VAT category code.
    pub vat_category: String,
    /// VAT rate in basis points.
    pub vat_rate_bp: u16,
    /// Line net in micro-units, as recorded.
    pub line_net_micros: i64,
    /// Tax on the line in micro-units, as recorded.
    pub tax_micros: i64,
}

impl DocumentLine {
    /// Converts one stored line.
    pub fn from_stored(line: &store::archive::StoredLine) -> Self {
        Self {
            position: line.position,
            name: line.name.clone(),
            description: line.description.clone(),
            quantity: line.quantity.to_trimmed_string(),
            unit: line.unit.as_str().to_owned(),
            unit_price_micros: line.unit_price.micros(),
            vat_category: line.vat_category.code().to_owned(),
            vat_rate_bp: line.vat_rate.basis_points(),
            line_net_micros: line.line_net.micros(),
            tax_micros: line.tax.micros(),
        }
    }
}

/// One artefact filed under a document.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocumentArtefact {
    /// `ubl`, `pdf`, and so on.
    pub kind: String,
    /// Syntax profile, when the kind has one.
    pub syntax_profile: Option<String>,
    /// SHA-256 of the bytes.
    pub bytes_hash: String,
    /// Size in bytes.
    pub size_bytes: i64,
    /// When it was rendered, RFC 3339.
    pub rendered_at: String,
    /// What rendered it.
    pub renderer_version: String,
}

impl DocumentArtefact {
    /// Converts one stored artefact.
    pub fn from_info(info: &store::archive::ArtefactInfo) -> Self {
        Self {
            kind: info.kind.clone(),
            syntax_profile: info.syntax_profile.clone(),
            bytes_hash: info.bytes_hash.clone(),
            size_bytes: info.size_bytes,
            rendered_at: info.rendered_at.clone(),
            renderer_version: info.renderer_version.clone(),
        }
    }
}

/// One status change on a document.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocumentEvent {
    /// Status code.
    pub kind: String,
    /// When it happened, RFC 3339.
    pub at: String,
    /// Who decided.
    pub actor: String,
    /// Optional detail, such as a payment reference.
    pub detail: Option<String>,
}

impl DocumentEvent {
    /// Converts one stored event.
    pub fn from_stored(event: &store::archive::StatusEvent) -> Self {
        Self {
            kind: event.kind.clone(),
            at: event.at.clone(),
            actor: event.actor.clone(),
            detail: event.detail.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crux::{
        Currency, CustomerId, DocType, DocumentId, DraftLine, InvoiceTotals, LineId, LocalDate, Money, Quantity,
        SellerId, TaxTreatment, UnitCode, VatRate,
    };

    fn line() -> DraftLine {
        DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Architecture review".to_owned(),
            description: Some("Discovery workshop".to_owned()),
            quantity: Quantity::parse("3").expect("a quantity"),
            unit: UnitCode::HOUR,
            unit_price: Money::from_major_units(95, Currency::EUR).expect("a price"),
            price_base_quantity: Quantity::ONE,
            treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
        }
    }

    fn draft() -> InvoiceDraft {
        InvoiceDraft {
            id: DocumentId::new(),
            seller_id: SellerId::new(),
            customer_id: CustomerId::new(),
            doc_type: DocType::Invoice,
            currency: Currency::EUR,
            issue_date: LocalDate::parse_iso("2026-11-07").expect("a date"),
            due_date: Some(LocalDate::parse_iso("2026-12-07").expect("a date")),
            delivery_date: None,
            buyer_reference: Some("PO-1".to_owned()),
            order_reference: None,
            note: Some("Thanks".to_owned()),
            preceding_invoice: None,
            template: None,
            lines: vec![line()],
        }
    }

    #[test]
    fn a_draft_crosses_as_iso_dates_and_micros() {
        let dto = DraftDto::from_draft(&draft()).expect("a draft");
        assert_eq!(dto.doc_type, "380");
        assert_eq!(dto.currency, "EUR");
        assert_eq!(dto.issue_date, "2026-11-07");
        assert_eq!(dto.due_date.as_deref(), Some("2026-12-07"));
        assert_eq!(dto.lines.len(), 1);

        let line = &dto.lines[0];
        assert_eq!(line.unit, "HUR");
        assert_eq!(line.quantity_micros, 3_000_000);
        assert_eq!(line.unit_price_micros, 95_000_000);
        assert_eq!(line.tax_category, "S");
        assert_eq!(line.tax_rate_bp, 2100);
        assert_eq!(line.line_net_micros, 285_000_000);
    }

    #[test]
    fn a_draft_round_trips_through_the_dto() {
        let original = draft();
        let dto = DraftDto::from_draft(&original).expect("a draft");
        let rebuilt = dto.into_draft().expect("a draft");
        assert_eq!(rebuilt, original);
    }

    #[test]
    fn a_finding_crosses_as_data() {
        let finding = Finding::blocking("BR-16", Some("BT-129"), "lines[0].quantity", "quantity missing");
        let dto = FindingDto::from(&finding);
        assert_eq!(dto.code, "BR-16");
        assert_eq!(dto.severity, "blocking");
        assert_eq!(dto.bt.as_deref(), Some("BT-129"));
        assert_eq!(dto.field, "lines[0].quantity");
        assert_eq!(dto.message, "quantity missing");
    }

    #[test]
    fn an_issued_record_crosses_as_a_document_summary() {
        let record = DocumentRecord {
            id: DocumentId::new(),
            number: "2026-0001".to_owned(),
            issue_date: LocalDate::parse_iso("2026-11-07").expect("a date"),
            totals: InvoiceTotals {
                net: Money::from_major_units(100, Currency::EUR).expect("net"),
                tax: Money::from_major_units(21, Currency::EUR).expect("tax"),
                gross: Money::from_major_units(121, Currency::EUR).expect("gross"),
                breakdown: Vec::new(),
            },
            ubl_sha256: "00".to_owned(),
            audit_head: "00".to_owned(),
        };

        let dto = DocumentSummary::from_record(&record, DocumentStatus::Issued);
        assert_eq!(dto.number, "2026-0001");
        assert_eq!(dto.issue_date, "2026-11-07");
        assert_eq!(dto.currency, "EUR");
        assert_eq!(dto.gross_micros, 121_000_000);
        assert_eq!(dto.status, "issued");
    }
}
