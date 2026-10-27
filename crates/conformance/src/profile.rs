//! Conformance profiles.
//!
//! A profile selects the extra rules and the specification identifier written
//! into BT-24. EN 16931 is the semantic baseline; Peppol BIS 3.0 is a CIUS on
//! top of the UBL binding.

use core::fmt;

/// An EN 16931 or Peppol conformance profile.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Profile {
    /// EN 16931-1 semantic model.
    En16931,
    /// Peppol BIS Billing 3.0.
    PeppolBis3,
}

impl Profile {
    /// The BT-24 specification identifier.
    pub const fn specification_identifier(self) -> &'static str {
        match self {
            Self::En16931 => "urn:cen.eu:en16931:2017",
            Self::PeppolBis3 => "urn:cen.eu:en16931:2017#compliant#urn:fdc:peppol.eu:2017:poacc:billing:3.0",
        }
    }

    /// Human-readable profile name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::En16931 => "EN 16931",
            Self::PeppolBis3 => "Peppol BIS Billing 3.0",
        }
    }

    /// True when this is the Peppol profile.
    pub const fn is_peppol(self) -> bool {
        matches!(self, Self::PeppolBis3)
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_match_the_published_profiles() {
        assert_eq!(Profile::En16931.specification_identifier(), "urn:cen.eu:en16931:2017");
        assert!(Profile::PeppolBis3.specification_identifier().contains("peppol.eu"));
        assert!(Profile::PeppolBis3.is_peppol());
        assert!(!Profile::En16931.is_peppol());
    }
}
