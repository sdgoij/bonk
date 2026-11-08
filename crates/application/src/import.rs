//! Received-document import: bytes or a file in, a stored inbound document out.
//!
//! The format readers and the folder watcher stay in the `import` crate; this
//! module is the use case a front end calls, so the desktop and mobile shells do
//! not each have to remember the argument order or the dedupe rules.

use std::path::Path;

use crux::SellerId;
use store::Database;

pub use ::import::{ImportError, ImportOutcome};

/// Imports a file as a received document, taking the display name from its path.
pub fn import_file(database: &Database, owner: SellerId, path: &Path, now: &str) -> Result<ImportOutcome, ImportError> {
    ::import::import_file(database, owner, path, now)
}

/// Imports bytes as a received document under an optional display name.
///
/// This is the mobile path: SAF or a share intent hands over bytes and a name,
/// and nothing on the host needs a filesystem path.
pub fn import_bytes(
    database: &Database,
    owner: SellerId,
    name: Option<&str>,
    bytes: &[u8],
    now: &str,
) -> Result<ImportOutcome, ImportError> {
    ::import::import_bytes(database, owner, name, bytes, now)
}
