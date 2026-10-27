//! Monetary amounts.
//!
//! Amounts are signed integers in micro-units (10⁻⁶) of the currency's major
//! unit. Binary floating point never touches money: the workspace denies
//! `clippy::float_arithmetic` and this crate exposes no float API.
//!
//! Six decimal places are stored because unit prices and quantities in real
//! invoices need more precision than the two decimals EN 16931 allows on totals.
//! Rounding happens only when a value is fixed (an amount is written), via
//! [`Money::scale_to`], and is half-away-from-zero.

use core::fmt;

use crate::error::DomainError;

/// Micro-units per major currency unit.
pub const MICROS_PER_UNIT: i64 = 1_000_000;

/// Highest supported output scale.
pub const MAX_SCALE: u32 = 6;

/// ISO 4217 currency code, stored as three uppercase ASCII bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Currency([u8; 3]);

impl Currency {
    /// Euro.
    pub const EUR: Self = Self(*b"EUR");
    /// US dollar.
    pub const USD: Self = Self(*b"USD");
    /// Pound sterling.
    pub const GBP: Self = Self(*b"GBP");

    /// Parses a three-letter code, rejecting lowercase and non-letters.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        let bytes = value.as_bytes();
        if bytes.len() != 3 || !bytes.iter().all(u8::is_ascii_uppercase) {
            return Err(DomainError::InvalidCurrency(value.to_owned()));
        }
        Ok(Self([bytes[0], bytes[1], bytes[2]]))
    }

    /// The code as a string slice.
    pub fn as_str(&self) -> &str {
        // Safe by construction: only ASCII uppercase bytes are ever stored.
        std::str::from_utf8(&self.0).unwrap_or("???")
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Currency({})", self.as_str())
    }
}

impl std::str::FromStr for Currency {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// A monetary amount in a single currency.
///
/// Ordering is derived from micro-units and is only meaningful between amounts
/// in the same currency; arithmetic refuses to mix currencies.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Money {
    micros: i64,
    currency: Currency,
}

impl Money {
    /// Zero in the given currency.
    pub const fn zero(currency: Currency) -> Self {
        Self { micros: 0, currency }
    }

    /// Builds an amount directly from micro-units.
    pub const fn from_micros(micros: i64, currency: Currency) -> Self {
        Self { micros, currency }
    }

    /// Builds an amount from whole major units, e.g. `12` -> `12.000000`.
    pub fn from_major_units(units: i64, currency: Currency) -> Result<Self, DomainError> {
        units
            .checked_mul(MICROS_PER_UNIT)
            .map(|micros| Self { micros, currency })
            .ok_or(DomainError::AmountOverflow)
    }

    /// Builds an amount from minor units (cents for a two-decimal currency).
    pub fn from_minor_units(units: i64, currency: Currency) -> Result<Self, DomainError> {
        units
            .checked_mul(10_000)
            .map(|micros| Self { micros, currency })
            .ok_or(DomainError::AmountOverflow)
    }

    /// Raw micro-units.
    pub const fn micros(self) -> i64 {
        self.micros
    }

    /// The amount's currency.
    pub const fn currency(self) -> Currency {
        self.currency
    }

    /// True when the amount is exactly zero.
    pub const fn is_zero(self) -> bool {
        self.micros == 0
    }

    /// True when the amount is negative.
    pub const fn is_negative(self) -> bool {
        self.micros < 0
    }

    /// True when the amount is strictly positive.
    pub const fn is_positive(self) -> bool {
        self.micros > 0
    }

    fn ensure_same_currency(self, other: Self) -> Result<(), DomainError> {
        if self.currency == other.currency {
            Ok(())
        } else {
            Err(DomainError::CurrencyMismatch {
                left: self.currency.to_string(),
                right: other.currency.to_string(),
            })
        }
    }

    /// Checked addition; mixed currencies are rejected.
    pub fn checked_add(self, other: Self) -> Result<Self, DomainError> {
        self.ensure_same_currency(other)?;
        self.micros
            .checked_add(other.micros)
            .map(|micros| Self {
                micros,
                currency: self.currency,
            })
            .ok_or(DomainError::AmountOverflow)
    }

    /// Checked subtraction; mixed currencies are rejected.
    pub fn checked_sub(self, other: Self) -> Result<Self, DomainError> {
        self.ensure_same_currency(other)?;
        self.micros
            .checked_sub(other.micros)
            .map(|micros| Self {
                micros,
                currency: self.currency,
            })
            .ok_or(DomainError::AmountOverflow)
    }

    /// Checked negation.
    pub fn checked_neg(self) -> Result<Self, DomainError> {
        self.micros
            .checked_neg()
            .map(|micros| Self {
                micros,
                currency: self.currency,
            })
            .ok_or(DomainError::AmountOverflow)
    }

    /// Absolute value.
    pub fn abs(self) -> Result<Self, DomainError> {
        if self.micros < 0 { self.checked_neg() } else { Ok(self) }
    }

    /// Rounds the amount to `scale` decimal places, half away from zero.
    ///
    /// The result is still stored in micro-units; the low digits are zero.
    pub fn scale_to(self, scale: u32) -> Result<Self, DomainError> {
        if scale > MAX_SCALE {
            return Err(DomainError::InvalidScale(scale));
        }
        let factor = pow10_i64(MAX_SCALE - scale);
        let rounded = div_round_half_away_from_zero(i128::from(self.micros), i128::from(factor))
            .checked_mul(i128::from(factor))
            .ok_or(DomainError::AmountOverflow)?;
        let micros = i64::try_from(rounded).map_err(|_| DomainError::AmountOverflow)?;
        Ok(Self {
            micros,
            currency: self.currency,
        })
    }

    /// Formats with exactly `scale` decimals, rounding half away from zero.
    ///
    /// This is the canonical wire format: dot decimal separator, no grouping,
    /// no currency symbol, `-` for negatives.
    pub fn to_fixed_string(self, scale: u32) -> Result<String, DomainError> {
        if scale > MAX_SCALE {
            return Err(DomainError::InvalidScale(scale));
        }
        let rounded = self.scale_to(scale)?;
        let factor = pow10_i64(MAX_SCALE - scale);
        let units = rounded.micros / factor;
        let negative = units < 0;
        let magnitude = units.unsigned_abs();
        let divisor = 10_u64.pow(scale);
        let whole = magnitude / divisor;
        let fraction = magnitude % divisor;
        let sign = if negative { "-" } else { "" };
        if scale == 0 {
            Ok(format!("{sign}{whole}"))
        } else {
            Ok(format!("{sign}{whole}.{fraction:0width$}", width = scale as usize))
        }
    }

    /// Parses a canonical dot-decimal string, rejecting grouping separators.
    ///
    /// Accepts up to six decimals; `-` is allowed, `+` is not.
    pub fn parse_fixed(value: &str, currency: Currency) -> Result<Self, DomainError> {
        let micros = parse_decimal_micros(value)?;
        Ok(Self { micros, currency })
    }

    /// Multiplies by `numerator / denominator`, rounding half away from zero.
    ///
    /// Used for line amounts (`quantity × unit price ÷ price base quantity`) and
    /// VAT (`base × rate`), where an intermediate float would lose cents.
    pub fn mul_div_round(self, numerator: i128, denominator: i128) -> Result<Self, DomainError> {
        if denominator == 0 {
            return Err(DomainError::DivisionByZero);
        }
        let product = i128::from(self.micros)
            .checked_mul(numerator)
            .ok_or(DomainError::AmountOverflow)?;
        let rounded = div_round_half_away_from_zero(product, denominator);
        let micros = i64::try_from(rounded).map_err(|_| DomainError::AmountOverflow)?;
        Ok(Self {
            micros,
            currency: self.currency,
        })
    }

    /// Debug-friendly rendering without trailing zeros, e.g. `12.34 EUR`.
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

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.to_trimmed_string(), self.currency)
    }
}

impl fmt::Debug for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Money({} {})", self.to_trimmed_string(), self.currency)
    }
}

/// Rounds `numerator / denominator` to the nearest integer, ties away from zero.
pub(crate) fn div_round_half_away_from_zero(numerator: i128, denominator: i128) -> i128 {
    debug_assert!(denominator != 0, "caller must reject a zero divisor");
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    if remainder == 0 {
        return quotient;
    }
    let twice_remainder = remainder.abs().saturating_mul(2);
    if twice_remainder >= denominator.abs() {
        quotient + numerator.signum()
    } else {
        quotient
    }
}

fn pow10_i64(exponent: u32) -> i64 {
    // Bounded by MAX_SCALE, so this cannot overflow.
    10_i64.pow(exponent.min(MAX_SCALE))
}

/// Parses a decimal string into micro-units without going through a float.
pub(crate) fn parse_decimal_micros(value: &str) -> Result<i64, DomainError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DomainError::InvalidDecimal(value.to_owned()));
    }
    let (negative, digits) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed),
    };
    if digits.is_empty() || digits.starts_with('+') {
        return Err(DomainError::InvalidDecimal(value.to_owned()));
    }

    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (digits, ""),
    };
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return Err(DomainError::InvalidDecimal(value.to_owned()));
    }
    if fraction.len() > MAX_SCALE as usize || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return Err(DomainError::InvalidDecimal(value.to_owned()));
    }

    let whole: i64 = whole
        .parse()
        .map_err(|_| DomainError::InvalidDecimal(value.to_owned()))?;
    let mut micros = whole.checked_mul(MICROS_PER_UNIT).ok_or(DomainError::AmountOverflow)?;

    if !fraction.is_empty() {
        let mut padded = String::with_capacity(MAX_SCALE as usize);
        padded.push_str(fraction);
        while padded.len() < MAX_SCALE as usize {
            padded.push('0');
        }
        let fractional: i64 = padded
            .parse()
            .map_err(|_| DomainError::InvalidDecimal(value.to_owned()))?;
        micros = micros.checked_add(fractional).ok_or(DomainError::AmountOverflow)?;
    }

    if negative {
        micros = micros.checked_neg().ok_or(DomainError::AmountOverflow)?;
    }
    Ok(micros)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eur(micros: i64) -> Money {
        Money::from_micros(micros, Currency::EUR)
    }

    #[test]
    fn currency_rejects_bad_codes() {
        assert!(Currency::parse("eur").is_err());
        assert!(Currency::parse("EURO").is_err());
        assert!(Currency::parse("EU").is_err());
        assert!(Currency::parse("E1R").is_err());
        assert_eq!(Currency::parse("EUR").unwrap(), Currency::EUR);
    }

    #[test]
    fn construction_is_checked() {
        assert_eq!(Money::from_major_units(12, Currency::EUR).unwrap(), eur(12_000_000));
        assert!(Money::from_major_units(i64::MAX, Currency::EUR).is_err());
    }

    #[test]
    fn arithmetic_rejects_mixed_currencies() {
        let eur_amount = Money::from_micros(100, Currency::EUR);
        let usd_amount = Money::from_micros(100, Currency::USD);
        assert!(eur_amount.checked_add(usd_amount).is_err());
        assert!(eur_amount.checked_sub(usd_amount).is_err());
    }

    #[test]
    fn addition_and_subtraction_round_trip() {
        let a = eur(1_234_567);
        let b = eur(765_433);
        assert_eq!(a.checked_add(b).unwrap(), eur(2_000_000));
        assert_eq!(a.checked_add(b).unwrap().checked_sub(b).unwrap(), a);
    }

    #[test]
    fn scale_rounds_half_away_from_zero() {
        assert_eq!(eur(10_500).scale_to(2).unwrap(), eur(10_000));
        assert_eq!(eur(15_000).scale_to(2).unwrap(), eur(20_000));
        assert_eq!(eur(-15_000).scale_to(2).unwrap(), eur(-20_000));
        assert_eq!(eur(-14_999).scale_to(2).unwrap(), eur(-10_000));
        assert_eq!(eur(1).scale_to(0).unwrap(), eur(0));
        assert_eq!(eur(500_000).scale_to(0).unwrap(), eur(1_000_000));
    }

    #[test]
    fn scale_rejects_out_of_range() {
        assert!(eur(1).scale_to(7).is_err());
    }

    #[test]
    fn fixed_string_is_canonical() {
        assert_eq!(eur(1_234_560).to_fixed_string(2).unwrap(), "1.23");
        assert_eq!(eur(1_235_000).to_fixed_string(2).unwrap(), "1.24");
        assert_eq!(eur(-1_235_000).to_fixed_string(2).unwrap(), "-1.24");
        assert_eq!(eur(1_000_000).to_fixed_string(2).unwrap(), "1.00");
        assert_eq!(eur(0).to_fixed_string(2).unwrap(), "0.00");
        assert_eq!(eur(1_234_567).to_fixed_string(6).unwrap(), "1.234567");
        assert_eq!(eur(1_234_567).to_fixed_string(0).unwrap(), "1");
    }

    #[test]
    fn fixed_string_handles_negative_fractions() {
        assert_eq!(eur(-500_000).to_fixed_string(2).unwrap(), "-0.50");
        assert_eq!(eur(-10_000).to_fixed_string(2).unwrap(), "-0.01");
    }

    #[test]
    fn parsing_accepts_canonical_decimals() {
        assert_eq!(Money::parse_fixed("12.34", Currency::EUR).unwrap(), eur(12_340_000));
        assert_eq!(Money::parse_fixed("-0.50", Currency::EUR).unwrap(), eur(-500_000));
        assert_eq!(Money::parse_fixed("7", Currency::EUR).unwrap(), eur(7_000_000));
        assert_eq!(Money::parse_fixed("0.000001", Currency::EUR).unwrap(), eur(1));
    }

    #[test]
    fn parsing_rejects_ambiguity() {
        assert!(Money::parse_fixed("", Currency::EUR).is_err());
        assert!(Money::parse_fixed("1.234,56", Currency::EUR).is_err());
        assert!(Money::parse_fixed("1 234.56", Currency::EUR).is_err());
        assert!(Money::parse_fixed("+1.00", Currency::EUR).is_err());
        assert!(Money::parse_fixed(".50", Currency::EUR).is_err());
        assert!(Money::parse_fixed("1.2345678", Currency::EUR).is_err());
        assert!(Money::parse_fixed("1.2.3", Currency::EUR).is_err());
    }

    #[test]
    fn mul_div_rounds_correctly() {
        let unit_price = eur(10_000_000);
        assert_eq!(unit_price.mul_div_round(3, 2).unwrap(), eur(15_000_000));
        assert_eq!(unit_price.mul_div_round(1, 3).unwrap(), eur(3_333_333));
        assert!(unit_price.mul_div_round(1, 0).is_err());
    }

    #[test]
    fn trimmed_rendering_is_human_readable() {
        assert_eq!(eur(12_340_000).to_trimmed_string(), "12.34");
        assert_eq!(eur(12_000_000).to_trimmed_string(), "12");
        assert_eq!(eur(-1).to_trimmed_string(), "-0.000001");
        assert_eq!(eur(0).to_string(), "0 EUR");
    }
}
