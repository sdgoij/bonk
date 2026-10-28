//! Import pipeline: sniffing, extraction, dedupe, supplier matching.
//!
//! The pipeline records what arrived; it does not decide what it means. A
//! received document becomes a `document` row with `direction = 'inbound'`, the
//! supplier is matched against the party table, and anything unmatched is kept
//! with `match_status = 'unmatched'` rather than invented as a customer.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![forbid(unsafe_code)]

mod pdf;
mod pipeline;
mod watch;

pub use pdf::{ExtractedInvoice, PdfError, extract_invoice};
pub use pipeline::{ImportError, ImportOutcome, best_match, import_bytes, import_file, match_supplier};
pub use watch::{Watcher, ready_now};

/// Crate identity, used by diagnostics and support bundles.
pub const CRATE_NAME: &str = "import";

/// What a file appears to be, decided before any parsing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SniffedKind {
    /// UBL invoice XML.
    Ubl,
    /// CII invoice XML.
    Cii,
    /// A PDF that may contain an embedded Factur-X document.
    Pdf,
    /// Unknown or unsupported.
    Unknown,
}

/// Classifies the first bytes of a file without trusting the extension.
pub fn sniff(prefix: &[u8]) -> SniffedKind {
    if prefix.starts_with(b"%PDF-") {
        return SniffedKind::Pdf;
    }
    let head = String::from_utf8_lossy(&prefix[..prefix.len().min(4096)]);
    let head = head.trim_start();
    if head.starts_with("<?xml") || head.starts_with('<') {
        if head.contains("CrossIndustryInvoice") {
            return SniffedKind::Cii;
        }
        if head.contains("Invoice") {
            return SniffedKind::Ubl;
        }
    }
    SniffedKind::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_pdf_magic() {
        assert_eq!(sniff(b"%PDF-1.7\n"), SniffedKind::Pdf);
    }

    #[test]
    fn sniffs_cii_and_ubl_roots() {
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><rsm:CrossIndustryInvoice"),
            SniffedKind::Cii
        );
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><Invoice xmlns=\"urn:oasis:names:specification:ubl:schema:xsd:Invoice-2\">"),
            SniffedKind::Ubl
        );
    }

    #[test]
    fn refuses_to_guess() {
        assert_eq!(sniff(b"\x00\x01\x02"), SniffedKind::Unknown);
        assert_eq!(sniff(b""), SniffedKind::Unknown);
        assert_eq!(sniff(b"<html>"), SniffedKind::Unknown);
    }
}
