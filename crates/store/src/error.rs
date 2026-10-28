//! Storage errors.
//!
//! Every failure mode here is something the UI must be able to explain: a wrong
//! passphrase, a damaged file, a migration that refuses to run, or a database
//! that has been tampered with.

use thiserror::Error;

use crux::Finding;

/// Storage-layer failures.
#[derive(Debug, Error)]
pub enum StoreError {
    /// Filesystem failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// SQLite or SQLCipher failure.
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// The passphrase did not decrypt the data key.
    #[error("wrong passphrase")]
    WrongPassphrase,

    /// The keystore carries no recovery wrap, so a recovery key cannot open it.
    #[error("this keystore has no recovery key; add one while the passphrase is still known")]
    NoRecoveryKey,

    /// The recovery key is incomplete or has mistyped symbols.
    #[error("the recovery key is not valid; check for missing or mistyped groups")]
    InvalidRecoveryKey,

    /// The keystore file is malformed or from a newer version.
    #[error("keystore is corrupt or uses an unsupported version")]
    InvalidKeystore,

    /// A migration failed or its checksum changed.
    #[error("migration {version} ({name}) failed: {detail}")]
    Migration {
        /// Migration number.
        version: i64,
        /// Migration name.
        name: &'static str,
        /// What went wrong.
        detail: String,
    },

    /// The database reports internal inconsistency.
    #[error("database integrity check failed: {0}")]
    Integrity(String),

    /// A cryptographic primitive refused the input.
    #[error("cryptographic failure: {0}")]
    Crypto(String),

    /// The operating system's credential store refused the operation.
    #[error("the credential store refused: {0}")]
    Keychain(String),

    /// JSON serialisation or parsing failed.
    #[error("JSON failure: {0}")]
    Json(#[from] serde_json::Error),

    /// A record was requested but does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// A stored record does not hold the values the schema promises.
    #[error("corrupt record: {0}")]
    Data(String),

    /// The request is understood but not allowed in the current state.
    #[error("{0}")]
    Rejected(String),

    /// Artefact rendering failed.
    #[error("rendering failed: {0}")]
    Render(String),

    /// Conformance validation blocked the operation.
    #[error("conformance validation failed with {} finding(s)", .0.len())]
    Conformance(Vec<Finding>),

    /// The build is missing a required capability.
    #[error("this build lacks the {0} feature")]
    FeatureDisabled(&'static str),
}
