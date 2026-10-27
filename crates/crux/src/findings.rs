//! Validation findings.
//!
//! A finding always carries a stable code, the EN 16931 business term it
//! belongs to, and a field path the UI can focus. "Invalid invoice" without a
//! pointer is a support ticket; this type exists to prevent that.

/// How badly a finding hurts.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    /// Issuing is refused.
    Blocking,
    /// Issuing is allowed, but the result is probably wrong.
    Warning,
    /// Context only.
    Info,
}

/// One problem found in a draft.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Finding {
    /// Severity.
    pub severity: Severity,
    /// Stable code, e.g. `T1-003` today and `BR-16` later.
    pub code: &'static str,
    /// EN 16931 business term, e.g. `BT-129`.
    pub bt: Option<&'static str>,
    /// Path the UI focuses, e.g. `lines[2].quantity`.
    pub field: String,
    /// Human-readable message (English for now; localisation lands in M5).
    pub message: String,
}

impl Finding {
    /// A blocking finding.
    pub fn blocking(
        code: &'static str,
        bt: Option<&'static str>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Blocking,
            code,
            bt,
            field: field.into(),
            message: message.into(),
        }
    }

    /// A warning.
    pub fn warning(
        code: &'static str,
        bt: Option<&'static str>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            bt,
            field: field.into(),
            message: message.into(),
        }
    }
}

/// True when any finding blocks issuing.
pub fn has_blocking(findings: &[Finding]) -> bool {
    findings.iter().any(|finding| finding.severity == Severity::Blocking)
}
