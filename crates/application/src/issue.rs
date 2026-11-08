//! Turning a draft into the two artefacts a document is filed under.
//!
//! UBL is what a Peppol access point reads; the PDF is what a person reads. Both
//! are produced here so the desktop window, the CLI, and the mobile shell cannot
//! drift into two slightly different invoice pipelines.

use conformance::{Profile, SemanticInvoice};
use crux::{Finding, InvoiceDraft, InvoiceTotals, LocalDate, Party};
use render::PdfRenderer;
use store::{CustomerRecord, DocumentStatus, IssueRequest, IssueView, SellerRecord, StoreError};
use syntax::{UblInvoice, UblLine, UblPrecedingInvoice, write_invoice};

use crate::dto::{DocumentSummary, DraftDto, FindingDto, IssueOutcome};
use crate::session::Archive;
use crate::templates::ArchiveTemplate;
use crate::templates::renderer_for;

/// The language the printed document speaks.
///
/// Re-exported so a front end that renders through [`invoice_pdf`] does not need
/// a direct dependency on the renderer.
pub use render::Language as DocumentLanguage;

/// Issues a draft: validate, render UBL, render the PDF, file both.
///
/// The draft is saved first, because an editor may hold edits newer than its
/// last save. A blocking finding stops the attempt before a number is allocated,
/// and the draft keeps its identity so it can be corrected and retried.
///
/// `actor` names the front end in the audit trail; it is recorded as the device
/// as well, because one process holds the session.
pub fn issue(
    archive: &mut Archive,
    draft: &DraftDto,
    language: DocumentLanguage,
    template: Option<&ArchiveTemplate>,
    actor: &str,
) -> Result<IssueOutcome, StoreError> {
    let domain = draft
        .clone()
        .into_draft()
        .map_err(|error| StoreError::Data(error.to_string()))?;
    let now = store::now_rfc3339();
    store::save_draft(archive.database().connection(), &domain, &now)?;

    let seller = store::load_seller(archive.database().connection(), domain.seller_id)?;
    let customer = store::load_customer(archive.database().connection(), domain.customer_id)?;

    let findings = crux::validate_draft(&domain, &seller.party, &customer.party);
    let findings = without_suppressed(archive, domain.id, findings)?;
    if crux::has_blocking(&findings) {
        return Ok(stopped(findings));
    }

    let record = match store::issue_draft(
        archive.database_mut().connection_mut(),
        IssueRequest {
            draft_id: domain.id,
            actor,
            device: actor,
            renderer_version: "syntax-0.1",
            now: &now,
        },
        render_issue,
    ) {
        Ok(record) => record,
        // The conformance profile can reject a document that passed tier 1. That
        // is a reviewable outcome, not a failed transaction, so the caller gets
        // the findings the official profile raised.
        Err(StoreError::Conformance(findings)) => return Ok(stopped(findings)),
        Err(error) => return Err(error),
    };

    // The document stands even when the PDF cannot be drawn, matching the
    // desktop: the UBL artefact is what the archive is audited on, so a failed
    // layout must not roll back an issued document.
    let mut pdf_sha256 = None;
    let mut pdf_error = None;
    match invoice_pdf(
        &record.number,
        &domain,
        &seller,
        &customer,
        &record.totals,
        language,
        template.map(|template| &template.template),
    ) {
        Ok(bytes) => match store::attach_artefact(
            archive.database().connection(),
            record.id,
            "pdf",
            &bytes,
            render::RENDERER_VERSION,
            template.map(|template| template.sha256.as_str()),
            &now,
        ) {
            Ok(hash) => pdf_sha256 = Some(hash),
            Err(error) => pdf_error = Some(error.to_string()),
        },
        Err(error) => pdf_error = Some(error.to_string()),
    }

    Ok(IssueOutcome {
        issued: Some(DocumentSummary::from_record(&record, DocumentStatus::Issued)),
        findings: findings.iter().map(FindingDto::from).collect(),
        ubl_sha256: Some(record.ubl_sha256.clone()),
        pdf_sha256,
        pdf_error,
    })
}

/// Removes findings the user accepted with a reason.
///
/// A blocking finding cannot be suppressed (the store refuses to record one) and
/// a code keeps its severity, so filtering by code and field cannot hide one.
pub(crate) fn without_suppressed(
    archive: &Archive,
    draft_id: crux::DocumentId,
    findings: Vec<Finding>,
) -> Result<Vec<Finding>, StoreError> {
    let suppressions = store::list_suppressions(archive.database().connection(), draft_id)?;
    Ok(findings
        .into_iter()
        .filter(|finding| {
            !suppressions
                .iter()
                .any(|suppression| suppression.code == finding.code && suppression.field == finding.field)
        })
        .collect())
}

fn stopped(findings: Vec<Finding>) -> IssueOutcome {
    IssueOutcome {
        issued: None,
        findings: findings.iter().map(FindingDto::from).collect(),
        ubl_sha256: None,
        pdf_sha256: None,
        pdf_error: None,
    }
}

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
