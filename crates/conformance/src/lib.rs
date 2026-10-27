#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! EN 16931 semantic model, business rules, codelists, and profiles.
//!
//! M2 fills this crate. For M0 it exists so the dependency direction is real:
//! conformance may depend on the domain, never the reverse.

/// Crate identity, used by diagnostics and support bundles.
pub const CRATE_NAME: &str = "conformance";

/// Milestone that implements this crate.
pub const PLANNED_MILESTONE: &str = "M2";

#[cfg(test)]
mod tests {
    #[test]
    fn identity_is_stable() {
        assert_eq!(super::CRATE_NAME, "conformance");
        assert_eq!(super::PLANNED_MILESTONE, "M2");
    }
}
