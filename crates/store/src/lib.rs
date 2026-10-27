#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Encrypted local storage.
//!
//! Responsibilities:
//!
//! - [`keystore`]: wrap a random data encryption key with an Argon2id-derived
//!   key, so the passphrase can change without re-encrypting the database.
//! - [`db`]: open SQLCipher with that raw key, apply the pragmas the design
//!   requires, and run migrations.
//! - [`migrations`]: forward-only, checksummed, transactional schema changes.
//! - [`repo`]: parties, drafts, issuance, artefacts.
//! - [`audit`]: the append-only hash chain.

#![forbid(unsafe_code)]

pub mod audit;
pub mod db;
mod dto;
pub mod error;
pub mod keystore;
pub mod migrations;
pub mod repo;

pub use db::Database;
pub use error::StoreError;
pub use keystore::{KdfParams, KeyStore};
pub use repo::{
    CustomerRecord, IssueRequest, IssueView, IssuedInvoice, SellerRecord, attach_artefact, first_seller,
    insert_customer, insert_draft, insert_seller, issue_draft, list_customers, load_customer, load_draft, load_seller,
    now_rfc3339, save_draft, verify_artefacts,
};

/// True when the database file is actually encrypted.
///
/// Builds with the `plain` feature exist only so the rest of the stack can be
/// developed on hosts without a working OpenSSL/Perl toolchain. Every layer
/// above this one surfaces the difference loudly.
pub const fn cipher_enabled() -> bool {
    cfg!(feature = "sqlcipher")
}

/// Human-readable encryption state, printed by the CLI and shown in support
/// bundles. `none` means this is a development build and must not hold real
/// data.
pub const fn encryption_label() -> &'static str {
    if cfg!(feature = "sqlcipher") {
        "sqlcipher"
    } else {
        "none (test build)"
    }
}
