//! Vendored EN 16931 code lists.
//!
//! The bundles are extracted from the official codelist Schematron by
//! `cargo run -p xtask -- codelists` and committed under
//! `crates/conformance/codelists/<bundle>/`. Each bundle carries a manifest
//! with the upstream URL, the pinned source hash, and a SHA-256 per file. The
//! tests below recompute those hashes, so a corrupted or stale bundle fails
//! before it can ship; a release is a deliberate diff of the bundle directory
//! plus a bump of [`BUNDLE_VERSION`].
//!
//! Lookups are cheap: every list is parsed once, on first access.

use std::collections::BTreeSet;
use std::sync::OnceLock;

/// Version label of the embedded bundle.
pub const BUNDLE_VERSION: &str = "en16931-ubl-codes-2026-10";

/// A code list the rule engine consults.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(usize)]
pub enum Codelist {
    /// UN/TDID 1001 codes valid for an invoice (BR-CL-01).
    InvoiceTypeCode,
    /// UN/TDID 1001 codes valid for a credit note (BR-CL-01).
    CreditNoteTypeCode,
    /// ISO 4217 alpha-3 currency codes (BR-CL-03/04/05).
    CurrencyCode,
    /// ISO 3166-1 country codes (BR-CL-14/15).
    CountryCode,
    /// UNCL 5305 VAT category codes (BR-CL-17/18).
    VatCategoryCode,
    /// CEF VATEX exemption reason codes (BR-CL-22).
    ExemptionReasonCode,
    /// UN/ECE Recommendation 20 unit codes with Recommendation 21 extensions (BR-CL-23).
    UnitCode,
    /// CEF EAS endpoint scheme identifiers (BR-CL-25).
    EndpointSchemeCode,
}

impl Codelist {
    /// Every vendored list, in manifest order.
    pub const ALL: [Self; 8] = [
        Self::InvoiceTypeCode,
        Self::CreditNoteTypeCode,
        Self::CurrencyCode,
        Self::CountryCode,
        Self::VatCategoryCode,
        Self::ExemptionReasonCode,
        Self::UnitCode,
        Self::EndpointSchemeCode,
    ];

    /// The bundle version this list was extracted from.
    pub const fn version(self) -> &'static str {
        BUNDLE_VERSION
    }

    /// The bundle file name, without its directory.
    pub const fn file(self) -> &'static str {
        self.bundle().file
    }

    /// The sorted entries.
    pub fn entries(self) -> &'static BTreeSet<&'static str> {
        self.bundle().entries()
    }

    /// True when `code` is in the list.
    ///
    /// Comparisons are byte-exact and case-sensitive; the official rules only
    /// fold case where they say so (see [`Self::contains_uppercase`]).
    pub fn contains(self, code: &str) -> bool {
        self.entries().contains(code)
    }

    /// True when `code` is in the list after ASCII upper-casing.
    ///
    /// BR-CL-22 normalises with `upper-case()` before the lookup.
    pub fn contains_uppercase(self, code: &str) -> bool {
        self.contains(&code.to_ascii_uppercase())
    }

    const fn bundle(self) -> &'static Bundle {
        &BUNDLES[self as usize]
    }
}

struct Bundle {
    file: &'static str,
    raw: &'static str,
    entries: OnceLock<BTreeSet<&'static str>>,
}

impl Bundle {
    const fn new(file: &'static str, raw: &'static str) -> Self {
        Self {
            file,
            raw,
            entries: OnceLock::new(),
        }
    }

    fn entries(&self) -> &BTreeSet<&'static str> {
        self.entries.get_or_init(|| {
            self.raw
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect()
        })
    }
}

/// The bundle, indexed by [`Codelist`] discriminant.
static BUNDLES: [Bundle; 8] = [
    Bundle::new(
        "untdid-1001-invoice.txt",
        include_str!("../codelists/en16931-ubl-codes-2026-10/untdid-1001-invoice.txt"),
    ),
    Bundle::new(
        "untdid-1001-credit-note.txt",
        include_str!("../codelists/en16931-ubl-codes-2026-10/untdid-1001-credit-note.txt"),
    ),
    Bundle::new(
        "iso-4217.txt",
        include_str!("../codelists/en16931-ubl-codes-2026-10/iso-4217.txt"),
    ),
    Bundle::new(
        "iso-3166-1.txt",
        include_str!("../codelists/en16931-ubl-codes-2026-10/iso-3166-1.txt"),
    ),
    Bundle::new(
        "uncl5305.txt",
        include_str!("../codelists/en16931-ubl-codes-2026-10/uncl5305.txt"),
    ),
    Bundle::new(
        "vatex.txt",
        include_str!("../codelists/en16931-ubl-codes-2026-10/vatex.txt"),
    ),
    Bundle::new(
        "un-ece-rec20-21.txt",
        include_str!("../codelists/en16931-ubl-codes-2026-10/un-ece-rec20-21.txt"),
    ),
    Bundle::new(
        "eas.txt",
        include_str!("../codelists/en16931-ubl-codes-2026-10/eas.txt"),
    ),
];

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use crux::{DocType, UnitCode, VatCategory};

    use super::*;

    const MANIFEST: &str = include_str!("../codelists/en16931-ubl-codes-2026-10/manifest.json");

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn manifest() -> serde_json::Value {
        serde_json::from_str(MANIFEST).expect("the bundle manifest is valid JSON")
    }

    #[test]
    fn the_manifest_and_the_embedded_bundles_agree() {
        let manifest = manifest();
        assert_eq!(manifest["bundle"].as_str(), Some(BUNDLE_VERSION));

        let files = manifest["files"].as_array().expect("files is an array");
        assert_eq!(files.len(), Codelist::ALL.len(), "manifest lists every bundle");

        for (codelist, entry) in Codelist::ALL.iter().zip(files) {
            assert_eq!(entry["name"].as_str(), Some(codelist.file()));
            assert_eq!(
                entry["sha256"].as_str(),
                Some(sha256_hex(codelist.bundle().raw.as_bytes()).as_str()),
                "{} does not match its manifest hash",
                codelist.file()
            );
            assert_eq!(
                entry["entries"].as_u64(),
                Some(codelist.entries().len() as u64),
                "{} does not match its manifest entry count",
                codelist.file()
            );
        }
    }

    #[test]
    fn the_manifest_pins_a_source() {
        let manifest = manifest();
        let source = &manifest["source"];
        assert!(source["url"].as_str().is_some_and(|url| url.starts_with("https://")));
        assert!(
            source["sha256"]
                .as_str()
                .is_some_and(|hash| hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
        );
    }

    #[test]
    fn no_bundle_is_empty() {
        for codelist in Codelist::ALL {
            assert!(!codelist.entries().is_empty(), "{} is empty", codelist.file());
        }
    }

    #[test]
    fn lookups_are_exact() {
        assert!(Codelist::CurrencyCode.contains("EUR"));
        assert!(!Codelist::CurrencyCode.contains("EURO"));
        assert!(!Codelist::CurrencyCode.contains("eur"));
        assert!(Codelist::CountryCode.contains("NL"));
        assert!(!Codelist::CountryCode.contains("XX"));
        assert!(Codelist::UnitCode.contains("C62"));
        assert!(!Codelist::UnitCode.contains("XXX"));
    }

    #[test]
    fn exemption_reason_codes_fold_case() {
        assert!(Codelist::ExemptionReasonCode.contains_uppercase("vatex-eu-79-c"));
        assert!(!Codelist::ExemptionReasonCode.contains_uppercase("VATEX-XX-1"));
    }

    #[test]
    fn every_domain_doc_type_is_in_untdid_1001() {
        assert!(Codelist::InvoiceTypeCode.contains(DocType::Invoice.code()));
        assert!(Codelist::CreditNoteTypeCode.contains(DocType::CreditNote.code()));
    }

    #[test]
    fn every_domain_unit_is_in_recommendation_20() {
        for unit in [
            UnitCode::PIECE,
            UnitCode::HOUR,
            UnitCode::DAY,
            UnitCode::KILOGRAM,
            UnitCode::KILOMETRE,
        ] {
            assert!(
                Codelist::UnitCode.contains(unit.as_str()),
                "{} is missing",
                unit.as_str()
            );
        }
    }

    #[test]
    fn every_domain_vat_category_is_in_uncl5305() {
        for category in [
            VatCategory::Standard,
            VatCategory::ZeroRated,
            VatCategory::Exempt,
            VatCategory::ReverseCharge,
            VatCategory::IntraCommunity,
            VatCategory::Export,
            VatCategory::OutsideScope,
            VatCategory::CanaryIslands,
            VatCategory::CeutaMelilla,
        ] {
            assert!(
                Codelist::VatCategoryCode.contains(category.code()),
                "{} is missing from UNCL5305",
                category.code()
            );
        }
    }
}
