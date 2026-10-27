#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Official EN 16931 conformance artefacts and the oracle that executes them.
//!
//! The EN 16931 rules are defined by Schematron/XSLT published by CEN and
//! OpenPeppol. Re-implementing them by hand is how a product confidently rejects
//! valid invoices, so they stay the authority: [`report`] turns the official
//! validator's output into typed findings, and [`oracle`] runs that validator.
//!
//! The published validation XSLT is **XSLT 2.0** (`xsl:function`,
//! `for-each-group`). libxslt implements XSLT 1.0 only, so a small native
//! binding is not a viable path; the oracle runs the Java validator that bundles
//! the artefacts. See `spikes/validation.md` and ADR-006.

#![forbid(unsafe_code)]

pub mod report;

#[cfg(feature = "official-validation")]
pub mod oracle;

pub use report::{Failure, Report, ReportError, parse_report};

/// Crate identity, used by diagnostics and support bundles.
pub const CRATE_NAME: &str = "conformance-official";

/// True when this build can execute the official artefacts.
pub const fn has_official_runner() -> bool {
    cfg!(feature = "official-validation")
}

#[cfg(test)]
mod tests {
    #[test]
    fn runner_flag_reflects_the_feature() {
        assert_eq!(super::has_official_runner(), cfg!(feature = "official-validation"));
    }
}
