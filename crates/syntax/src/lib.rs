#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Invoice syntax writers and readers.
//!
//! M1 provides the UBL 2.1 writer. The readers arrive with M3 and the CII
//! writer with M4. Writers are dumb serialisers over the domain view and must
//! be byte-deterministic; business rules live in `crux` and `conformance`.

pub mod read;
pub mod ubl;

pub use read::{ImportError, ImportedInvoice, ImportedLine, ImportedReference, read_invoice};
pub use ubl::{UblError, UblInvoice, UblLine, UblPrecedingInvoice, write_invoice};

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
