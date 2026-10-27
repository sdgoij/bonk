//! Calculation and rounding.
//!
//! One module owns invoice arithmetic so that no other code can invent a
//! rounding order. The sequence is the one EN 16931's BR-CO rules assume:
//! round each line net, sum per VAT category, then apply the rate.

use crate::error::DomainError;
use crate::money::{Currency, Money};
use crate::quantity::Quantity;
use crate::vat::VatRate;

/// Decimal places allowed on invoice amounts (EN 16931 BR-DEC rules).
pub const AMOUNT_SCALE: u32 = 2;

/// Line net amount: `quantity × unit price ÷ price base quantity`, rounded to
/// the cent (half away from zero).
pub fn line_net(quantity: Quantity, unit_price: Money, price_base_quantity: Quantity) -> Result<Money, DomainError> {
    if price_base_quantity.micros() <= 0 {
        return Err(DomainError::InvalidQuantity(
            "price base quantity must be positive".to_owned(),
        ));
    }
    unit_price
        .mul_div_round(i128::from(quantity.micros()), i128::from(price_base_quantity.micros()))?
        .scale_to(AMOUNT_SCALE)
}

/// Tax on a taxable base at a basis-point rate, rounded to the cent.
pub fn tax_amount(taxable_base: Money, rate: VatRate) -> Result<Money, DomainError> {
    taxable_base
        .mul_div_round(i128::from(rate.basis_points()), 10_000)?
        .scale_to(AMOUNT_SCALE)
}

/// Net and tax totals for a document.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Totals {
    /// Sum of line net amounts plus charges minus allowances.
    pub net: Money,
    /// Sum of tax per category.
    pub tax: Money,
}

impl Totals {
    /// Builds totals from a net base and a rate (single-rate documents).
    pub fn single_rate(net: Money, rate: VatRate) -> Result<Self, DomainError> {
        Ok(Self {
            net,
            tax: tax_amount(net, rate)?,
        })
    }

    /// Gross amount payable.
    pub fn gross(self) -> Result<Money, DomainError> {
        self.net.checked_add(self.tax)
    }
}

/// Sums amounts, requiring at least one element so the currency is known.
pub fn sum(amounts: &[Money]) -> Result<Money, DomainError> {
    let Some(first) = amounts.first() else {
        return Err(DomainError::EmptySum);
    };
    let mut total = Money::zero(first.currency());
    for amount in amounts {
        total = total.checked_add(*amount)?;
    }
    Ok(total)
}

/// Sums amounts in an explicitly named currency (allows an empty slice).
pub fn sum_in(amounts: &[Money], currency: Currency) -> Result<Money, DomainError> {
    let mut total = Money::zero(currency);
    for amount in amounts {
        total = total.checked_add(*amount)?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const EUR: Currency = Currency::EUR;

    fn euros(units: i64) -> Money {
        Money::from_major_units(units, EUR).unwrap()
    }

    fn cents(units: i64) -> Money {
        Money::from_micros(units * 10_000, EUR)
    }

    fn qty(value: &str) -> Quantity {
        Quantity::parse(value).unwrap()
    }

    #[test]
    fn line_net_multiplies_and_rounds() {
        // 3 × 19.99 = 59.97
        let net = line_net(qty("3"), cents(1999), Quantity::ONE).unwrap();
        assert_eq!(net, cents(5997));
    }

    #[test]
    fn line_net_respects_the_price_base_quantity() {
        // 10 kg at 12.00 per 100 kg = 1.20
        let net = line_net(qty("10"), euros(12), qty("100")).unwrap();
        assert_eq!(net, cents(120));
    }

    #[test]
    fn line_net_rounds_half_away_from_zero() {
        // 0.5 × 0.05 = 0.025 -> 0.03
        assert_eq!(line_net(qty("0.5"), cents(5), Quantity::ONE).unwrap(), cents(3));
    }

    #[test]
    fn line_net_rejects_non_positive_base() {
        assert!(line_net(qty("1"), euros(1), Quantity::ZERO).is_err());
        assert!(line_net(qty("1"), euros(1), Quantity::parse("-1").unwrap()).is_err());
    }

    #[test]
    fn tax_is_rounded_once_at_the_cent() {
        // 100.00 × 21 % = 21.00
        assert_eq!(tax_amount(euros(100), VatRate::STANDARD_NL).unwrap(), cents(2100));
        // 0.05 × 21 % = 0.0105 -> 0.01
        assert_eq!(tax_amount(cents(5), VatRate::STANDARD_NL).unwrap(), cents(1));
        // 0.02 × 21 % = 0.0042 -> 0.00
        assert_eq!(tax_amount(cents(2), VatRate::STANDARD_NL).unwrap(), cents(0));
    }

    #[test]
    fn totals_add_up() {
        let totals = Totals::single_rate(euros(100), VatRate::STANDARD_NL).unwrap();
        assert_eq!(totals.tax, cents(2100));
        assert_eq!(totals.gross().unwrap(), cents(12100));
    }

    #[test]
    fn sum_requires_a_currency() {
        assert!(matches!(sum(&[]), Err(DomainError::EmptySum)));
        assert_eq!(sum_in(&[], EUR).unwrap(), euros(0));
        assert_eq!(sum(&[euros(1), euros(2)]).unwrap(), euros(3));
    }

    #[test]
    fn sum_rejects_mixed_currencies() {
        let usd = Money::from_major_units(1, Currency::USD).unwrap();
        assert!(sum(&[euros(1), usd]).is_err());
    }

    proptest! {
        #[test]
        fn rounding_is_idempotent(
            micros in -1_000_000_000_000_i64..1_000_000_000_000_i64,
            scale in 0_u32..=6_u32,
        ) {
            let amount = Money::from_micros(micros, EUR);
            let once = amount.scale_to(scale).unwrap();
            let twice = once.scale_to(scale).unwrap();
            prop_assert_eq!(once, twice);
            prop_assert_eq!(once.micros() % 10_i64.pow(6 - scale), 0);
        }

        #[test]
        fn rounding_error_never_exceeds_half_a_step(
            micros in -1_000_000_000_000_i64..1_000_000_000_000_i64,
            scale in 0_u32..=6_u32,
        ) {
            let amount = Money::from_micros(micros, EUR);
            let rounded = amount.scale_to(scale).unwrap();
            let step = 10_i64.pow(6 - scale);
            let difference = (rounded.micros() - amount.micros()).abs();
            prop_assert!(difference <= step / 2 + 1);
        }

        #[test]
        fn addition_is_reversible(
            a in -1_000_000_000_i64..1_000_000_000_i64,
            b in -1_000_000_000_i64..1_000_000_000_i64,
        ) {
            let left = Money::from_micros(a, EUR);
            let right = Money::from_micros(b, EUR);
            let sum = left.checked_add(right).unwrap();
            prop_assert_eq!(sum.checked_sub(right).unwrap(), left);
        }

        #[test]
        fn line_net_is_monotonic_in_quantity(
            first in 0_i64..1_000_000_000_i64,
            extra in 0_i64..1_000_000_000_i64,
            price_micros in 0_i64..1_000_000_000_i64,
        ) {
            let price = Money::from_micros(price_micros, EUR);
            let low = Quantity::from_micros(first);
            let high = Quantity::from_micros(first + extra);
            let low_net = line_net(low, price, Quantity::ONE).unwrap();
            let high_net = line_net(high, price, Quantity::ONE).unwrap();
            prop_assert!(high_net >= low_net);
        }

        #[test]
        fn tax_is_within_one_cent_of_the_exact_rate(
            base_micros in 0_i64..1_000_000_000_000_i64,
            basis_points in 0_u16..=10_000_u16,
        ) {
            let base = Money::from_micros(base_micros, EUR);
            let rate = VatRate::from_basis_points(basis_points).unwrap();
            let tax = tax_amount(base, rate).unwrap();
            let exact = base.mul_div_round(i128::from(basis_points), 10_000).unwrap();
            let difference = (tax.micros() - exact.micros()).abs();
            prop_assert!(difference <= 10_000, "tax {} differs from exact {} by {}", tax.micros(), exact.micros(), difference);
        }
    }
}
