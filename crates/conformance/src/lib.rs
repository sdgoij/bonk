#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![forbid(unsafe_code)]
//! EN 16931 semantic model, business rules, codelists, and profiles.
//!
//! The dependency direction is deliberate: conformance may depend on the
//! domain, never the reverse. Storage, UI, and network code do not appear here.
//!
//! M2 starts with the high-frequency arithmetic and VAT-category rules. The
//! official Schematron/XSLT artefacts remain the export authority.

pub mod bt;
pub mod profile;
pub mod rule;
pub mod semantic;
pub mod validate;

pub use bt::{Bg, Bt};
pub use profile::Profile;
pub use rule::{CORE_RULES, CategoryRules, Rule, category_rules};
pub use semantic::SemanticInvoice;
pub use validate::validate;

/// Crate identity, used by diagnostics and support bundles.
pub const CRATE_NAME: &str = "conformance";
