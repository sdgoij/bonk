//! The semantic view the rule engine validates.
//!
//! This is intentionally a view over the domain draft rather than a second
//! mutable model. It exposes totals and document-level amounts that the UBL
//! writer also needs, so a rule failure points at exactly the value that would
//! be serialised.

use crux::{Currency, DocType, DraftLine, InvoiceDraft, InvoiceTotals, LocalDate, Money, Party};

use crate::profile::Profile;

/// One invoice in EN 16931 terms, ready for rule evaluation.
#[derive(Clone, Debug)]
pub struct SemanticInvoice<'a> {
    /// Invoice number (BT-1).
    pub number: &'a str,
    /// Issue date (BT-2).
    pub issue_date: LocalDate,
    /// Due date (BT-9), when present.
    pub due_date: Option<LocalDate>,
    /// Actual delivery date (BT-72), when present.
    pub delivery_date: Option<LocalDate>,
    /// Invoice or credit note (BT-3).
    pub doc_type: DocType,
    /// Document currency (BT-5).
    pub currency: Currency,
    /// Seller snapshot.
    pub seller: &'a Party,
    /// Buyer snapshot.
    pub buyer: &'a Party,
    /// Invoice lines (BG-25).
    pub lines: &'a [DraftLine],
    /// Per-category totals produced by the domain calculation.
    pub totals: InvoiceTotals,
    /// Invoice total without VAT (BT-109).
    pub tax_exclusive: Money,
    /// Invoice total with VAT (BT-112).
    pub tax_inclusive: Money,
    /// Sum of document-level allowances (BT-107).
    pub allowances: Money,
    /// Sum of document-level charges (BT-108).
    pub charges: Money,
    /// Paid amount (BT-113).
    pub paid: Money,
    /// Rounding amount (BT-114).
    pub rounding: Money,
    /// Amount due for payment (BT-115).
    pub amount_due: Money,
    /// Profile whose rules and specification identifier apply.
    pub profile: Profile,
}

impl<'a> SemanticInvoice<'a> {
    /// Builds a semantic view from a numbered draft and its calculated totals.
    ///
    /// Document-level allowances, charges, payments, and rounding are zero in
    /// the M1 subset. Keeping them explicit means the rule engine does not need
    /// a breaking change when those features arrive.
    pub fn from_draft(
        number: &'a str,
        draft: &'a InvoiceDraft,
        seller: &'a Party,
        buyer: &'a Party,
        totals: InvoiceTotals,
        profile: Profile,
    ) -> Self {
        let zero = Money::zero(draft.currency);
        let tax_exclusive = totals.net;
        let tax_inclusive = totals.gross;
        let amount_due = totals.gross;
        Self {
            number,
            issue_date: draft.issue_date,
            due_date: draft.due_date,
            delivery_date: draft.delivery_date,
            doc_type: draft.doc_type,
            currency: draft.currency,
            seller,
            buyer,
            lines: &draft.lines,
            totals,
            tax_exclusive,
            tax_inclusive,
            allowances: zero,
            charges: zero,
            paid: zero,
            rounding: zero,
            amount_due,
            profile,
        }
    }
}

#[cfg(test)]
mod tests {
    use crux::{
        Address, CountryCode, Currency, CustomerId, DraftLine, InvoiceDraft, LineId, LocalDate, Money, Party, Quantity,
        SellerId, TaxTreatment, UnitCode, VatRate, calculate_totals,
    };

    use super::*;

    fn party(name: &str) -> Party {
        Party::new(name, Address::new("Amsterdam", CountryCode::NL))
    }

    fn draft() -> InvoiceDraft {
        let mut draft = InvoiceDraft::new(
            SellerId::new(),
            CustomerId::new(),
            Currency::EUR,
            LocalDate::parse_iso("2026-10-27").unwrap(),
        );
        draft.lines.push(DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Consulting".to_owned(),
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(100, Currency::EUR).unwrap(),
            price_base_quantity: Quantity::ONE,
            treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
        });
        draft
    }

    #[test]
    fn from_draft_preserves_totals_and_zeros_document_adjustments() {
        let draft = draft();
        let totals = calculate_totals(&draft.lines).unwrap();
        let seller = party("Seller BV");
        let buyer = party("Buyer BV");
        let invoice =
            SemanticInvoice::from_draft("2026-0001", &draft, &seller, &buyer, totals.clone(), Profile::En16931);

        assert_eq!(invoice.tax_exclusive, totals.net);
        assert_eq!(invoice.tax_inclusive, totals.gross);
        assert_eq!(invoice.amount_due, totals.gross);
        assert!(invoice.allowances.is_zero());
        assert!(invoice.charges.is_zero());
        assert!(invoice.paid.is_zero());
        assert!(invoice.rounding.is_zero());
    }
}
