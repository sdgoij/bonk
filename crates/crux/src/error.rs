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

    /// A phone number could not be expressed in E.164.
    #[error("invalid phone number: {0}")]
    InvalidPhone(String),

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

    /// An account code was empty, too long, or held a character a code may not.
    #[error("invalid account code: {0:?}")]
    InvalidAccountCode(String),

    /// An account kind was not one of the five the ledger knows.
    #[error("invalid account kind: {0:?}")]
    InvalidAccountKind(String),

    /// A journal code was not one of the shipped journals.
    #[error("invalid journal: {0:?}")]
    InvalidJournal(String),

    /// A ledger side was neither `debit` nor `credit`.
    #[error("invalid ledger side: {0:?}")]
    InvalidSide(String),

    /// A ledger line carried a zero or negative amount.
    #[error("a ledger line amount must be positive, got {0} micro-units")]
    AmountNotPositive(i64),

    /// An entry was built without lines.
    #[error("a journal entry needs at least one line")]
    EmptyEntry,

    /// An entry's debits and credits did not match.
    #[error("journal entry does not balance: debits {debit_micros} vs credits {credit_micros}")]
    EntryUnbalanced {
        /// Sum of the debit lines, in micro-units.
        debit_micros: i64,
        /// Sum of the credit lines, in micro-units.
        credit_micros: i64,
    },

    /// A required piece of text was blank.
    #[error("required text is empty: {0}")]
    EmptyText(&'static str),
}

impl DomainError {
    /// A stable identifier for this error.
    ///
    /// [`Display`](std::fmt::Display) is for a developer reading a log: it is
    /// English and it is not a part of the interface. A screen shows its own
    /// sentence, chosen by this code, so the message follows the interface
    /// language and the wording around a value can change without the code moving.
    /// A code never changes once released, because findings and support bundles
    /// quote it.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidCurrency(_) => "currency-invalid",
            Self::CurrencyMismatch { .. } => "currency-mismatch",
            Self::AmountOverflow => "amount-overflow",
            Self::InvalidDecimal(_) => "decimal-invalid",
            Self::InvalidScale(_) => "scale-invalid",
            Self::InvalidQuantity(_) => "quantity-invalid",
            Self::InvalidUnitCode(_) => "unit-code-invalid",
            Self::InvalidVatRate(_) => "vat-rate-invalid",
            Self::InvalidVatTreatment { .. } => "vat-treatment-invalid",
            Self::InvalidDate(_) => "date-invalid",
            Self::InvalidCountry(_) => "country-invalid",
            Self::InvalidPhone(_) => "phone-invalid",
            Self::EmptyInvoice => "invoice-empty",
            Self::DateOutOfRange => "date-out-of-range",
            Self::InvalidId(_) => "id-invalid",
            Self::DivisionByZero => "division-by-zero",
            Self::EmptySum => "sum-empty",
            Self::InvalidAccountCode(_) => "account-code-invalid",
            Self::InvalidAccountKind(_) => "account-kind-invalid",
            Self::InvalidJournal(_) => "journal-invalid",
            Self::InvalidSide(_) => "side-invalid",
            Self::AmountNotPositive(_) => "amount-not-positive",
            Self::EmptyEntry => "entry-empty",
            Self::EntryUnbalanced { .. } => "entry-unbalanced",
            Self::EmptyText(_) => "text-empty",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DomainError;

    /// One of every variant. The compiler makes `code` cover the enum; this list
    /// is what proves the codes stay distinct, since the interface picks its
    /// sentence by code.
    fn every_error() -> Vec<DomainError> {
        vec![
            DomainError::InvalidCurrency("EU".to_owned()),
            DomainError::CurrencyMismatch {
                left: "EUR".to_owned(),
                right: "USD".to_owned(),
            },
            DomainError::AmountOverflow,
            DomainError::InvalidDecimal("x".to_owned()),
            DomainError::InvalidScale(9),
            DomainError::InvalidQuantity("x".to_owned()),
            DomainError::InvalidUnitCode("x".to_owned()),
            DomainError::InvalidVatRate("x".to_owned()),
            DomainError::InvalidVatTreatment {
                category: "S",
                basis_points: 2100,
            },
            DomainError::InvalidDate("x".to_owned()),
            DomainError::InvalidCountry("X".to_owned()),
            DomainError::InvalidPhone("x".to_owned()),
            DomainError::EmptyInvoice,
            DomainError::DateOutOfRange,
            DomainError::InvalidId("x".to_owned()),
            DomainError::DivisionByZero,
            DomainError::EmptySum,
            DomainError::InvalidAccountCode("x".to_owned()),
            DomainError::InvalidAccountKind("x".to_owned()),
            DomainError::InvalidJournal("x".to_owned()),
            DomainError::InvalidSide("x".to_owned()),
            DomainError::AmountNotPositive(-1),
            DomainError::EmptyEntry,
            DomainError::EntryUnbalanced {
                debit_micros: 1,
                credit_micros: 0,
            },
            DomainError::EmptyText("x"),
        ]
    }

    #[test]
    fn every_error_has_its_own_code() {
        let errors = every_error();
        let mut codes: Vec<&str> = errors.iter().map(DomainError::code).collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total, "two errors share a code: {codes:?}");

        for code in codes {
            assert!(
                !code.is_empty()
                    && code
                        .chars()
                        .all(|character| character.is_ascii_lowercase() || character == '-'),
                "{code} is not a code"
            );
        }
    }
}
