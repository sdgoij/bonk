#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Vendored official EN 16931 / Peppol Schematron and XSLT artefacts.
//!
//! The native XSLT binding lands with the M0 validation spike and stays behind
//! the `official-validation` feature so the default build has no C dependency.

/// Crate identity, used by diagnostics and support bundles.
pub const CRATE_NAME: &str = "conformance-official";

/// True when this build can execute the official artefacts.
pub const fn has_native_backend() -> bool {
    cfg!(feature = "official-validation")
}

#[cfg(test)]
mod tests {
    #[test]
    fn backend_flag_reflects_the_feature() {
        assert_eq!(super::has_native_backend(), cfg!(feature = "official-validation"));
    }
}
