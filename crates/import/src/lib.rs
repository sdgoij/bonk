//! Import pipeline: sniffing, extraction, dedupe, supplier matching.
//!
//! The pipeline records what arrived; it does not decide what it means. A
//! received document becomes a `document` row with `direction = 'inbound'`, the
//! supplier is matched against the party table, and anything unmatched is kept
//! with `match_status = 'unmatched'` rather than invented as a customer.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![forbid(unsafe_code)]

pub mod bank_file;
pub mod camt;
pub mod csv;
pub mod mt940;
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
    /// A MT940 bank statement.
    Mt940,
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
    if looks_like_mt940(head) {
        return SniffedKind::Mt940;
    }
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

/// True when one of MT940's opening tags sits at the start of a line.
///
/// The tags are checked at line starts rather than anywhere in the head, because
/// `:20:` and friends also appear inside ordinary text and inside XML content.
fn looks_like_mt940(head: &str) -> bool {
    head.lines().take(8).any(|line| {
        let line = line.trim_start();
        line.starts_with(":20:") || line.starts_with(":25:") || line.starts_with(":60F:")
    })
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

    #[test]
    fn sniffs_mt940_tags() {
        assert_eq!(
            sniff(b":20:BONK-2026-02\n:25:NL91ABNA0417164300\n:60F:C260201EUR1,00\n"),
            SniffedKind::Mt940
        );
        // Framed by SWIFT blocks, the tags are still on their own lines.
        assert_eq!(
            sniff(b"{1:F01ABNANL2AXXX0000000000}{2:I940}{4:\n:60F:C260201EUR1,00\n-}"),
            SniffedKind::Mt940
        );
        // A tag-looking string inside a line is not a statement.
        assert_eq!(sniff(b"zie :20: in de omschrijving"), SniffedKind::Unknown);
    }
}
