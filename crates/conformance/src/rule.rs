//! The rule catalogue used by the conformance engine.
//!
//! Codes and BT references come from the official EN 16931 UBL Schematron.
//! Keeping the catalogue static makes rules discoverable from findings and
//! prevents ad-hoc codes from leaking into the UI.

use crux::{Finding, Severity, VatCategory};

use crate::bt::{Bg, Bt};

/// Metadata for one business rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rule {
    /// Stable rule code, e.g. `BR-CO-10`.
    pub code: &'static str,
    /// Business term the finding should focus, when there is one.
    pub bt: Option<Bt>,
    /// Business group the rule applies to.
    pub group: Option<Bg>,
    /// How the rule affects issuing.
    pub severity: Severity,
    /// Short description for diagnostics and support bundles.
    pub summary: &'static str,
}

impl Rule {
    /// Builds a finding with this rule's code, BT reference, and severity.
    pub fn finding(&self, field: impl Into<String>, message: impl Into<String>) -> Finding {
        Finding {
            severity: self.severity,
            code: self.code,
            bt: self.bt.map(Bt::code),
            field: field.into(),
            message: message.into(),
        }
    }
}

macro_rules! rule {
    ($name:ident, $code:literal, $bt:expr, $group:expr, $summary:literal) => {
        /// Rule metadata.
        pub static $name: Rule = Rule {
            code: $code,
            bt: $bt,
            group: $group,
            severity: Severity::Blocking,
            summary: $summary,
        };
    };
}

rule!(
    BR_02,
    "BR-02",
    Some(Bt::InvoiceNumber),
    None,
    "An invoice shall have an invoice number."
);
rule!(
    BR_16,
    "BR-16",
    None,
    Some(Bg::InvoiceLine),
    "An invoice shall have at least one invoice line."
);
rule!(
    BR_CO_10,
    "BR-CO-10",
    Some(Bt::SumLineNet),
    Some(Bg::DocumentTotals),
    "Sum of invoice line net amounts equals the sum of line net amounts."
);
rule!(
    BR_CO_13,
    "BR-CO-13",
    Some(Bt::TaxExclusiveAmount),
    Some(Bg::DocumentTotals),
    "Invoice total without VAT equals line net amounts minus allowances plus charges."
);
rule!(
    BR_CO_14,
    "BR-CO-14",
    Some(Bt::TaxTotal),
    Some(Bg::DocumentTotals),
    "Invoice total VAT amount equals the sum of VAT category tax amounts."
);
rule!(
    BR_CO_15,
    "BR-CO-15",
    Some(Bt::TaxInclusiveAmount),
    Some(Bg::DocumentTotals),
    "Invoice total with VAT equals total without VAT plus total VAT."
);
rule!(
    BR_CO_16,
    "BR-CO-16",
    Some(Bt::AmountDue),
    Some(Bg::DocumentTotals),
    "Amount due equals total with VAT minus paid amount plus rounding."
);
rule!(
    BR_CO_17,
    "BR-CO-17",
    Some(Bt::VatTaxAmount),
    Some(Bg::VatBreakdown),
    "VAT category tax equals taxable amount times rate, rounded to two decimals."
);
rule!(
    BR_CO_26,
    "BR-CO-26",
    Some(Bt::SellerVatId),
    Some(Bg::Seller),
    "The seller has a seller identifier, legal registration identifier, or VAT identifier."
);

rule!(
    BR_S_01,
    "BR-S-01",
    Some(Bt::VatCategoryCode),
    Some(Bg::VatBreakdown),
    "A standard-rated line requires a standard-rated VAT breakdown."
);
rule!(
    BR_S_05,
    "BR-S-05",
    Some(Bt::ItemVatRate),
    Some(Bg::InvoiceLine),
    "A standard-rated line has a VAT rate greater than zero."
);
rule!(
    BR_S_08,
    "BR-S-08",
    Some(Bt::VatTaxableAmount),
    Some(Bg::VatBreakdown),
    "The standard-rated taxable amount equals the matching line net amounts."
);
rule!(
    BR_S_09,
    "BR-S-09",
    Some(Bt::VatTaxAmount),
    Some(Bg::VatBreakdown),
    "The standard-rated tax equals the taxable amount times the category rate."
);
rule!(
    BR_S_10,
    "BR-S-10",
    Some(Bt::VatExemptionText),
    Some(Bg::VatBreakdown),
    "A standard-rated VAT breakdown has no exemption reason."
);

rule!(
    BR_Z_01,
    "BR-Z-01",
    Some(Bt::VatCategoryCode),
    Some(Bg::VatBreakdown),
    "A zero-rated line requires a zero-rated VAT breakdown."
);
rule!(
    BR_Z_05,
    "BR-Z-05",
    Some(Bt::ItemVatRate),
    Some(Bg::InvoiceLine),
    "A zero-rated line has a VAT rate of zero."
);
rule!(
    BR_Z_08,
    "BR-Z-08",
    Some(Bt::VatTaxableAmount),
    Some(Bg::VatBreakdown),
    "The zero-rated taxable amount equals the matching line net amounts."
);
rule!(
    BR_Z_09,
    "BR-Z-09",
    Some(Bt::VatTaxAmount),
    Some(Bg::VatBreakdown),
    "The zero-rated tax amount is zero."
);
rule!(
    BR_Z_10,
    "BR-Z-10",
    Some(Bt::VatExemptionText),
    Some(Bg::VatBreakdown),
    "A zero-rated VAT breakdown has no exemption reason."
);

rule!(
    BR_E_01,
    "BR-E-01",
    Some(Bt::VatCategoryCode),
    Some(Bg::VatBreakdown),
    "An exempt line requires an exempt VAT breakdown."
);
rule!(
    BR_E_05,
    "BR-E-05",
    Some(Bt::ItemVatRate),
    Some(Bg::InvoiceLine),
    "An exempt line has a VAT rate of zero."
);
rule!(
    BR_E_08,
    "BR-E-08",
    Some(Bt::VatTaxableAmount),
    Some(Bg::VatBreakdown),
    "The exempt taxable amount equals the matching line net amounts."
);
rule!(
    BR_E_09,
    "BR-E-09",
    Some(Bt::VatTaxAmount),
    Some(Bg::VatBreakdown),
    "The exempt tax amount is zero."
);
rule!(
    BR_E_10,
    "BR-E-10",
    Some(Bt::VatExemptionText),
    Some(Bg::VatBreakdown),
    "An exempt VAT breakdown has an exemption reason."
);

rule!(
    BR_AE_01,
    "BR-AE-01",
    Some(Bt::VatCategoryCode),
    Some(Bg::VatBreakdown),
    "A reverse-charge line requires a reverse-charge VAT breakdown."
);
rule!(
    BR_AE_05,
    "BR-AE-05",
    Some(Bt::ItemVatRate),
    Some(Bg::InvoiceLine),
    "A reverse-charge line has a VAT rate of zero."
);
rule!(
    BR_AE_08,
    "BR-AE-08",
    Some(Bt::VatTaxableAmount),
    Some(Bg::VatBreakdown),
    "The reverse-charge taxable amount equals the matching line net amounts."
);
rule!(
    BR_AE_09,
    "BR-AE-09",
    Some(Bt::VatTaxAmount),
    Some(Bg::VatBreakdown),
    "The reverse-charge tax amount is zero."
);
rule!(
    BR_AE_10,
    "BR-AE-10",
    Some(Bt::VatExemptionText),
    Some(Bg::VatBreakdown),
    "A reverse-charge VAT breakdown has an exemption reason."
);

rule!(
    BR_IC_01,
    "BR-IC-01",
    Some(Bt::VatCategoryCode),
    Some(Bg::VatBreakdown),
    "An intra-community line requires an intra-community VAT breakdown."
);
rule!(
    BR_IC_05,
    "BR-IC-05",
    Some(Bt::ItemVatRate),
    Some(Bg::InvoiceLine),
    "An intra-community line has a VAT rate of zero."
);
rule!(
    BR_IC_08,
    "BR-IC-08",
    Some(Bt::VatTaxableAmount),
    Some(Bg::VatBreakdown),
    "The intra-community taxable amount equals the matching line net amounts."
);
rule!(
    BR_IC_09,
    "BR-IC-09",
    Some(Bt::VatTaxAmount),
    Some(Bg::VatBreakdown),
    "The intra-community tax amount is zero."
);
rule!(
    BR_IC_10,
    "BR-IC-10",
    Some(Bt::VatExemptionText),
    Some(Bg::VatBreakdown),
    "An intra-community VAT breakdown has an exemption reason."
);
rule!(
    BR_IC_11,
    "BR-IC-11",
    Some(Bt::ActualDeliveryDate),
    None,
    "Intra-community supply has an actual delivery date or invoicing period."
);

rule!(
    BR_G_01,
    "BR-G-01",
    Some(Bt::VatCategoryCode),
    Some(Bg::VatBreakdown),
    "An export line requires an export VAT breakdown."
);
rule!(
    BR_G_05,
    "BR-G-05",
    Some(Bt::ItemVatRate),
    Some(Bg::InvoiceLine),
    "An export line has a VAT rate of zero."
);
rule!(
    BR_G_08,
    "BR-G-08",
    Some(Bt::VatTaxableAmount),
    Some(Bg::VatBreakdown),
    "The export taxable amount equals the matching line net amounts."
);
rule!(
    BR_G_09,
    "BR-G-09",
    Some(Bt::VatTaxAmount),
    Some(Bg::VatBreakdown),
    "The export tax amount is zero."
);
rule!(
    BR_G_10,
    "BR-G-10",
    Some(Bt::VatExemptionText),
    Some(Bg::VatBreakdown),
    "An export VAT breakdown has an exemption reason."
);

rule!(
    BR_O_01,
    "BR-O-01",
    Some(Bt::VatCategoryCode),
    Some(Bg::VatBreakdown),
    "A not-subject-to-VAT line requires a not-subject-to-VAT VAT breakdown."
);
rule!(
    BR_O_05,
    "BR-O-05",
    Some(Bt::ItemVatRate),
    Some(Bg::InvoiceLine),
    "A not-subject-to-VAT line has no VAT rate."
);
rule!(
    BR_O_08,
    "BR-O-08",
    Some(Bt::VatTaxableAmount),
    Some(Bg::VatBreakdown),
    "The not-subject-to-VAT taxable amount equals the matching line net amounts."
);
rule!(
    BR_O_09,
    "BR-O-09",
    Some(Bt::VatTaxAmount),
    Some(Bg::VatBreakdown),
    "The not-subject-to-VAT tax amount is zero."
);
rule!(
    BR_O_10,
    "BR-O-10",
    Some(Bt::VatExemptionText),
    Some(Bg::VatBreakdown),
    "A not-subject-to-VAT VAT breakdown has an exemption reason."
);
rule!(
    BR_O_12,
    "BR-O-12",
    Some(Bt::ItemVatCategory),
    Some(Bg::InvoiceLine),
    "An invoice with a not-subject-to-VAT line cannot mix other VAT categories."
);

/// Rules attached to one VAT category.
#[derive(Clone, Copy, Debug)]
pub struct CategoryRules {
    /// The category must appear in the VAT breakdown.
    pub presence: &'static Rule,
    /// The category/rate pairing on each line.
    pub rate: &'static Rule,
    /// The taxable amount in the breakdown.
    pub base: &'static Rule,
    /// The tax amount in the breakdown.
    pub tax: &'static Rule,
    /// Whether an exemption reason is required or forbidden.
    pub exemption: &'static Rule,
    /// True when an exemption reason is required.
    pub exemption_required: bool,
}

/// Standard-rated rules.
pub static STANDARD_RULES: CategoryRules = CategoryRules {
    presence: &BR_S_01,
    rate: &BR_S_05,
    base: &BR_S_08,
    tax: &BR_S_09,
    exemption: &BR_S_10,
    exemption_required: false,
};

/// Zero-rated rules.
pub static ZERO_RATED_RULES: CategoryRules = CategoryRules {
    presence: &BR_Z_01,
    rate: &BR_Z_05,
    base: &BR_Z_08,
    tax: &BR_Z_09,
    exemption: &BR_Z_10,
    exemption_required: false,
};

/// Exempt rules.
pub static EXEMPT_RULES: CategoryRules = CategoryRules {
    presence: &BR_E_01,
    rate: &BR_E_05,
    base: &BR_E_08,
    tax: &BR_E_09,
    exemption: &BR_E_10,
    exemption_required: true,
};

/// Reverse-charge rules.
pub static REVERSE_CHARGE_RULES: CategoryRules = CategoryRules {
    presence: &BR_AE_01,
    rate: &BR_AE_05,
    base: &BR_AE_08,
    tax: &BR_AE_09,
    exemption: &BR_AE_10,
    exemption_required: true,
};

/// Intra-community supply rules.
pub static INTRA_COMMUNITY_RULES: CategoryRules = CategoryRules {
    presence: &BR_IC_01,
    rate: &BR_IC_05,
    base: &BR_IC_08,
    tax: &BR_IC_09,
    exemption: &BR_IC_10,
    exemption_required: true,
};

/// Export rules.
pub static EXPORT_RULES: CategoryRules = CategoryRules {
    presence: &BR_G_01,
    rate: &BR_G_05,
    base: &BR_G_08,
    tax: &BR_G_09,
    exemption: &BR_G_10,
    exemption_required: true,
};

/// Outside-the-scope rules.
pub static OUTSIDE_SCOPE_RULES: CategoryRules = CategoryRules {
    presence: &BR_O_01,
    rate: &BR_O_05,
    base: &BR_O_08,
    tax: &BR_O_09,
    exemption: &BR_O_10,
    exemption_required: true,
};

/// Returns the rules for a category, or `None` when EN 16931 has no dedicated
/// category rule set (the Canary Islands and Ceuta/Melilla categories).
pub fn category_rules(category: VatCategory) -> Option<&'static CategoryRules> {
    match category {
        VatCategory::Standard => Some(&STANDARD_RULES),
        VatCategory::ZeroRated => Some(&ZERO_RATED_RULES),
        VatCategory::Exempt => Some(&EXEMPT_RULES),
        VatCategory::ReverseCharge => Some(&REVERSE_CHARGE_RULES),
        VatCategory::IntraCommunity => Some(&INTRA_COMMUNITY_RULES),
        VatCategory::Export => Some(&EXPORT_RULES),
        VatCategory::OutsideScope => Some(&OUTSIDE_SCOPE_RULES),
        VatCategory::CanaryIslands | VatCategory::CeutaMelilla => None,
    }
}

/// Every rule the engine can emit.
pub static CORE_RULES: &[&Rule] = &[
    &BR_02, &BR_16, &BR_CO_10, &BR_CO_13, &BR_CO_14, &BR_CO_15, &BR_CO_16, &BR_CO_17, &BR_CO_26, &BR_S_01, &BR_S_05,
    &BR_S_08, &BR_S_09, &BR_S_10, &BR_Z_01, &BR_Z_05, &BR_Z_08, &BR_Z_09, &BR_Z_10, &BR_E_01, &BR_E_05, &BR_E_08,
    &BR_E_09, &BR_E_10, &BR_AE_01, &BR_AE_05, &BR_AE_08, &BR_AE_09, &BR_AE_10, &BR_IC_01, &BR_IC_05, &BR_IC_08,
    &BR_IC_09, &BR_IC_10, &BR_IC_11, &BR_G_01, &BR_G_05, &BR_G_08, &BR_G_09, &BR_G_10, &BR_O_01, &BR_O_05, &BR_O_08,
    &BR_O_09, &BR_O_10, &BR_O_12,
];

/// Looks up a rule by code.
pub fn rule(code: &str) -> Option<&'static Rule> {
    CORE_RULES.iter().copied().find(|rule| rule.code == code)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn category_rule_sets() -> [&'static CategoryRules; 7] {
        [
            &STANDARD_RULES,
            &ZERO_RATED_RULES,
            &EXEMPT_RULES,
            &REVERSE_CHARGE_RULES,
            &INTRA_COMMUNITY_RULES,
            &EXPORT_RULES,
            &OUTSIDE_SCOPE_RULES,
        ]
    }

    #[test]
    fn rule_codes_are_unique() {
        let mut seen = BTreeSet::new();
        for rule in CORE_RULES {
            assert!(seen.insert(rule.code), "duplicate rule code {}", rule.code);
        }
    }

    #[test]
    fn every_category_rule_is_in_the_catalogue() {
        for rules in category_rule_sets() {
            for candidate in [rules.presence, rules.rate, rules.base, rules.tax, rules.exemption] {
                assert!(rule(candidate.code).is_some(), "{} is missing", candidate.code);
            }
        }
    }

    #[test]
    fn lookup_returns_the_rule() {
        assert_eq!(rule("BR-CO-10").map(|rule| rule.bt), Some(Some(Bt::SumLineNet)));
        assert!(rule("BR-99-99").is_none());
    }
}
