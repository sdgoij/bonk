//! Quantities and units of measure.
//!
//! Like money, quantities are fixed-point integers. UN/ECE Recommendation 20
//! unit codes are exactly three characters; `C62` (piece) is the default that
//! the EN 16931 syntaxes expect when nothing else applies.

use crate::error::DomainError;
use crate::money::{MICROS_PER_UNIT, parse_decimal_micros};

/// A decimal quantity in micro-units.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Quantity {
    micros: i64,
}

impl Quantity {
    /// The quantity `1`.
    pub const ONE: Self = Self {
        micros: MICROS_PER_UNIT,
    };

    /// Zero.
    pub const ZERO: Self = Self { micros: 0 };

    /// Builds a quantity from micro-units.
    pub const fn from_micros(micros: i64) -> Self {
        Self { micros }
    }

    /// Builds a quantity from whole units, checked for overflow.
    pub fn from_units(units: i64) -> Result<Self, DomainError> {
        units
            .checked_mul(MICROS_PER_UNIT)
            .map(Self::from_micros)
            .ok_or(DomainError::AmountOverflow)
    }

    /// Parses a dot-decimal quantity with up to six decimals.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        parse_decimal_micros(value)
            .map(Self::from_micros)
            .map_err(|error| match error {
                DomainError::InvalidDecimal(raw) => DomainError::InvalidQuantity(raw),
                other => other,
            })
    }

    /// Raw micro-units.
    pub const fn micros(self) -> i64 {
        self.micros
    }

    /// True when the quantity is exactly zero.
    pub const fn is_zero(self) -> bool {
        self.micros == 0
    }

    /// True when the quantity is negative (credit lines).
    pub const fn is_negative(self) -> bool {
        self.micros < 0
    }

    /// Renders without trailing zeros, e.g. `2.5`.
    pub fn to_trimmed_string(self) -> String {
        let negative = self.micros < 0;
        let magnitude = self.micros.unsigned_abs();
        let whole = magnitude / MICROS_PER_UNIT as u64;
        let fraction = magnitude % MICROS_PER_UNIT as u64;
        let sign = if negative { "-" } else { "" };
        if fraction == 0 {
            return format!("{sign}{whole}");
        }
        let mut digits = format!("{fraction:06}");
        while digits.ends_with('0') {
            digits.pop();
        }
        format!("{sign}{whole}.{digits}")
    }
}

impl std::fmt::Display for Quantity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_trimmed_string())
    }
}

impl std::fmt::Debug for Quantity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Quantity({})", self.to_trimmed_string())
    }
}

/// A UN/ECE Recommendation 20 unit code.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitCode([u8; 3]);

impl UnitCode {
    /// Piece (`C62`), the default for count-based lines.
    pub const PIECE: Self = Self(*b"C62");
    /// Hour (`HUR`).
    pub const HOUR: Self = Self(*b"HUR");
    /// Day (`DAY`).
    pub const DAY: Self = Self(*b"DAY");
    /// Kilogram (`KGM`).
    pub const KILOGRAM: Self = Self(*b"KGM");
    /// Kilometre (`KMT`).
    pub const KILOMETRE: Self = Self(*b"KMT");

    /// Parses an exactly-three-character, uppercase alphanumeric code.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        let bytes = value.as_bytes();
        if bytes.len() != 3
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        {
            return Err(DomainError::InvalidUnitCode(value.to_owned()));
        }
        Ok(Self([bytes[0], bytes[1], bytes[2]]))
    }

    /// The code as a string slice.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("C62")
    }
}

impl std::fmt::Display for UnitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::fmt::Debug for UnitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UnitCode({})", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_decimals() {
        assert_eq!(Quantity::parse("2").unwrap().micros(), 2_000_000);
        assert_eq!(Quantity::parse("0.5").unwrap().micros(), 500_000);
        assert_eq!(Quantity::parse("-1.25").unwrap().micros(), -1_250_000);
        assert_eq!(Quantity::parse("0.000001").unwrap().micros(), 1);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Quantity::parse("").is_err());
        assert!(Quantity::parse("2,5").is_err());
        assert!(Quantity::parse("abc").is_err());
        assert!(Quantity::parse("1.1234567").is_err());
        assert!(Quantity::parse("+").is_err());
    }

    #[test]
    fn unit_codes_are_three_characters() {
        assert_eq!(UnitCode::parse("C62").unwrap(), UnitCode::PIECE);
        assert_eq!(UnitCode::parse("HUR").unwrap(), UnitCode::HOUR);
        assert!(UnitCode::parse("PCE").is_ok());
        assert!(UnitCode::parse("pce").is_err());
        assert!(UnitCode::parse("PC").is_err());
        assert!(UnitCode::parse("PCEE").is_err());
        assert!(UnitCode::parse("P E").is_err());
    }

    #[test]
    fn renders_readably() {
        assert_eq!(Quantity::from_units(2).unwrap().to_string(), "2");
        assert_eq!(Quantity::parse("2.50").unwrap().to_string(), "2.5");
    }

    #[test]
    fn negative_and_zero_detection() {
        assert!(Quantity::ZERO.is_zero());
        assert!(Quantity::parse("-1").unwrap().is_negative());
        assert!(!Quantity::ONE.is_negative());
    }
}
