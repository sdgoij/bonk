#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Invoice documents: layout and PDF.
//!
//! A template describes the document — page, type, sections, table columns,
//! labels, and a footer — as data. [`layout`] measures and paginates it into a
//! [`Document`] of draw operations in points; [`pdf`](mod@pdf) writes that model
//! with `krilla`, and the window draws the same model with egui for the
//! preview. One layout, two backends, so what is previewed is what is issued.
//!
//! Artefacts are deterministic for the same input: fonts are compiled into the
//! binary, and the creation date comes from the issue date, never the clock.

#![forbid(unsafe_code)]

pub mod layout;
pub mod pdf;
pub mod template;
mod text;

use serde::Serialize;
use thiserror::Error;

pub use layout::{Colour, Document, Op, Page, Point};
pub use template::Template;

/// The renderer name and version, recorded on every artefact.
pub const RENDERER_VERSION: &str = "bonk-layout-1";

/// The data the document shows. Strings are pre-formatted, so no business
/// logic — and no floating-point money — reaches the renderer.
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

/// The words the document prints for the fixed parts of the form.
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

/// A party block in the document.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct PartyData {
    /// Legal name.
    pub name: String,
    /// Single-line address.
    pub address: String,
    /// Dutch chamber of commerce number, for footers.
    pub kvk: Option<String>,
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
    /// A template field is not valid.
    #[error("template field {field}: {message}")]
    Template {
        /// Where in the template the problem is.
        field: String,
        /// What is wrong.
        message: String,
    },
    /// The bundled font could not be read.
    #[error("the bundled font could not be read")]
    Font,
    /// The PDF writer refused the document.
    #[error("the PDF writer refused the document: {0}")]
    Pdf(String),
}

/// A document backend.
pub trait PdfRenderer {
    /// Renderer name and version, recorded on the artefact.
    fn renderer_version(&self) -> &str;

    /// Lays the invoice out, for the PDF and for the window preview alike.
    fn layout(&self, data: &InvoiceData) -> Result<Document, RenderError>;

    /// Renders the document as PDF bytes.
    fn render(&self, data: &InvoiceData) -> Result<Vec<u8>, RenderError>;
}

/// Renders with the bundled layout engine and `krilla`.
///
/// The template, the fonts, and the text shaping all live in the binary, so
/// issuing an invoice needs nothing installed on the machine and produces the
/// same bytes everywhere.
pub struct Renderer {
    template: Template,
}

impl Renderer {
    /// Builds the renderer with the built-in template.
    pub fn new() -> Self {
        Self {
            template: Template::default(),
        }
    }

    /// Builds the renderer with a specific template. The template is validated
    /// when a document is laid out, so a bad one is an error, not a panic.
    pub fn with_template(template: Template) -> Self {
        Self { template }
    }

    /// The template this renderer draws.
    pub fn template(&self) -> &Template {
        &self.template
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-wide renderer.
pub fn renderer() -> &'static Renderer {
    static RENDERER: std::sync::OnceLock<Renderer> = std::sync::OnceLock::new();
    RENDERER.get_or_init(Renderer::new)
}

impl PdfRenderer for Renderer {
    fn renderer_version(&self) -> &str {
        RENDERER_VERSION
    }

    fn layout(&self, data: &InvoiceData) -> Result<Document, RenderError> {
        layout::layout(&self.template, data)
    }

    fn render(&self, data: &InvoiceData) -> Result<Vec<u8>, RenderError> {
        pdf::write(&self.layout(data)?)
    }
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
                kvk: Some("12345678".to_owned()),
                vat: Some("NL123456782B01".to_owned()),
            },
            customer: PartyData {
                name: "Customer BV".to_owned(),
                address: "Rotterdam, NL".to_owned(),
                kvk: None,
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

    #[test]
    fn renders_a_pdf() {
        let renderer = Renderer::new();
        let pdf = renderer.render(&sample()).expect("render");
        assert!(pdf.starts_with(b"%PDF-"), "not a PDF");
        assert!(pdf.len() > 1_000, "suspiciously small PDF: {}", pdf.len());
        assert_eq!(renderer.renderer_version(), RENDERER_VERSION);
    }

    #[test]
    fn output_is_deterministic() {
        let renderer = Renderer::new();
        let first = renderer.render(&sample()).expect("first render");
        let second = renderer.render(&sample()).expect("second render");
        assert_eq!(first, second, "PDF bytes must be stable for the same input");
    }

    #[test]
    fn the_same_invoice_printing_dutch_is_a_different_document() {
        let renderer = Renderer::new();
        let english = renderer.render(&sample()).expect("english");
        let mut dutch = sample();
        dutch.labels = Language::Dutch.labels();
        dutch.title = Language::Dutch.document_title(false).to_owned();
        let dutch = renderer.render(&dutch).expect("dutch");
        assert_ne!(english, dutch, "the printed language reaches the page");
    }

    #[test]
    fn the_layout_is_the_same_model_the_pdf_is_written_from() {
        let renderer = Renderer::new();
        let document = renderer.layout(&sample()).expect("layout");
        assert_eq!(document.pages.len(), 1);
        assert!(
            document.pages[0]
                .ops
                .iter()
                .any(|op| matches!(op, Op::Text { text, .. } if text == "Consulting"))
        );
    }

    #[test]
    fn a_template_field_error_names_the_field() {
        let error = template::Template::parse(r#"{"version": 2}"#).unwrap_err();
        assert!(error.to_string().contains("version"), "{error}");
    }
}
