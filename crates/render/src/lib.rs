#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! PDF rendering behind a port.
//!
//! M1 renders a visual invoice through the `typst` CLI (Typst 0.14). The layout
//! logic is a template over a JSON data file, so the same template can be
//! compiled by the embedded Typst library when PDF/A-3 work starts in M3, and
//! the CLI dependency disappears from the shipped binary.
//!
//! Artefacts are deterministic for the same input: the creation timestamp is
//! derived from the issue date, not from the wall clock.

#![forbid(unsafe_code)]

use serde::Serialize;
use thiserror::Error;

/// The invoice template, compiled into the binary.
pub const TEMPLATE: &str = include_str!("../templates/invoice.typ");

/// The data the template consumes. Strings are pre-formatted, so the template
/// stays free of business logic.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct InvoiceData {
    /// The word for the document itself: "Invoice", "Creditnota", and so on.
    pub title: String,
    /// The words for the fixed parts of the form.
    pub labels: Labels,
    /// Invoice number.
    pub number: String,
    /// ISO issue date.
    pub issue_date: String,
    /// ISO due date.
    pub due_date: Option<String>,
    /// Currency code.
    pub currency: String,
    /// Seller.
    pub seller: PartyData,
    /// Customer.
    pub customer: PartyData,
    /// Lines.
    pub lines: Vec<LineData>,
    /// Net total.
    pub net: String,
    /// Tax total.
    pub tax: String,
    /// Gross total.
    pub gross: String,
    /// Optional note.
    pub notes: Option<String>,
}

/// The language the printed document speaks.
///
/// Not the interface language: a Dutch seller may print an English invoice for a
/// customer abroad, and the interface speaks one language at a time.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Language {
    /// English labels.
    #[default]
    English,
    /// Dutch labels.
    Dutch,
}

impl Language {
    /// The words this language prints.
    pub const fn labels(self) -> Labels {
        match self {
            Language::English => Labels {
                due: "Due",
                vat: "VAT",
                bill_to: "Bill to",
                item: "Item",
                qty: "Qty",
                unit_price: "Unit price",
                total: "Total",
                net: "Net",
            },
            Language::Dutch => Labels {
                due: "Vervaldatum",
                vat: "BTW",
                bill_to: "Factuur aan",
                item: "Omschrijving",
                qty: "Aantal",
                unit_price: "Stukprijs",
                total: "Totaal",
                net: "Netto",
            },
        }
    }

    /// The word for the document, which a credit note does not share.
    pub const fn document_title(self, credit_note: bool) -> &'static str {
        match (self, credit_note) {
            (Language::English, false) => "Invoice",
            (Language::English, true) => "Credit note",
            (Language::Dutch, false) => "Factuur",
            (Language::Dutch, true) => "Creditnota",
        }
    }

    /// Infers the printed language from a locale tag; `nl` anywhere in the
    /// primary subtag means Dutch, anything else English.
    pub fn from_locale(locale: &str) -> Self {
        if locale.trim().to_ascii_lowercase().starts_with("nl") {
            Language::Dutch
        } else {
            Language::English
        }
    }
}

/// The words the template prints for the fixed parts of the form.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
pub struct Labels {
    /// Due date.
    pub due: &'static str,
    /// VAT identifier.
    pub vat: &'static str,
    /// Heading over the customer block.
    pub bill_to: &'static str,
    /// Item column.
    pub item: &'static str,
    /// Quantity column.
    pub qty: &'static str,
    /// Unit price column.
    pub unit_price: &'static str,
    /// Line total column.
    pub total: &'static str,
    /// Net total row.
    pub net: &'static str,
}

/// A party block in the template.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct PartyData {
    /// Legal name.
    pub name: String,
    /// Single-line address.
    pub address: String,
    /// VAT identifier.
    pub vat: Option<String>,
}

/// One invoice line.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct LineData {
    /// Item name.
    pub name: String,
    /// Quantity.
    pub qty: String,
    /// Unit price.
    pub price: String,
    /// VAT rate as a percentage.
    pub vat: String,
    /// Line net.
    pub total: String,
}

/// Rendering failures.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RenderError {
    /// The template did not compile, or Typst refused to export it.
    #[error("typst rejected the document: {0}")]
    Typst(String),
    /// The template produced no pages, so there is nothing to export.
    #[error("the template produced no pages")]
    NoPages,
    /// Encoding the raster preview failed.
    #[error("could not encode the preview: {0}")]
    Png(String),
    /// Serialising the template data failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// A capability that is not implemented yet.
    #[error("{feature} arrives in {milestone}")]
    NotYet {
        /// Capability name.
        feature: &'static str,
        /// Milestone that implements it.
        milestone: &'static str,
    },
}

/// A PDF backend.
pub trait PdfRenderer {
    /// Renderer name and version, recorded on the artefact.
    fn renderer_version(&self) -> &str;

    /// Renders the document.
    fn render(&self, data: &InvoiceData) -> Result<Vec<u8>, RenderError>;

    /// Renders the first page as a PNG, for a preview inside the window.
    fn render_png(&self, data: &InvoiceData, ppi: f32) -> Result<Vec<u8>, RenderError>;
}

/// The Typst version compiled into the binary, recorded on every artefact.
pub const RENDERER_VERSION: &str = "typst-0.15";

/// Renders with Typst as a library.
///
/// The compiler, the template, and the fonts it needs all live in the binary,
/// so issuing an invoice needs nothing installed on the machine and produces the
/// same bytes everywhere. Building the font database is the expensive part, so
/// one renderer is built per process and shared.
pub struct TypstRenderer {
    fonts: std::sync::Arc<embedded_typst::Fonts>,
}

impl TypstRenderer {
    /// Builds the renderer: Typst's bundled fonts, and nothing from the system.
    ///
    /// System fonts would make the same invoice render differently on two
    /// machines, which is exactly what an archive must not do.
    pub fn new() -> Self {
        Self {
            fonts: std::sync::Arc::new(embedded_typst::Fonts::builder().typst_fonts().build()),
        }
    }

    /// Compiles the template with the payload as the file `data.json`.
    ///
    /// `sys.inputs.data` names a file, exactly as the Typst CLI was handed it, so
    /// the template stays usable by hand and by an editor with no renderer in the
    /// loop.
    fn compile(&self, data: &InvoiceData) -> Result<typst_layout::PagedDocument, RenderError> {
        use typst::foundations::{Dict, Str, Value};
        use typst::syntax::VirtualPath;

        let main = VirtualPath::new("invoice.typ").map_err(|error| RenderError::Typst(error.to_string()))?;
        let payload = VirtualPath::new("data.json").map_err(|error| RenderError::Typst(error.to_string()))?;

        let world = embedded_typst::EmbeddedWorld::new(main, TEMPLATE, self.fonts.clone())
            .with_creation_timestamp(creation_timestamp(&data.issue_date));
        world.add_file(payload, serde_json::to_vec(data)?);

        let mut inputs = Dict::new();
        inputs.insert(Str::from("data"), Value::Str(Str::from("data.json")));
        let world = world.with_inputs(inputs);

        typst::compile::<typst_layout::PagedDocument>(&world)
            .output
            .map_err(|errors| RenderError::Typst(describe(&errors)))
    }
}

impl Default for TypstRenderer {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-wide renderer.
///
/// Building the font database is the expensive part, and it does not depend on
/// the document, so one instance serves every render in the process.
pub fn renderer() -> &'static TypstRenderer {
    static RENDERER: std::sync::OnceLock<TypstRenderer> = std::sync::OnceLock::new();
    RENDERER.get_or_init(TypstRenderer::new)
}

impl PdfRenderer for TypstRenderer {
    fn renderer_version(&self) -> &str {
        RENDERER_VERSION
    }

    fn render(&self, data: &InvoiceData) -> Result<Vec<u8>, RenderError> {
        let document = self.compile(data)?;
        typst_pdf::pdf(&document, &typst_pdf::PdfOptions::default())
            .map_err(|errors| RenderError::Typst(describe(&errors)))
    }

    fn render_png(&self, data: &InvoiceData, ppi: f32) -> Result<Vec<u8>, RenderError> {
        let document = self.compile(data)?;
        let page = document.pages().first().ok_or(RenderError::NoPages)?;
        let options = typst_render::RenderOptions {
            pixel_per_pt: typst::utils::Scalar::new(pixels_per_point(ppi)),
            render_bleed: false,
        };
        typst_render::render(page, &options)
            .encode_png()
            .map_err(|error| RenderError::Png(error.to_string()))
    }
}

/// Joins the diagnostics of a failed compilation into one sentence.
fn describe(errors: &[typst::diag::SourceDiagnostic]) -> String {
    let messages: Vec<String> = errors.iter().map(|error| error.message.to_string()).collect();
    if messages.is_empty() {
        "unknown error".to_owned()
    } else {
        messages.join("; ")
    }
}

/// Pixels per Typst point for a preview at `ppi`.
///
/// Typst measures in points, 1/72 inch. Rasterising is the one place in this
/// crate that needs floating point; it never touches an amount.
#[allow(clippy::float_arithmetic)]
fn pixels_per_point(ppi: f32) -> f64 {
    f64::from(ppi) / 72.0
}

/// Midnight UTC on the issue date, as a UNIX timestamp. Zero if unparseable.
fn creation_timestamp(issue_date: &str) -> i64 {
    use time::format_description::well_known::Iso8601;
    time::Date::parse(issue_date, &Iso8601::DATE)
        .ok()
        .and_then(|date| date.midnight().assume_utc().unix_timestamp().into())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> InvoiceData {
        InvoiceData {
            title: "Invoice".to_owned(),
            labels: Language::English.labels(),
            number: "2026-0001".to_owned(),
            issue_date: "2026-10-27".to_owned(),
            due_date: Some("2026-11-26".to_owned()),
            currency: "EUR".to_owned(),
            seller: PartyData {
                name: "Seller BV".to_owned(),
                address: "Main 1, Amsterdam, NL".to_owned(),
                vat: Some("NL123456782B01".to_owned()),
            },
            customer: PartyData {
                name: "Customer BV".to_owned(),
                address: "Rotterdam, NL".to_owned(),
                vat: None,
            },
            lines: vec![LineData {
                name: "Consulting".to_owned(),
                qty: "2".to_owned(),
                price: "50".to_owned(),
                vat: "21".to_owned(),
                total: "100.00".to_owned(),
            }],
            net: "100.00".to_owned(),
            tax: "21.00".to_owned(),
            gross: "121.00".to_owned(),
            notes: None,
        }
    }

    #[test]
    fn a_dutch_locale_prints_dutch() {
        assert_eq!(Language::from_locale("nl-NL"), Language::Dutch);
        assert_eq!(Language::from_locale("NL-nl"), Language::Dutch);
        assert_eq!(Language::from_locale("nl"), Language::Dutch);
        assert_eq!(Language::from_locale("en-GB"), Language::English);
        assert_eq!(Language::from_locale(""), Language::English);
    }

    #[test]
    fn labels_differ_between_languages() {
        let dutch = Language::Dutch.labels();
        let english = Language::English.labels();
        assert_eq!(dutch.vat, "BTW");
        assert_ne!(dutch.item, english.item);
        assert_eq!(dutch.net, "Netto");
        assert_eq!(english.net, "Net");
    }

    /// The eight bytes every PNG starts with, spelled out so no escape can be
    /// mangled on the way into this file.
    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

    #[test]
    fn renders_a_pdf() {
        let renderer = TypstRenderer::new();
        let pdf = renderer.render(&sample()).expect("render");
        assert!(pdf.starts_with(b"%PDF-"), "not a PDF");
        assert!(pdf.len() > 1_000, "suspiciously small PDF: {}", pdf.len());
        assert_eq!(renderer.renderer_version(), RENDERER_VERSION);
    }

    #[test]
    fn output_is_deterministic() {
        let renderer = TypstRenderer::new();
        let first = renderer.render(&sample()).expect("first render");
        let second = renderer.render(&sample()).expect("second render");
        assert_eq!(first, second, "PDF bytes must be stable for the same input");
    }

    #[test]
    fn the_same_invoice_printing_dutch_is_a_different_document() {
        let renderer = TypstRenderer::new();
        let english = renderer.render(&sample()).expect("english");
        let mut dutch = sample();
        dutch.labels = Language::Dutch.labels();
        dutch.title = Language::Dutch.document_title(false).to_owned();
        let dutch = renderer.render(&dutch).expect("dutch");
        assert_ne!(english, dutch, "the printed language reaches the page");
    }

    #[test]
    fn creation_timestamp_is_derived_from_the_issue_date() {
        // 2026-10-27T00:00:00Z
        assert_eq!(creation_timestamp("2026-10-27"), 1_793_059_200);
        assert_eq!(creation_timestamp("not a date"), 0);
    }

    #[test]
    fn previews_a_png() {
        let png = TypstRenderer::new().render_png(&sample(), 96.0).expect("preview");
        assert!(png.starts_with(&PNG_MAGIC), "not a PNG");
        assert!(png.len() > 1_000, "suspiciously small PNG: {}", png.len());
    }
}
