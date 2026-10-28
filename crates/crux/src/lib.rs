//! Domain core for the invoicing client.
//!
//! This crate has no I/O, no async runtime, no database, and no network. It owns
//! the values that legal documents are made of — money, quantities, VAT
//! treatments, dates, identifiers — and the rules that must never be duplicated
//! elsewhere.
//!
//! Design constraints enforced here:
//!
//! - Money is integer micro-units; no floating point is exposed or used.
//! - Setters are absent where an invariant would be lost; construction is
//!   checked and errors are values.
//! - Issued documents are immutable; [`invoice::transition`] is the only way to
//!   move between states, and it refuses to edit a numbered document.

#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod calc;
pub mod date;
pub mod error;
pub mod findings;
pub mod ids;
pub mod invoice;
pub mod money;
pub mod party;
pub mod quantity;
pub mod totals;
pub mod validation;
pub mod vat;

pub use calc::{AMOUNT_SCALE, Totals, line_net, sum, sum_in, tax_amount};
pub use date::LocalDate;
pub use error::DomainError;
pub use findings::{Finding, Severity, has_blocking};
pub use ids::{CustomerId, DeviceId, DocumentId, JournalEntryId, LineId, SavedSearchId, SellerId};
pub use invoice::{
    DocType, DraftLine, InvoiceDraft, InvoiceEvent, InvoiceState, PrecedingInvoice, TransitionError, transition,
};
pub use money::{Currency, MAX_SCALE, MICROS_PER_UNIT, Money};
pub use party::{Address, CountryCode, Party, VatIdCheck, validate_vat_id};
pub use quantity::{Quantity, UnitCode};
pub use totals::{InvoiceTotals, TaxBreakdown, calculate_totals};
pub use validation::validate_draft;
pub use vat::{TaxTreatment, VatCategory, VatRate, validate_treatment};
