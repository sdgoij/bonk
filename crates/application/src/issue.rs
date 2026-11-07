//! Turning a draft into the two artefacts a document is filed under.
//!
//! UBL is what a Peppol access point reads; the PDF is what a person reads. Both
//! are produced here so the desktop window, the CLI, and the mobile shell cannot
//! drift into two slightly different invoice pipelines.

use conformance::{Profile, SemanticInvoice};
use crux::{InvoiceDraft, InvoiceTotals, LocalDate, Party};
use render::PdfRenderer;
use store::{CustomerRecord, IssueView, SellerRecord, StoreError};
use syntax::{UblInvoice, UblLine, UblPrecedingInvoice, write_invoice};

use crate::templates::renderer_for;

/// The language the printed document speaks.
///
/// Re-exported so a front end that renders through [`invoice_pdf`] does not need
/// a direct dependency on the renderer.
pub use render::Language as DocumentLanguage;

/// Renders a document to UBL for the issue transaction.
///
/// Shared by the GUI and the CLI so both produce byte-identical artefacts.
pub fn render_issue(view: &IssueView<'_>) -> Result<Vec<u8>, StoreError> {
    let semantic = SemanticInvoice::from_draft(
        view.number,
        view.draft,
        &view.seller.party,
        &view.customer.party,
        view.totals.clone(),
        Profile::PeppolBis3,
    );
    let findings = conformance::validate(&semantic);
    if crux::has_blocking(&findings) {
        return Err(StoreError::Conformance(findings));
    }

    let mut lines = Vec::with_capacity(view.draft.lines.len());
    for line in &view.draft.lines {
        let net = line.net().map_err(|error| StoreError::Data(error.to_string()))?;
        lines.push(UblLine {
            id: line.position,
            name: &line.name,
            description: line.description.as_deref(),
            quantity: line.quantity,
            unit: line.unit,
            unit_price: line.unit_price,
            base_quantity: line.price_base_quantity,
            line_net: net,
            treatment: &line.treatment,
        });
    }

    let invoice = UblInvoice {
        number: view.number,
        issue_date: view.draft.issue_date,
        due_date: view.draft.due_date,
        delivery_date: view.draft.delivery_date,
        doc_type: view.draft.doc_type,
        currency: view.draft.currency,
        buyer_reference: view.draft.buyer_reference.as_deref(),
        order_reference: view.draft.order_reference.as_deref(),
        note: view.draft.note.as_deref(),
        preceding_invoice: view
            .draft
            .preceding_invoice
            .as_ref()
            .map(|preceding| UblPrecedingInvoice {
                number: &preceding.number,
                issue_date: preceding.issue_date,
            }),
        seller: &view.seller.party,
        customer: &view.customer.party,
        lines: &lines,
        totals: view.totals,
    };

    write_invoice(&invoice)
        .map(String::into_bytes)
        .map_err(|error| StoreError::Render(error.to_string()))
}

/// Renders the visual invoice PDF for an issued document.
///
/// The layout engine and fonts are linked in, so this needs nothing installed
/// and produces the same bytes wherever it runs.
pub fn invoice_pdf(
    number: &str,
    draft: &InvoiceDraft,
    seller: &SellerRecord,
    customer: &CustomerRecord,
    totals: &InvoiceTotals,
    language: render::Language,
    template: Option<&render::Template>,
) -> Result<Vec<u8>, StoreError> {
    let data = pdf_data(number, draft, seller, customer, totals, language);

    renderer_for(template)
        .render(&data)
        .map_err(|error| StoreError::Render(error.to_string()))
}

/// The payload handed to the template.
///
/// Kept apart from the renderer so the shape can be checked without compiling a
/// document, and so the words the document prints are decided in one place.
pub fn pdf_data(
    number: &str,
    draft: &InvoiceDraft,
    seller: &SellerRecord,
    customer: &CustomerRecord,
    totals: &InvoiceTotals,
    language: render::Language,
) -> render::InvoiceData {
    render::InvoiceData {
        title: language.document_title(draft.doc_type.is_credit_note()).to_owned(),
        labels: language.labels(),
        number: number.to_owned(),
        issue_date: draft.issue_date.to_iso_string(),
        due_date: draft.due_date.map(LocalDate::to_iso_string),
        currency: draft.currency.to_string(),
        seller: party_data(&seller.party),
        customer: party_data(&customer.party),
        lines: draft
            .lines
            .iter()
            .map(|line| render::LineData {
                name: line.name.clone(),
                qty: line.quantity.to_trimmed_string(),
                price: line.unit_price.to_trimmed_string(),
                vat: line.treatment.rate.percent_string(),
                total: line
                    .net()
                    .ok()
                    .and_then(|net| net.to_fixed_string(2).ok())
                    .unwrap_or_default(),
            })
            .collect(),
        net: totals.net.to_fixed_string(2).unwrap_or_default(),
        tax: totals.tax.to_fixed_string(2).unwrap_or_default(),
        gross: totals.gross.to_fixed_string(2).unwrap_or_default(),
        notes: draft.note.clone(),
    }
}

fn party_data(party: &Party) -> render::PartyData {
    let mut parts = Vec::new();
    if let Some(street) = &party.address.street {
        parts.push(street.clone());
    }
    parts.push(party.address.city.clone());
    parts.push(party.address.country.to_string());
    render::PartyData {
        name: party.legal_name.clone(),
        address: parts.join(", "),
        kvk: party.kvk_number.clone(),
        vat: party.vat_id.clone(),
    }
}
