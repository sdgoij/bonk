//! Invoice totals with per-VAT-category breakdown.
//!
//! The order is fixed by the standard: round each line net, sum the taxable
//! base per category, then apply the rate. Doing it in any other order produces
//! totals that disagree with the tax breakdown, which is the first thing a
//! validator checks.

use std::collections::BTreeMap;

use crate::error::DomainError;
use crate::invoice::DraftLine;
use crate::money::Money;
use crate::vat::{VatCategory, VatRate};

/// One VAT category's contribution.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TaxBreakdown {
    /// EN 16931 category.
    pub category: VatCategory,
    /// Rate in basis points.
    pub rate_bp: u16,
    /// Sum of line nets in this category.
    pub taxable_base: Money,
    /// Tax on that base.
    pub tax: Money,
}

/// Document totals.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InvoiceTotals {
    /// Sum of line nets (BT-106).
    pub net: Money,
    /// Sum of tax per category (BT-110).
    pub tax: Money,
    /// Net + tax (BT-112).
    pub gross: Money,
    /// Per-category breakdown, ordered by category then rate.
    pub breakdown: Vec<TaxBreakdown>,
}

/// Calculates totals for a set of lines.
///
/// All lines must share one currency; the first line determines it.
pub fn calculate_totals(lines: &[DraftLine]) -> Result<InvoiceTotals, DomainError> {
    let Some(first) = lines.first() else {
        return Err(DomainError::EmptyInvoice);
    };
    let currency = first.unit_price.currency();

    let mut base_by_category: BTreeMap<(VatCategory, u16), Money> = BTreeMap::new();
    let mut net = Money::zero(currency);

    for line in lines {
        let line_net = line.net()?;
        net = net.checked_add(line_net)?;
        let key = (line.treatment.category, line.treatment.rate.basis_points());
        let entry = base_by_category.entry(key).or_insert_with(|| Money::zero(currency));
        *entry = entry.checked_add(line_net)?;
    }

    let mut tax = Money::zero(currency);
    let mut breakdown = Vec::with_capacity(base_by_category.len());
    for ((category, rate_bp), taxable_base) in base_by_category {
        let rate = VatRate::from_basis_points(rate_bp)?;
        let category_tax = crate::calc::tax_amount(taxable_base, rate)?;
        tax = tax.checked_add(category_tax)?;
        breakdown.push(TaxBreakdown {
            category,
            rate_bp,
            taxable_base,
            tax: category_tax,
        });
    }

    Ok(InvoiceTotals {
        net,
        tax,
        gross: net.checked_add(tax)?,
        breakdown,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::LineId;
    use crate::money::Currency;
    use crate::quantity::{Quantity, UnitCode};
    use crate::vat::TaxTreatment;

    fn line(minor: i64, rate: u16, category: VatCategory) -> DraftLine {
        DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Item".to_owned(),
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_minor_units(minor, Currency::EUR).unwrap(),
            price_base_quantity: Quantity::ONE,
            treatment: TaxTreatment {
                category,
                rate: VatRate::from_basis_points(rate).unwrap(),
                exemption_reason: None,
                exemption_reason_code: None,
            },
        }
    }

    #[test]
    fn single_rate_document() {
        let totals = calculate_totals(&[line(10_000, 2100, VatCategory::Standard)]).unwrap();
        assert_eq!(totals.net, Money::from_major_units(100, Currency::EUR).unwrap());
        assert_eq!(totals.tax, Money::from_major_units(21, Currency::EUR).unwrap());
        assert_eq!(totals.gross, Money::from_major_units(121, Currency::EUR).unwrap());
        assert_eq!(totals.breakdown.len(), 1);
    }

    #[test]
    fn groups_by_category_and_rate() {
        let lines = [
            line(10_000, 2100, VatCategory::Standard),
            line(5_000, 2100, VatCategory::Standard),
            line(2_000, 900, VatCategory::Standard),
            line(4_000, 0, VatCategory::ZeroRated),
        ];
        let totals = calculate_totals(&lines).unwrap();
        assert_eq!(totals.breakdown.len(), 3);
        let standard = totals.breakdown.iter().find(|row| row.rate_bp == 2100).unwrap();
        assert_eq!(
            standard.taxable_base,
            Money::from_major_units(150, Currency::EUR).unwrap()
        );
        assert_eq!(standard.tax, Money::from_minor_units(3_150, Currency::EUR).unwrap());
    }

    #[test]
    fn empty_document_is_rejected() {
        assert!(matches!(
            calculate_totals(&[]),
            Err(crate::error::DomainError::EmptyInvoice)
        ));
    }
}
