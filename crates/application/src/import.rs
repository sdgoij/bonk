//! Received-document import: bytes or a file in, a stored inbound document out.
//!
//! The format readers and the folder watcher stay in the `import` crate; this
//! module is the use case a front end calls, so the desktop and mobile shells do
//! not each have to remember the argument order or the dedupe rules.
//!
//! A document that arrives as data also gets a page. The archive keeps the bytes
//! exactly as received, and files beside them a hybrid drawn from the terms that
//! were read, so a register can show a supplier's invoice without anyone opening
//! an XML file. Nothing inbound is rewritten: the hybrid is a second artefact,
//! named as ours.

use std::path::Path;

use crux::{DocumentId, FacturXProfile, SellerId};
use store::Database;
use syntax::{ImportedInvoice, Invoice, InvoiceLine, InvoiceReference, write_cii_at};

use crate::DocumentLanguage;
use crate::issue::party_data;

pub use ::import::{ImportError, ImportOutcome};

/// Imports a file as a received document, taking the display name from its path.
pub fn import_file(database: &Database, owner: SellerId, path: &Path, now: &str) -> Result<ImportOutcome, ImportError> {
    let display = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|error| ImportError::Read {
        path: display.clone(),
        detail: error.to_string(),
    })?;
    import_bytes(database, owner, Some(&display), &bytes, now)
}

/// Imports bytes as a received document under an optional display name.
///
/// This is the mobile path: SAF or a share intent hands over bytes and a name,
/// and nothing on the host needs a filesystem path.
pub fn import_bytes(
    database: &Database,
    owner: SellerId,
    name: Option<&str>,
    bytes: &[u8],
    now: &str,
) -> Result<ImportOutcome, ImportError> {
    let mut outcome = ::import::import_bytes(database, owner, name, bytes, now)?;
    if outcome.duplicate {
        return Ok(outcome);
    }
    // A PDF already is the page: what the supplier sent stays what was filed, and
    // drawing a second page for it would add nothing a reader can use.
    if matches!(::import::sniff(bytes), ::import::SniffedKind::Pdf) {
        return Ok(outcome);
    }

    match attach_hybrid(database, owner, outcome.document_id, bytes, now) {
        Ok(hash) => outcome.facturx_sha256 = Some(hash),
        // The document is imported and filed either way; a page we could not draw
        // is reported and never rolls the import back.
        Err(error) => outcome.facturx_error = Some(error),
    }
    Ok(outcome)
}

/// Draws the received document and files the hybrid beside its payload.
///
/// Returns the hash of the artefact, or the reason it was not filed.
fn attach_hybrid(
    database: &Database,
    owner: SellerId,
    document_id: DocumentId,
    bytes: &[u8],
    now: &str,
) -> Result<String, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let invoice = syntax::read_document(text).map_err(|error| error.to_string())?.invoice;

    // The level is chosen from what the document carries, not from what its sender
    // claimed: this hybrid is our re-serialisation, so it has to claim what we
    // wrote. A document with lines and no VAT breakdown is already inconsistent,
    // and EN 16931 is the level it was claiming.
    let profile = FacturXProfile::carrying(!invoice.lines.is_empty(), !invoice.totals.breakdown.is_empty());
    let cii = write_received_cii(&invoice, profile).map_err(|error| error.to_string())?;

    let seller = store::load_seller(database.connection(), owner).map_err(|error| error.to_string())?;
    let language = DocumentLanguage::from_locale(&seller.default_locale);
    let data = received_data(&invoice, language);
    let hybrid = crate::renderer_for(None)
        .render_facturx(&data, &cii, profile.conformance_level())
        .map_err(|error| error.to_string())?;

    store::attach_artefact_with_profile(
        database.connection(),
        document_id,
        "facturx",
        Some(&profile.artefact_profile()),
        &hybrid,
        render::RENDERER_VERSION,
        None,
        now,
    )
    .map_err(|error| error.to_string())
}

/// The received terms as CII, at the level the document fills.
fn write_received_cii(invoice: &ImportedInvoice, profile: FacturXProfile) -> Result<Vec<u8>, syntax::CiiError> {
    let lines: Vec<InvoiceLine<'_>> = invoice
        .lines
        .iter()
        .map(|line| InvoiceLine {
            id: line.position,
            name: &line.name,
            description: line.description.as_deref(),
            quantity: line.quantity,
            unit: line.unit,
            unit_price: line.unit_price,
            base_quantity: line.base_quantity,
            line_net: line.line_net,
            treatment: &line.treatment,
        })
        .collect();
    // A credit note without the corrected invoice's date cannot be written as CII;
    // the writer says so, and the caller reports it rather than inventing a date.
    let preceding = invoice.preceding.as_ref().and_then(|preceding| {
        preceding.issue_date.map(|issue_date| InvoiceReference {
            number: &preceding.number,
            issue_date,
        })
    });
    let view = Invoice {
        number: &invoice.number,
        issue_date: invoice.issue_date,
        due_date: invoice.due_date,
        delivery_date: invoice.delivery_date,
        doc_type: invoice.doc_type,
        currency: invoice.currency,
        buyer_reference: invoice.buyer_reference.as_deref(),
        order_reference: invoice.order_reference.as_deref(),
        note: invoice.note.as_deref(),
        preceding_invoice: preceding,
        seller: &invoice.seller,
        customer: &invoice.customer,
        lines: &lines,
        totals: &invoice.totals,
    };
    write_cii_at(&view, profile).map(String::into_bytes)
}

/// What the page shows: the received terms, worded as a document of ours.
fn received_data(invoice: &ImportedInvoice, language: DocumentLanguage) -> render::InvoiceData {
    render::InvoiceData {
        title: language.document_title(invoice.doc_type.is_credit_note()).to_owned(),
        labels: language.labels(),
        number: invoice.number.clone(),
        issue_date: invoice.issue_date.to_iso_string(),
        due_date: invoice.due_date.map(crux::LocalDate::to_iso_string),
        currency: invoice.currency.to_string(),
        seller: party_data(&invoice.seller),
        customer: party_data(&invoice.customer),
        lines: invoice
            .lines
            .iter()
            .map(|line| render::LineData {
                name: line.name.clone(),
                qty: line.quantity.to_trimmed_string(),
                price: line.unit_price.to_trimmed_string(),
                vat: line.treatment.rate.percent_string(),
                total: line.line_net.to_fixed_string(2).unwrap_or_default(),
            })
            .collect(),
        net: invoice.totals.net.to_fixed_string(2).unwrap_or_default(),
        tax: invoice.totals.tax.to_fixed_string(2).unwrap_or_default(),
        gross: invoice.totals.gross.to_fixed_string(2).unwrap_or_default(),
        notes: invoice.note.clone(),
    }
}
