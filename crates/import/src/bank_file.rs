//! What every bank-statement reader shares.
//!
//! CAMT and MT940 come from different worlds — one is an ISO 20022 schema, the
//! other is a 1970s fixed-field telegram format — but a reader refuses the same
//! things in both: a file too large to hold twice in memory, a structure that
//! can expand beyond what it says (a DTD, unbounded nesting), a tag or element
//! that never closes, and a field the reader needs that is not there.
//!
//! The error type lives here so a caller has one thing to handle no matter which
//! format a file turned out to be. `Dtd` and `TooDeep` are only ever produced by
//! the XML reader; they stay on the shared type because splitting it in two buys
//! nothing at a call site that does not know the format in advance.

/// Largest statement a reader accepts.
pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

/// Why a statement could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BankError {
    /// The file is larger than the reader accepts.
    #[error("the statement is larger than {MAX_DOCUMENT_BYTES} bytes")]
    TooLarge,
    /// The file carries a DTD, which is how entity expansion gets in.
    #[error("the statement carries a DTD, which is not accepted")]
    Dtd,
    /// Nesting went past what the reader allows.
    #[error("the statement is nested deeper than 64 elements")]
    TooDeep,
    /// The file is not a statement of the kind the reader handles.
    #[error("this is not a bank statement: {0}")]
    NotAStatement(String),
    /// The structure itself is broken.
    #[error("the statement is malformed: {0}")]
    Malformed(String),
    /// A field the reader needs is missing or unusable.
    #[error("{0}")]
    Missing(String),
}
