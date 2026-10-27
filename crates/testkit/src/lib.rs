#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Test fixtures, builders, fake clocks, and the golden-file harness.
//!
//! Kept separate so production crates never depend on test scaffolding.

use crux::{Currency, Money, Quantity, VatRate};

/// A deterministic EUR amount builder for tests.
pub fn eur(major: i64, minor: u8) -> Money {
    debug_assert!(minor < 100);
    Money::from_micros(major * crux::MICROS_PER_UNIT + i64::from(minor) * 10_000, Currency::EUR)
}

/// A deterministic quantity builder for tests.
pub fn qty(whole: i64, thousandths: u16) -> Quantity {
    debug_assert!(thousandths < 1000);
    Quantity::from_micros(whole * crux::MICROS_PER_UNIT + i64::from(thousandths) * 1000)
}

/// The Dutch standard rate, spelled out so tests do not hardcode 21.
pub const NL_STANDARD: VatRate = VatRate::STANDARD_NL;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_match_the_domain_parsers() {
        assert_eq!(eur(12, 34), Money::parse_fixed("12.34", Currency::EUR).unwrap());
        assert_eq!(qty(2, 500), Quantity::parse("2.5").unwrap());
    }

    #[test]
    fn standard_rate_is_21_percent() {
        assert_eq!(NL_STANDARD.basis_points(), 2100);
    }
}
