//! Turning a draft into the two artefacts a document is filed under.
//!
//! UBL is what a Peppol access point reads; the PDF is what a person reads. Both
//! are produced here so the desktop window, the CLI, and the mobile shell cannot
//! drift into two slightly different invoice pipelines.

use conformance::{Profile, SemanticInvoice};
use crux::{FacturXProfile, Finding, InvoiceDraft, InvoiceTotals, LocalDate, Party};
use render::PdfRenderer;
use store::{CustomerRecord, DocumentStatus, IssueRequest, IssueView, SellerRecord, StoreError};
use syntax::{Invoice, InvoiceLine, InvoiceReference, write_cii_at, write_ubl};

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
/// as well, because one process holds the session. `profile` is the Factur-X
/// level the hybrid is written at; it is recorded on the artefact row, so a
/// reader of the archive can see what was claimed without opening the bytes.
pub fn issue(
    archive: &mut Archive,
    draft: &DraftDto,
    language: DocumentLanguage,
    template: Option<&ArchiveTemplate>,
    profile: FacturXProfile,
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
    // layout must not roll back an issued document. The hybrid is the same page
    // with the CII inside, so it is filed in the same breath.
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

    let view = IssueView {
        id: record.id,
        number: &record.number,
        draft: &domain,
        seller: &seller,
        customer: &customer,
        totals: &record.totals,
    };
    let mut facturx_sha256 = None;
    let mut facturx_error = None;
    match attach_facturx(
        archive,
        &view,
        language,
        template.map(|template| &template.template),
        template.map(|template| template.sha256.as_str()),
        profile,
        &now,
    ) {
        Ok(hash) => facturx_sha256 = Some(hash),
        // Same rule as the PDF: the document is issued and filed either way.
        Err(error) => facturx_error = Some(error.to_string()),
    }

    Ok(IssueOutcome {
        issued: Some(DocumentSummary::from_record(&record, DocumentStatus::Issued)),
        findings: findings.iter().map(FindingDto::from).collect(),
        ubl_sha256: Some(record.ubl_sha256.clone()),
        pdf_sha256,
        pdf_error,
        facturx_sha256,
        facturx_error,
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
        facturx_sha256: None,
        facturx_error: None,
    }
}

/// Renders a document to UBL for the issue transaction.
///
/// Shared by the GUI and the CLI so both produce byte-identical artefacts. UBL is
/// Peppol output, so it passes the Peppol gate whichever Factur-X level the
/// hybrid is written at.
pub fn render_issue(view: &IssueView<'_>) -> Result<Vec<u8>, StoreError> {
    gate(view, Profile::PeppolBis3)?;
    let lines = invoice_lines(view.draft)?;
    write_ubl(&invoice(view, &lines))
        .map(String::into_bytes)
        .map_err(|error| StoreError::Render(error.to_string()))
}

/// Renders the CII invoice a Factur-X PDF carries, at a stated level.
///
/// The level decides both what is written and whether the EN 16931 gate runs: the
/// three levels that claim EN 16931 are checked against it, and the two below it
/// are checked by the writer, which is the last place that knows what the level
/// actually carries. A document no profile accepts must not leave as a hybrid
/// PDF either, because the XML inside it is the invoice.
pub fn render_cii(view: &IssueView<'_>, profile: FacturXProfile) -> Result<Vec<u8>, StoreError> {
    if profile.claims_en16931() {
        gate(view, Profile::En16931)?;
    }
    let lines = invoice_lines(view.draft)?;
    write_cii_at(&invoice(view, &lines), profile)
        .map(String::into_bytes)
        .map_err(|error| StoreError::Render(error.to_string()))
}

/// Renders the document as Factur-X: the printed page with the CII inside.
pub fn facturx(
    view: &IssueView<'_>,
    language: render::Language,
    template: Option<&render::Template>,
    profile: FacturXProfile,
) -> Result<Vec<u8>, StoreError> {
    let cii = render_cii(view, profile)?;
    facturx_from_cii(view, &cii, language, template, profile)
}

/// The container, when the CII was already produced for something else.
///
/// The CLI writes the CII artefact as part of the issue transaction, so it hands
/// the bytes back rather than serialising the invoice a second time. The level
/// passed here is the one those bytes were written at; the packet repeats it.
pub fn facturx_from_cii(
    view: &IssueView<'_>,
    cii: &[u8],
    language: render::Language,
    template: Option<&render::Template>,
    profile: FacturXProfile,
) -> Result<Vec<u8>, StoreError> {
    let data = pdf_data(
        view.number,
        view.draft,
        view.seller,
        view.customer,
        view.totals,
        language,
    );
    renderer_for(template)
        .render_facturx(&data, cii, profile.conformance_level())
        .map_err(|error| StoreError::Render(error.to_string()))
}

/// Renders and files the hybrid, stating the level on the artefact.
pub fn attach_facturx(
    archive: &Archive,
    view: &IssueView<'_>,
    language: render::Language,
    template: Option<&render::Template>,
    template_digest: Option<&str>,
    profile: FacturXProfile,
    now: &str,
) -> Result<String, StoreError> {
    let bytes = facturx(view, language, template, profile)?;
    store::attach_artefact_with_profile(
        archive.database().connection(),
        view.id,
        "facturx",
        Some(&profile.artefact_profile()),
        &bytes,
        render::RENDERER_VERSION,
        template_digest,
        now,
    )
}

/// Runs the conformance gate a payload has to pass before it is serialised.
fn gate(view: &IssueView<'_>, profile: Profile) -> Result<(), StoreError> {
    let semantic = SemanticInvoice::from_draft(
        view.number,
        view.draft,
        &view.seller.party,
        &view.customer.party,
        view.totals.clone(),
        profile,
    );
    let findings = conformance::validate(&semantic);
    if crux::has_blocking(&findings) {
        return Err(StoreError::Conformance(findings));
    }
    Ok(())
}

/// The lines of a draft, with each line's net computed once.
fn invoice_lines(draft: &InvoiceDraft) -> Result<Vec<InvoiceLine<'_>>, StoreError> {
    let mut lines = Vec::with_capacity(draft.lines.len());
    for line in &draft.lines {
        let net = line.net().map_err(|error| StoreError::Data(error.to_string()))?;
        lines.push(InvoiceLine {
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
    Ok(lines)
}

/// The view both syntax writers serialise.
fn invoice<'a>(view: &'a IssueView<'a>, lines: &'a [InvoiceLine<'a>]) -> Invoice<'a> {
    Invoice {
        number: view.number,
        issue_date: view.draft.issue_date,
        due_date: view.draft.due_date,
        delivery_date: view.draft.delivery_date,
        doc_type: view.draft.doc_type,
        currency: view.draft.currency,
        buyer_reference: view.draft.buyer_reference.as_deref(),
        order_reference: view.draft.order_reference.as_deref(),
        note: view.draft.note.as_deref(),
        preceding_invoice: view.draft.preceding_invoice.as_ref().map(|preceding| InvoiceReference {
            number: &preceding.number,
            issue_date: preceding.issue_date,
        }),
        seller: &view.seller.party,
        customer: &view.customer.party,
        lines,
        totals: view.totals,
    }
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
