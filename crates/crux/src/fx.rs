//! Exchange rates between a document's currency and the reporting currency.
//!
//! A rate is stored as reporting-currency micro-units per one unit of the source
//! currency, so `1_000_000` is parity. Conversion multiplies and rounds once, at
//! the point the amount is fixed, and never goes through a float. The rate is a
//! value that travels with the posting: a report re-converting at today's rate
//! would restate history every time the market moved.

use crate::error::DomainError;
use crate::money::{Currency, MICROS_PER_UNIT, Money, div_round_half_away_from_zero};

/// A positive exchange rate, in reporting micro-units per source unit.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct ExchangeRate {
    micros: i64,
}

impl ExchangeRate {
    /// Parity: one unit of the source buys one unit of the reporting currency.
    pub const ONE: Self = Self {
        micros: MICROS_PER_UNIT,
    };

    /// Builds a rate from micro-units, rejecting zero and negative values.
    pub fn from_per_unit_micros(micros: i64) -> Result<Self, DomainError> {
        if micros <= 0 {
            return Err(DomainError::InvalidRate(micros));
        }
        Ok(Self { micros })
    }

    /// The rate in micro-units.
    pub const fn per_unit_micros(self) -> i64 {
        self.micros
    }

    /// Converts an amount into `base` at this rate, rounding half away from zero.
    ///
    /// The result can differ from the source amount by a micro-unit per line; a
    /// posting that must balance in the reporting currency adjusts the control
    /// line so the residual lands where the subledger can explain it.
    pub fn convert(self, amount: Money, base: Currency) -> Result<Money, DomainError> {
        let product = i128::from(amount.micros())
            .checked_mul(i128::from(self.micros))
            .ok_or(DomainError::AmountOverflow)?;
        let rounded = div_round_half_away_from_zero(product, i128::from(MICROS_PER_UNIT));
        let micros = i64::try_from(rounded).map_err(|_| DomainError::AmountOverflow)?;
        Ok(Money::from_micros(micros, base))
    }
}

impl std::fmt::Display for ExchangeRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.micros)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eur(micros: i64) -> Money {
        Money::from_micros(micros, Currency::EUR)
    }

    fn rate(micros: i64) -> ExchangeRate {
        ExchangeRate::from_per_unit_micros(micros).unwrap()
    }

    #[test]
    fn parity_is_the_identity() {
        assert_eq!(
            rate(1_000_000).convert(eur(12_345_678), Currency::EUR).unwrap(),
            eur(12_345_678)
        );
        assert_eq!(ExchangeRate::ONE.convert(eur(1), Currency::EUR).unwrap(), eur(1));
    }

    #[test]
    fn a_rate_changes_the_currency_and_the_amount() {
        // 0.9259 EUR per USD, six decimals of rate precision.
        let converted = rate(925_900).convert(Money::from_micros(10_000_000, Currency::USD), Currency::EUR);
        assert_eq!(converted.unwrap(), eur(9_259_000));
    }

    #[test]
    fn conversion_rounds_half_away_from_zero() {
        // 1.5 micros at a rate of one half is 0.75 -> 1.
        let half = rate(500_000);
        assert_eq!(half.convert(eur(1), Currency::EUR).unwrap(), eur(1));
        assert_eq!(half.convert(eur(-1), Currency::EUR).unwrap(), eur(-1));
    }

    #[test]
    fn zero_and_negative_rates_are_refused() {
        assert!(ExchangeRate::from_per_unit_micros(0).is_err());
        assert!(ExchangeRate::from_per_unit_micros(-1).is_err());
    }

    #[test]
    fn overflow_is_an_error_not_a_wrap() {
        let huge = ExchangeRate::from_per_unit_micros(i64::MAX).unwrap();
        assert!(
            huge.convert(Money::from_micros(i64::MAX, Currency::USD), Currency::EUR)
                .is_err()
        );
    }
}
