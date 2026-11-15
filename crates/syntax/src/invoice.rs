//! The view both writers serialise.
//!
//! UBL and CII carry the same business terms, so they share one view: a syntax
//! writer never decides a rule, and the two cannot disagree about what an invoice
//! says. The fields are pre-computed by `application`, which is where the draft
//! meets the domain.

use crux::{
    Currency, DocType, InvoiceTotals, LocalDate, Money, Party, Quantity, TaxBreakdown, TaxTreatment, UnitCode,
    VatCategory,
};

/// A document ready to be serialised.
pub struct Invoice<'a> {
    /// Human-readable invoice number (BT-1).
    pub number: &'a str,
    /// Issue date (BT-2).
    pub issue_date: LocalDate,
    /// Due date (BT-9).
    pub due_date: Option<LocalDate>,
    /// Actual delivery date (BT-72).
    pub delivery_date: Option<LocalDate>,
    /// Document type.
    pub doc_type: DocType,
    /// Document currency (BT-5).
    pub currency: Currency,
    /// Buyer reference (BT-10).
    pub buyer_reference: Option<&'a str>,
    /// Purchase order reference (BT-13).
    pub order_reference: Option<&'a str>,
    /// Free-text note (BT-22).
    pub note: Option<&'a str>,
    /// The invoice this credit note corrects (BG-3).
    pub preceding_invoice: Option<InvoiceReference<'a>>,
    /// Seller.
    pub seller: &'a Party,
    /// Customer.
    pub customer: &'a Party,
    /// Lines.
    pub lines: &'a [InvoiceLine<'a>],
    /// Totals and VAT breakdown.
    pub totals: &'a InvoiceTotals,
}

/// The invoice a credit note corrects (BG-3).
pub struct InvoiceReference<'a> {
    /// Preceding invoice number (BT-25).
    pub number: &'a str,
    /// Preceding invoice issue date (BT-26).
    pub issue_date: LocalDate,
}

/// One invoice line.
pub struct InvoiceLine<'a> {
    /// 1-based position.
    pub id: u32,
    /// Item name (BT-153).
    pub name: &'a str,
    /// Item description (BT-154).
    pub description: Option<&'a str>,
    /// Quantity (BT-129).
    pub quantity: Quantity,
    /// Unit (BT-130).
    pub unit: UnitCode,
    /// Unit price (BT-146).
    pub unit_price: Money,
    /// Price base quantity (BT-149).
    pub base_quantity: Quantity,
    /// Line net amount (BT-131), already rounded.
    pub line_net: Money,
    /// VAT treatment.
    pub treatment: &'a TaxTreatment,
}

/// The exemption code and reason a VAT breakdown row carries, if any (BT-120).
///
/// Both syntaxes write these on the header row, but the draft stores them per
/// line, so the first line that fits the row supplies them. Written here rather
/// than twice: the two writers have to agree about which line speaks for a row.
pub(crate) fn exemption_reason<'a>(
    invoice: &Invoice<'a>,
    row: &TaxBreakdown,
) -> Option<(Option<&'a str>, Option<&'a str>)> {
    invoice
        .lines
        .iter()
        .find(|line| {
            line.treatment.category == row.category
                && (row.category != VatCategory::Standard || line.treatment.rate.basis_points() == row.rate_bp)
                && (line
                    .treatment
                    .exemption_reason_code
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    || line
                        .treatment
                        .exemption_reason
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()))
        })
        .map(|line| {
            let code = line
                .treatment
                .exemption_reason_code
                .as_deref()
                .filter(|value| !value.trim().is_empty());
            let reason = line
                .treatment
                .exemption_reason
                .as_deref()
                .filter(|value| !value.trim().is_empty());
            (code, reason)
        })
}
