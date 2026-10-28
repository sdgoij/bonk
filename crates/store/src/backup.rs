//! Encrypted, rotating backups and their verification.
//!
//! A backup is only a backup if it opens, checks out, and holds the same audit
//! chain: a file that exists but cannot be restored is worse than no file,
//! because it is believed. Every copy is therefore verified before old ones are
//! pruned, and verification opens the copy read-only so it cannot repair what it
//! is supposed to judge.

use std::io;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::audit::{self, ChainHead};
use crate::db::{Database, apply_key};
use crate::error::StoreError;
use crate::keystore::KeyStore;
use crate::permissions;

/// How many backups a directory keeps by default.
pub const RETENTION: usize = 10;

/// What one backup run did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BackupReport {
    /// The copy that was written and verified.
    pub path: PathBuf,
    /// Size of the copy in bytes.
    pub bytes: u64,
    /// Older copies that were pruned to stay inside the retention limit.
    pub removed: Vec<PathBuf>,
}

/// Writes a verified backup of `database` into `directory`, then prunes old ones.
///
/// The destination must not exist; `stamp` is turned into a sortable file name,
/// so names sort chronologically without reading any file. Nothing is pruned
/// unless the new copy verified.
pub fn backup(
    database: &Database,
    keystore: &KeyStore,
    directory: impl AsRef<Path>,
    stamp: &str,
    keep: usize,
) -> Result<BackupReport, StoreError> {
    backup_with_key(database, keystore.dek(), directory, stamp, keep)
}

/// Writes a verified backup using a raw data key.
///
/// The window keeps the key from the unlock in memory, so it can back up
/// without asking for the passphrase again. The copy is verified with the same
/// key that wrote it; a keystore is not needed to make or check a copy.
pub fn backup_with_key(
    database: &Database,
    dek: &[u8; 32],
    directory: impl AsRef<Path>,
    stamp: &str,
    keep: usize,
) -> Result<BackupReport, StoreError> {
    let directory = directory.as_ref();
    permissions::private_directory(directory)?;

    let path = directory.join(file_name(stamp)?);
    database.backup_to(&path)?;
    // A copy holds the whole archive; it must not be readable by more accounts
    // than the original.
    permissions::restrict_quietly(&path);
    verify(&path, dek)?;
    let bytes = std::fs::metadata(&path)?.len();

    let removed = prune(directory, keep.max(1))?;
    Ok(BackupReport { path, bytes, removed })
}

/// What a verified copy holds.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BackupSummary {
    /// Audit events in the copy, verified end to end.
    pub events: usize,
    /// Schema version the copy was written with.
    pub schema_version: i64,
    /// Newest link in its chain, when it has one.
    pub head: Option<ChainHead>,
}

/// Checks that a backup opens with this key, is intact, and has a valid chain.
///
/// Read-only on purpose: verifying a backup must not be able to change it.
pub fn verify(path: impl AsRef<Path>, dek: &[u8; 32]) -> Result<usize, StoreError> {
    Ok(inspect(path, dek)?.events)
}

/// Verifies a copy and reports what it holds, for comparing two archives.
pub fn inspect(path: impl AsRef<Path>, dek: &[u8; 32]) -> Result<BackupSummary, StoreError> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(StoreError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("backup file does not exist: {}", path.display()),
        )));
    }

    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    apply_key(&conn, dek)?;
    // Exercise the key before the checks: a wrong key must not look like damage.
    let _: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))?;

    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(StoreError::Integrity(integrity));
    }

    let events = audit::verify_chain(&conn)?;
    let schema_version: Option<i64> =
        conn.query_row("SELECT max(version) FROM schema_migration", [], |row| row.get(0))?;

    Ok(BackupSummary {
        events,
        schema_version: schema_version.unwrap_or(0),
        head: audit::head(&conn)?,
    })
}

/// Turns a timestamp into a file name that sorts chronologically.
fn file_name(stamp: &str) -> Result<String, StoreError> {
    let compact: String = stamp.chars().filter(char::is_ascii_alphanumeric).collect();
    if compact.is_empty() {
        return Err(StoreError::Rejected("a backup needs a timestamp to name it".to_owned()));
    }
    Ok(format!("backup-{compact}.db"))
}

/// Deletes the oldest `backup-*.db` files above `keep`, returning what went.
fn prune(directory: &Path, keep: usize) -> Result<Vec<PathBuf>, StoreError> {
    let mut backups: Vec<PathBuf> = std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("backup-") && name.ends_with(".db"))
        })
        .collect();
    // The name carries a compact timestamp, so lexical order is chronological.
    backups.sort();

    if backups.len() <= keep {
        return Ok(Vec::new());
    }
    let excess = backups.len() - keep;
    let removed: Vec<PathBuf> = backups.drain(..excess).collect();
    for path in &removed {
        std::fs::remove_file(path)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_becomes_a_sortable_name() {
        assert_eq!(file_name("2026-10-28T14:52:10Z").unwrap(), "backup-20261028T145210Z.db");
        assert!(file_name("---").is_err(), "a stamp with nothing in it is refused");
    }

    #[test]
    fn names_sort_chronologically() {
        let mut names = [
            file_name("2026-10-28T09:00:00Z").unwrap(),
            file_name("2026-10-28T14:52:10Z").unwrap(),
            file_name("2026-10-27T23:59:59Z").unwrap(),
        ];
        names.sort();
        assert_eq!(
            names,
            [
                "backup-20261027T235959Z.db".to_owned(),
                "backup-20261028T090000Z.db".to_owned(),
                "backup-20261028T145210Z.db".to_owned(),
            ]
        );
    }
}
