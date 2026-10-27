//! VAT rates and EN 16931 categories.
//!
//! Rates are basis points, so 21 % is `2100` and 9.5 % is `950`. The pairing
//! rules here are the small set that can be enforced without a codelist: a
//! standard-rated line needs a positive rate, every non-standard category needs
//! zero at the invoice level (the tax is accounted for elsewhere).

use crate::error::DomainError;

/// A VAT rate in basis points (1 % = 100 bp).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct VatRate {
    basis_points: u16,
}

impl VatRate {
    /// 0 %.
    pub const ZERO: Self = Self { basis_points: 0 };
    /// Dutch standard rate, 21 %.
    pub const STANDARD_NL: Self = Self { basis_points: 2100 };
    /// Dutch reduced rate, 9 %.
    pub const REDUCED_NL: Self = Self { basis_points: 900 };

    /// Builds a rate from basis points, rejecting anything above 100 %.
    pub fn from_basis_points(basis_points: u16) -> Result<Self, DomainError> {
        if basis_points > 10_000 {
            return Err(DomainError::InvalidVatRate(basis_points.to_string()));
        }
        Ok(Self { basis_points })
    }

    /// Parses a percentage such as `21`, `9.5`, or `0`.
    pub fn parse_percent(value: &str) -> Result<Self, DomainError> {
        let trimmed = value.trim();
        let (whole, fraction) = match trimmed.split_once('.') {
            Some((whole, fraction)) => (whole, fraction),
            None => (trimmed, ""),
        };
        if whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || fraction.len() > 2
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(DomainError::InvalidVatRate(value.to_owned()));
        }
        let whole: u16 = whole
            .parse()
            .map_err(|_| DomainError::InvalidVatRate(value.to_owned()))?;
        let mut padded = String::from(fraction);
        while padded.len() < 2 {
            padded.push('0');
        }
        let fractional: u16 = padded
            .parse()
            .map_err(|_| DomainError::InvalidVatRate(value.to_owned()))?;
        let basis_points = whole
            .checked_mul(100)
            .and_then(|value| value.checked_add(fractional))
            .ok_or_else(|| DomainError::InvalidVatRate(value.to_owned()))?;
        Self::from_basis_points(basis_points)
    }

    /// The rate in basis points.
    pub const fn basis_points(self) -> u16 {
        self.basis_points
    }

    /// True for 0 %.
    pub const fn is_zero(self) -> bool {
        self.basis_points == 0
    }

    /// Renders as a percentage without trailing zeros, e.g. `21` or `9.5`.
    pub fn percent_string(self) -> String {
        let whole = self.basis_points / 100;
        let fraction = self.basis_points % 100;
        if fraction == 0 {
            return whole.to_string();
        }
        let rounded = format!("{fraction:02}");
        let trimmed = rounded.trim_end_matches('0');
        format!("{whole}.{trimmed}")
    }
}

impl std::fmt::Display for VatRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}%", self.percent_string())
    }
}

/// EN 16931 VAT category codes (BT-118 / BT-151).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum VatCategory {
    /// Standard rate (`S`).
    Standard,
    /// Zero rated (`Z`).
    ZeroRated,
    /// Exempt (`E`).
    Exempt,
    /// Reverse charge (`AE`).
    ReverseCharge,
    /// Intra-community supply (`K`).
    IntraCommunity,
    /// Export outside the EU (`G`).
    Export,
    /// Outside the scope of VAT (`O`).
    OutsideScope,
    /// Canary Islands (`L`).
    CanaryIslands,
    /// Ceuta and Melilla (`M`).
    CeutaMelilla,
}

impl VatCategory {
    /// The one- or two-letter EN 16931 code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Standard => "S",
            Self::ZeroRated => "Z",
            Self::Exempt => "E",
            Self::ReverseCharge => "AE",
            Self::IntraCommunity => "K",
            Self::Export => "G",
            Self::OutsideScope => "O",
            Self::CanaryIslands => "L",
            Self::CeutaMelilla => "M",
        }
    }

    /// Parses an EN 16931 category code.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "S" => Ok(Self::Standard),
            "Z" => Ok(Self::ZeroRated),
            "E" => Ok(Self::Exempt),
            "AE" => Ok(Self::ReverseCharge),
            "K" => Ok(Self::IntraCommunity),
            "G" => Ok(Self::Export),
            "O" => Ok(Self::OutsideScope),
            "L" => Ok(Self::CanaryIslands),
            "M" => Ok(Self::CeutaMelilla),
            other => Err(DomainError::InvalidVatRate(other.to_owned())),
        }
    }

    /// True when the category requires a zero invoice-level rate.
    pub const fn requires_zero_rate(self) -> bool {
        !matches!(self, Self::Standard | Self::CanaryIslands | Self::CeutaMelilla)
    }
}

/// Checks that a category and rate may appear together on an invoice line.
///
/// This is intentionally conservative: it rejects combinations that are always
/// wrong, and leaves the rest to the EN 16931 rule engine, which has the
/// codelist and the full context.
pub fn validate_treatment(category: VatCategory, rate: VatRate) -> Result<(), DomainError> {
    if category.requires_zero_rate() && !rate.is_zero() {
        return Err(DomainError::InvalidVatTreatment {
            category: category.code(),
            basis_points: rate.basis_points(),
        });
    }
    if category == VatCategory::Standard && rate.is_zero() {
        return Err(DomainError::InvalidVatTreatment {
            category: category.code(),
            basis_points: rate.basis_points(),
        });
    }
    Ok(())
}

/// The resolved VAT treatment of a line: category, rate, and (when relevant)
/// the exemption reason.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TaxTreatment {
    /// EN 16931 category.
    pub category: VatCategory,
    /// Invoice-level rate.
    pub rate: VatRate,
    /// Free-text exemption reason (BT-120).
    pub exemption_reason: Option<String>,
    /// Codelist exemption reason (BT-121).
    pub exemption_reason_code: Option<String>,
}

impl TaxTreatment {
    /// A standard-rated treatment.
    pub fn standard(rate: VatRate) -> Self {
        Self {
            category: VatCategory::Standard,
            rate,
            exemption_reason: None,
            exemption_reason_code: None,
        }
    }

    /// A zero-rated or exempt treatment.
    pub fn zero(category: VatCategory) -> Self {
        Self {
            category,
            rate: VatRate::ZERO,
            exemption_reason: None,
            exemption_reason_code: None,
        }
    }

    /// Attaches an exemption reason.
    pub fn with_exemption_reason(mut self, reason: impl Into<String>) -> Self {
        self.exemption_reason = Some(reason.into());
        self
    }

    /// Attaches a codelist exemption reason (BT-121).
    pub fn with_exemption_reason_code(mut self, code: impl Into<String>) -> Self {
        self.exemption_reason_code = Some(code.into());
        self
    }

    /// True when the standard asks for an exemption reason.
    pub fn needs_exemption_reason(&self) -> bool {
        matches!(self.category, VatCategory::Exempt | VatCategory::ReverseCharge)
    }

    /// Enforces the category/rate pairing.
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_treatment(self.category, self.rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_percentages() {
        assert_eq!(VatRate::parse_percent("21").unwrap().basis_points(), 2100);
        assert_eq!(VatRate::parse_percent("9.5").unwrap().basis_points(), 950);
        assert_eq!(VatRate::parse_percent("0").unwrap(), VatRate::ZERO);
        assert_eq!(VatRate::parse_percent("0.00").unwrap(), VatRate::ZERO);
        assert_eq!(VatRate::parse_percent("100").unwrap().basis_points(), 10_000);
    }

    #[test]
    fn rejects_bad_percentages() {
        assert!(VatRate::parse_percent("").is_err());
        assert!(VatRate::parse_percent("-21").is_err());
        assert!(VatRate::parse_percent("21.5.0").is_err());
        assert!(VatRate::parse_percent("21,5").is_err());
        assert!(VatRate::parse_percent("21.555").is_err());
        assert!(VatRate::from_basis_points(10_001).is_err());
        assert!(VatRate::parse_percent("101").is_err());
    }

    #[test]
    fn renders_percentages() {
        assert_eq!(VatRate::STANDARD_NL.percent_string(), "21");
        assert_eq!(VatRate::parse_percent("9.50").unwrap().percent_string(), "9.5");
        assert_eq!(VatRate::ZERO.to_string(), "0%");
    }

    #[test]
    fn categories_round_trip_through_codes() {
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
            assert_eq!(VatCategory::parse(category.code()).unwrap(), category);
        }
        assert!(VatCategory::parse("X").is_err());
    }

    #[test]
    fn treatment_rules() {
        assert!(validate_treatment(VatCategory::Standard, VatRate::STANDARD_NL).is_ok());
        assert!(validate_treatment(VatCategory::Standard, VatRate::ZERO).is_err());
        assert!(validate_treatment(VatCategory::ZeroRated, VatRate::ZERO).is_ok());
        assert!(validate_treatment(VatCategory::ZeroRated, VatRate::STANDARD_NL).is_err());
        assert!(validate_treatment(VatCategory::ReverseCharge, VatRate::ZERO).is_ok());
        assert!(validate_treatment(VatCategory::Exempt, VatRate::ZERO).is_ok());
        assert!(validate_treatment(VatCategory::IntraCommunity, VatRate::ZERO).is_ok());
        assert!(validate_treatment(VatCategory::Export, VatRate::ZERO).is_ok());
        assert!(validate_treatment(VatCategory::CanaryIslands, VatRate::ZERO).is_ok());
        assert!(validate_treatment(VatCategory::CanaryIslands, VatRate::REDUCED_NL).is_ok());
    }

    #[test]
    fn exemption_reason_builders_attach_both_forms() {
        let treatment = TaxTreatment::zero(VatCategory::Exempt)
            .with_exemption_reason("Exempt supply")
            .with_exemption_reason_code("VATEX-EU-132");

        assert_eq!(treatment.exemption_reason.as_deref(), Some("Exempt supply"));
        assert_eq!(treatment.exemption_reason_code.as_deref(), Some("VATEX-EU-132"));
        assert!(treatment.needs_exemption_reason());
    }
}
