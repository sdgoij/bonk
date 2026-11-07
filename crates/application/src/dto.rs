//! Plain data crossed to a front end.
//!
//! These are deliberately not the domain types: no lifetimes, no database
//! records, and no rendering types. The rules for anything added here are:
//!
//! - money is integer micros plus an ISO currency (or a pre-formatted string
//!   when the front end only displays it);
//! - dates are ISO strings;
//! - identifiers are strings;
//! - findings and outcomes are data, not errors, so a UI can show blocking
//!   findings without parsing a message.
//!
//! The FFI crate mirrors these shapes in language-neutral records, which keeps
//! `uniffi` out of this crate.

/// What the dashboard shows for an open archive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Summary {
    /// Number of seller records.
    pub sellers: i64,
    /// Number of drafts.
    pub drafts: i64,
    /// Applied schema version.
    pub schema_version: i64,
}
