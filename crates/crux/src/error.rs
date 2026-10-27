//! Domain errors.
//!
//! Errors in `crux` are values, not panics: every public operation that can fail
//! returns a result, and no amount of user input can trigger a panic.

use thiserror::Error;

/// Everything that can go wrong inside the domain core.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    /// A currency code was not exactly three uppercase ASCII letters.
    #[error("invalid currency code: {0:?}")]
    InvalidCurrency(String),

    /// Two amounts in different currencies were combined.
    #[error("currency mismatch: {left} vs {right}")]
    CurrencyMismatch { left: String, right: String },

    /// An arithmetic operation left the representable range.
    #[error("amount overflow")]
    AmountOverflow,

    /// A decimal string could not be parsed as an amount or quantity.
    #[error("invalid decimal value: {0:?}")]
    InvalidDecimal(String),

    /// A precision outside `0..=6` was requested.
    #[error("invalid scale: {0}; must be between 0 and 6")]
    InvalidScale(u32),

    /// A quantity must be a finite decimal within range.
    #[error("invalid quantity: {0}")]
    InvalidQuantity(String),

    /// A UN/ECE unit code was not three uppercase alphanumeric characters.
    #[error("invalid unit code: {0:?}")]
    InvalidUnitCode(String),

    /// A VAT rate was outside `0..=100` percent.
    #[error("invalid VAT rate: {0}")]
    InvalidVatRate(String),

    /// A VAT category was paired with a rate the standard does not allow.
    #[error("invalid VAT treatment: category {category} cannot use {basis_points} basis points")]
    InvalidVatTreatment {
        /// EN 16931 category code, e.g. `S`.
        category: &'static str,
        /// Rate in basis points.
        basis_points: u16,
    },

    /// A date string was not a valid ISO 8601 calendar date.
    #[error("invalid date: {0:?}")]
    InvalidDate(String),

    /// A country code was not two uppercase ASCII letters.
    #[error("invalid country code: {0:?}")]
    InvalidCountry(String),

    /// A document was submitted for issuing without any lines.
    #[error("the invoice has no lines")]
    EmptyInvoice,

    /// A date arithmetic operation left the representable range.
    #[error("date out of range")]
    DateOutOfRange,

    /// An identifier could not be parsed.
    #[error("invalid identifier: {0:?}")]
    InvalidId(String),

    /// A division by zero was attempted (percentage or base quantity).
    #[error("division by zero")]
    DivisionByZero,

    /// An empty collection was summed, so the currency is unknown.
    #[error("cannot sum an empty collection of amounts")]
    EmptySum,
}
