#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Invoice syntax writers and readers.
//!
//! M1 provides the UBL 2.1 writer, M3 the readers, and M4 the CII writer that
//! Factur-X carries. Writers are dumb serialisers over [`Invoice`] and must be
//! byte-deterministic; business rules live in `crux` and `conformance`.

pub mod cii;
mod dom;
mod invoice;
pub mod read;
pub mod ubl;
mod xml;

pub use cii::{CiiError, write_invoice as write_cii, write_invoice_at as write_cii_at};
pub use invoice::{Invoice, InvoiceLine, InvoiceReference};
pub use read::{
    ImportError, ImportedDocument, ImportedInvoice, ImportedLine, ImportedReference, read_document, read_invoice,
};
pub use ubl::{UblError, write_invoice as write_ubl};

/// Crate identity, used by diagnostics and support bundles.
pub const CRATE_NAME: &str = "syntax";

/// Syntaxes this crate will support.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Syntax {
    /// OASIS UBL 2.1.
    Ubl21,
    /// UN/CEFACT Cross Industry Invoice D16B.
    CiiD16B,
}

#[cfg(test)]
mod tests {
    #[test]
    fn syntaxes_are_distinct() {
        assert_ne!(super::Syntax::Ubl21, super::Syntax::CiiD16B);
    }
}
