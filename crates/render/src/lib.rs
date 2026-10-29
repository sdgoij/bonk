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
pub enum RenderError {
    /// The `typst` executable was not found.
    #[error("typst is not installed or not on PATH (set BONK_TYPST to override)")]
    ToolMissing,
    /// Typst rejected the document.
    #[error("typst failed: {0}")]
    Typst(String),
    /// Filesystem failure while staging the template.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
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
}

/// Renders through the Typst command-line tool.
#[derive(Clone, Debug)]
pub struct TypstCliRenderer {
    binary: String,
}

impl TypstCliRenderer {
    /// Creates a renderer using a specific binary path or name.
    pub fn new(binary: impl Into<String>) -> Self {
        Self { binary: binary.into() }
    }

    /// Uses `BONK_TYPST` when set, otherwise `typst` from `PATH`.
    pub fn from_environment() -> Self {
        let binary = std::env::var("BONK_TYPST").unwrap_or_else(|_| "typst".to_owned());
        Self::new(binary)
    }
}

impl Default for TypstCliRenderer {
    fn default() -> Self {
        Self::from_environment()
    }
}

impl PdfRenderer for TypstCliRenderer {
    fn renderer_version(&self) -> &str {
        "typst-cli-0.14"
    }

    fn render(&self, data: &InvoiceData) -> Result<Vec<u8>, RenderError> {
        render_pdf(&self.binary, data)
    }
}

impl TypstCliRenderer {
    /// Renders the first page as PNG, for a preview inside the window.
    ///
    /// The same template that produces the PDF is compiled with Typst's PNG
    /// export, so what the preview shows is what issuing writes; nothing here
    /// re-implements the layout.
    pub fn render_png(&self, data: &InvoiceData, ppi: f32) -> Result<Vec<u8>, RenderError> {
        render_png(&self.binary, data, ppi)
    }
}

/// Stages the template and data in a temporary directory and compiles them.
pub fn render_pdf(binary: &str, data: &InvoiceData) -> Result<Vec<u8>, RenderError> {
    let directory = tempfile::tempdir()?;
    let data_path = directory.path().join("data.json");
    let source_path = directory.path().join("invoice.typ");
    let output_path = directory.path().join("invoice.pdf");

    std::fs::write(&data_path, serde_json::to_vec(data)?)?;
    std::fs::write(&source_path, TEMPLATE)?;

    // A fixed timestamp keeps the PDF bytes stable for the same invoice.
    let timestamp = creation_timestamp(&data.issue_date);

    let result = std::process::Command::new(binary)
        .arg("compile")
        .arg("--root")
        .arg(directory.path())
        .arg("--creation-timestamp")
        .arg(timestamp.to_string())
        .arg("--input")
        .arg("data=data.json")
        .arg(&source_path)
        .arg(&output_path)
        .output();

    let result = match result {
        Ok(result) => result,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(RenderError::ToolMissing);
        }
        Err(error) => return Err(error.into()),
    };

    if !result.status.success() {
        return Err(RenderError::Typst(
            String::from_utf8_lossy(&result.stderr).trim().to_owned(),
        ));
    }

    Ok(std::fs::read(output_path)?)
}

/// Compiles the template to a PNG preview of its first page.
///
/// Typst writes one file per page when the output path contains `{p}`; an
/// invoice template is one page, so page 1 is the whole document.
pub fn render_png(binary: &str, data: &InvoiceData, ppi: f32) -> Result<Vec<u8>, RenderError> {
    let directory = tempfile::tempdir()?;
    let data_path = directory.path().join("data.json");
    let source_path = directory.path().join("invoice.typ");
    let output_path = directory.path().join("preview-{p}.png");

    std::fs::write(&data_path, serde_json::to_vec(data)?)?;
    std::fs::write(&source_path, TEMPLATE)?;

    let result = std::process::Command::new(binary)
        .arg("compile")
        .arg("--root")
        .arg(directory.path())
        .arg("--creation-timestamp")
        .arg(creation_timestamp(&data.issue_date).to_string())
        .arg("--input")
        .arg("data=data.json")
        .arg("--format")
        .arg("png")
        .arg("--ppi")
        .arg(format!("{ppi}"))
        .arg(&source_path)
        .arg(&output_path)
        .output();

    let result = match result {
        Ok(result) => result,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(RenderError::ToolMissing);
        }
        Err(error) => return Err(error.into()),
    };

    if !result.status.success() {
        return Err(RenderError::Typst(
            String::from_utf8_lossy(&result.stderr).trim().to_owned(),
        ));
    }

    Ok(std::fs::read(directory.path().join("preview-1.png"))?)
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

    #[test]
    fn template_compiles_without_external_data_for_editors() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("invoice.typ");
        let output = directory.path().join("preview.pdf");
        std::fs::write(&source, TEMPLATE).unwrap();

        let result = std::process::Command::new("typst")
            .arg("compile")
            .arg("--root")
            .arg(directory.path())
            .arg(&source)
            .arg(&output)
            .output();

        let result = match result {
            Ok(result) => result,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("skipping: typst is not installed");
                return;
            }
            Err(error) => panic!("typst failed to start: {error}"),
        };
        assert!(
            result.status.success(),
            "preview compile failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(std::fs::read(&output).unwrap().starts_with(b"%PDF-"));
    }

    #[test]
    fn renders_a_pdf_when_typst_is_installed() {
        match render_pdf("typst", &sample()) {
            Ok(bytes) => {
                assert!(bytes.starts_with(b"%PDF-"), "not a PDF");
                assert!(bytes.len() > 1_000, "suspiciously small PDF: {}", bytes.len());
            }
            Err(RenderError::ToolMissing) => {
                eprintln!("skipping: typst is not installed");
            }
            Err(other) => panic!("render failed: {other}"),
        }
    }

    #[test]
    fn output_is_deterministic_when_typst_is_installed() {
        let first = match render_pdf("typst", &sample()) {
            Ok(bytes) => bytes,
            Err(RenderError::ToolMissing) => {
                eprintln!("skipping: typst is not installed");
                return;
            }
            Err(other) => panic!("render failed: {other}"),
        };
        let second = render_pdf("typst", &sample()).expect("second render");
        assert_eq!(first, second, "PDF bytes must be stable for the same input");
    }

    #[test]
    fn creation_timestamp_is_derived_from_the_issue_date() {
        // 2026-10-27T00:00:00Z
        assert_eq!(creation_timestamp("2026-10-27"), 1_793_059_200);
        assert_eq!(creation_timestamp("not a date"), 0);
    }

    #[test]
    fn missing_tool_is_reported_clearly() {
        let error = render_pdf("definitely-not-typst", &sample()).unwrap_err();
        assert!(matches!(error, RenderError::ToolMissing));
    }

    #[test]
    fn previews_a_png_when_typst_is_installed() {
        match render_png("typst", &sample(), 96.0) {
            Ok(bytes) => {
                assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
                assert!(bytes.len() > 1_000, "suspiciously small PNG: {}", bytes.len());
            }
            Err(RenderError::ToolMissing) => {
                eprintln!("skipping: typst is not installed");
            }
            Err(other) => panic!("preview failed: {other}"),
        }
    }
}
