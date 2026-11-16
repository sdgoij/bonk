//! Factur-X conformance levels.
//!
//! A hybrid invoice claims a level in two places that have to agree: the CII
//! payload names it in BT-24, and the PDF's XMP packet names it again for the
//! reader that only opens the container. The level also decides what the payload
//! may contain, because the five levels are a ladder — each carries everything
//! the one below it carries, and more.
//!
//! MINIMUM and BASIC WL are not EN 16931 documents. They leave out the lines
//! (MINIMUM leaves out the document-level VAT breakdown too), so a bookkeeping
//! system that only needs to post an amount can read them without having to
//! understand a full invoice. BASIC, EN 16931 and EXTENDED are EN 16931, in that
//! order of detail.

use std::fmt;

/// A Factur-X (ZUGFeRD 2) conformance level.
///
/// Ordered from the least to the most detailed, so `profile >= Basic` asks
/// "does this level carry the lines?" without a second question.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum FacturXProfile {
    /// Amounts, parties, and dates: no lines, no VAT breakdown.
    Minimum,
    /// Adds addresses, the VAT breakdown, and payment terms.
    BasicWl,
    /// Adds the lines, contacts, and the delivery date. EN 16931 compliant.
    Basic,
    /// The full EN 16931 model; what a Peppol invoice also contains.
    En16931,
    /// EN 16931 plus the fields the standard leaves to extensions.
    Extended,
}

impl FacturXProfile {
    /// Every level, least detailed first.
    pub const ALL: [Self; 5] = [Self::Minimum, Self::BasicWl, Self::Basic, Self::En16931, Self::Extended];

    /// The level this crate writes when nobody chose one.
    ///
    /// EN 16931 is the level the archive already validates against and the one a
    /// counterparty can use without knowing which profile it asked for.
    pub const DEFAULT: Self = Self::En16931;

    /// BT-24: the specification identifier written into the CII payload.
    ///
    /// For the two levels that are not EN 16931 documents this is a Factur-X URN;
    /// for the other three it is EN 16931, restricted or extended.
    pub const fn guideline(self) -> &'static str {
        match self {
            Self::Minimum => "urn:factur-x.eu:1p0:minimum",
            Self::BasicWl => "urn:factur-x.eu:1p0:basicwl",
            Self::Basic => "urn:cen.eu:en16931:2017#compliant#urn:factur-x.eu:1p0:basic",
            Self::En16931 => "urn:cen.eu:en16931:2017",
            Self::Extended => "urn:cen.eu:en16931:2017#conformant#urn:factur-x.eu:1p0:extended",
        }
    }

    /// The level name the PDF's XMP packet must carry.
    ///
    /// Fixed by the Factur-X specification, spaces and case included: a reader
    /// compares this string to decide what it was handed.
    pub const fn conformance_level(self) -> &'static str {
        match self {
            Self::Minimum => "MINIMUM",
            Self::BasicWl => "BASIC WL",
            Self::Basic => "BASIC",
            Self::En16931 => "EN 16931",
            Self::Extended => "EXTENDED",
        }
    }

    /// The short name used on the command line and on an artefact row.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Minimum => "minimum",
            Self::BasicWl => "basic-wl",
            Self::Basic => "basic",
            Self::En16931 => "en16931",
            Self::Extended => "extended",
        }
    }

    /// The profile named on an artefact row, e.g. `factur-x:basic-wl`.
    pub fn artefact_profile(self) -> String {
        format!("factur-x:{}", self.code())
    }

    /// Reads a level from its [`code`](Self::code), case-insensitively.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        let normalised = value.replace([' ', '_'], "-");
        Self::ALL.into_iter().find(|profile| profile.code() == normalised)
    }

    /// The most detailed level a document's content can fill.
    ///
    /// Used for a document we did not write: re-serialising a received invoice at
    /// a level above what it carries would make the payload claim content it does
    /// not have, which is exactly what the level ladder exists to prevent.
    pub const fn carrying(lines: bool, vat_breakdown: bool) -> Self {
        match (lines, vat_breakdown) {
            (true, _) => Self::En16931,
            (false, true) => Self::BasicWl,
            (false, false) => Self::Minimum,
        }
    }

    /// True when the payload carries the lines (BG-25).
    pub const fn carries_lines(self) -> bool {
        matches!(self, Self::Basic | Self::En16931 | Self::Extended)
    }

    /// True when the payload carries the document-level VAT breakdown (BG-23).
    pub const fn carries_vat_breakdown(self) -> bool {
        !matches!(self, Self::Minimum)
    }

    /// True when the payload carries the parts of an address beyond the country.
    ///
    /// MINIMUM names the parties and their countries and nothing else: a posting
    /// system that only needs to know who was paid does not need a street.
    pub const fn carries_address(self) -> bool {
        !matches!(self, Self::Minimum)
    }

    /// True when the payload carries contact details (BG-5, BG-6).
    pub const fn carries_contact(self) -> bool {
        matches!(self, Self::Basic | Self::En16931 | Self::Extended)
    }

    /// True when the payload carries the payment terms and the due date (BT-9).
    pub const fn carries_payment_terms(self) -> bool {
        !matches!(self, Self::Minimum)
    }

    /// True when the payload carries the delivery date (BT-72).
    pub const fn carries_delivery(self) -> bool {
        matches!(self, Self::Basic | Self::En16931 | Self::Extended)
    }

    /// True when the payload carries references: buyer reference, order, the
    /// seller's identifier, and the invoice a credit note corrects.
    pub const fn carries_references(self) -> bool {
        !matches!(self, Self::Minimum)
    }

    /// True when the payload carries the parties' electronic addresses (BT-34,
    /// BT-49), which are part of the EN 16931 model but of no lower Factur-X level.
    pub const fn carries_endpoint(self) -> bool {
        self.claims_en16931()
    }

    /// True when the payload carries the document note (BT-22).
    pub const fn carries_note(self) -> bool {
        !matches!(self, Self::Minimum)
    }

    /// True when the level is an EN 16931 document, so the full gate applies.
    pub const fn claims_en16931(self) -> bool {
        matches!(self, Self::Basic | Self::En16931 | Self::Extended)
    }

    /// True when the level cannot be written without the seller's VAT identifier.
    ///
    /// MINIMUM carries no lines and no VAT breakdown, so the seller's tax
    /// registration is the only thing that makes the document attributable.
    pub const fn requires_seller_vat(self) -> bool {
        matches!(self, Self::Minimum)
    }
}

impl fmt::Display for FacturXProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_level_names_itself_once() {
        let codes: HashSet<&str> = FacturXProfile::ALL.iter().map(|profile| profile.code()).collect();
        let levels: HashSet<&str> = FacturXProfile::ALL
            .iter()
            .map(|profile| profile.conformance_level())
            .collect();
        let guidelines: HashSet<&str> = FacturXProfile::ALL.iter().map(|profile| profile.guideline()).collect();
        assert_eq!(codes.len(), 5);
        assert_eq!(levels.len(), 5);
        assert_eq!(guidelines.len(), 5);
    }

    #[test]
    fn a_level_reads_back_from_its_code() {
        for profile in FacturXProfile::ALL {
            assert_eq!(FacturXProfile::parse(profile.code()), Some(profile), "{profile}");
            assert_eq!(
                FacturXProfile::parse(&profile.code().to_ascii_uppercase()),
                Some(profile),
                "the command line may shout"
            );
        }
        assert_eq!(FacturXProfile::parse("basic wl"), Some(FacturXProfile::BasicWl));
        assert_eq!(
            FacturXProfile::parse("COMFORT"),
            None,
            "ZUGFeRD 1 names are not Factur-X"
        );
        assert_eq!(FacturXProfile::parse(""), None);
    }

    #[test]
    fn the_ladder_adds_rather_than_swaps() {
        // Every level carries what the one below it carries.
        for pair in FacturXProfile::ALL.windows(2) {
            let (lower, higher) = (pair[0], pair[1]);
            assert!(lower < higher);
            if lower.carries_lines() {
                assert!(higher.carries_lines(), "{higher} drops the lines {lower} had");
            }
            if lower.carries_vat_breakdown() {
                assert!(higher.carries_vat_breakdown(), "{higher} drops the VAT breakdown");
            }
            if lower.carries_address() {
                assert!(higher.carries_address(), "{higher} drops the address");
            }
            if lower.carries_contact() {
                assert!(higher.carries_contact(), "{higher} drops the contact");
            }
            if lower.carries_payment_terms() {
                assert!(higher.carries_payment_terms(), "{higher} drops the payment terms");
            }
            if lower.carries_delivery() {
                assert!(higher.carries_delivery(), "{higher} drops the delivery date");
            }
            if lower.carries_references() {
                assert!(higher.carries_references(), "{higher} drops the references");
            }
            if lower.carries_note() {
                assert!(higher.carries_note(), "{higher} drops the note");
            }
            if lower.claims_en16931() {
                assert!(higher.claims_en16931(), "{higher} stops being EN 16931");
            }
        }
    }

    #[test]
    fn only_the_en16931_levels_carry_lines() {
        for profile in FacturXProfile::ALL {
            assert_eq!(profile.carries_lines(), profile.claims_en16931(), "{profile}");
        }
    }

    #[test]
    fn the_en16931_level_is_the_guideline_peppol_also_uses() {
        assert_eq!(FacturXProfile::En16931.guideline(), "urn:cen.eu:en16931:2017");
        assert_eq!(
            FacturXProfile::Basic.guideline(),
            "urn:cen.eu:en16931:2017#compliant#urn:factur-x.eu:1p0:basic"
        );
    }

    #[test]
    fn a_document_is_re_serialised_at_the_level_its_content_fills() {
        assert_eq!(FacturXProfile::carrying(true, true), FacturXProfile::En16931);
        assert_eq!(FacturXProfile::carrying(false, true), FacturXProfile::BasicWl);
        assert_eq!(FacturXProfile::carrying(false, false), FacturXProfile::Minimum);
        // The level chosen always carries what was asked for.
        for (lines, breakdown) in [(true, true), (false, true), (false, false)] {
            let profile = FacturXProfile::carrying(lines, breakdown);
            assert_eq!(profile.carries_lines(), lines, "{profile}");
            assert_eq!(profile.carries_vat_breakdown(), breakdown, "{profile}");
        }
    }

    #[test]
    fn an_artefact_row_names_the_level_it_was_written_at() {
        assert_eq!(FacturXProfile::BasicWl.artefact_profile(), "factur-x:basic-wl");
        assert_eq!(FacturXProfile::DEFAULT.artefact_profile(), "factur-x:en16931");
    }
}
