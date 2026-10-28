//! Archive health: the facts a user needs to trust the file in front of them.
//!
//! The workspace shows this on demand rather than at unlock: re-hashing every
//! stored payload costs a pass over the whole archive, and a screen that freezes
//! on open is a screen people learn to avoid. The cheap facts — schema, size,
//! chain head, last backup — are collected immediately; the payload audit waits
//! for the user to ask.

use std::path::{Path, PathBuf};

use store::{ChainHead, Database, IntegrityReport, StoreError, audit, integrity};

/// Newest backup found beside the archive.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BackupFile {
    /// File name, which carries the compact timestamp.
    pub name: String,
    /// Size in bytes.
    pub bytes: u64,
}

/// What the health panel knows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Health {
    /// Applied schema version.
    pub schema_version: i64,
    /// `sqlcipher` or `none (test build)`.
    pub encryption: &'static str,
    /// Size of the archive file.
    pub database_bytes: u64,
    /// Newest audit link.
    pub chain: Option<ChainHead>,
    /// Newest backup beside the archive.
    pub last_backup: Option<BackupFile>,
    /// Result of the last payload audit, if one has run.
    pub report: Option<IntegrityReport>,
    /// Why the last payload audit failed, if it did.
    pub error: Option<String>,
    /// When the payload audit last ran.
    pub checked_at: Option<String>,
}

impl Health {
    /// Collects the facts that cost nothing: no payload is re-hashed.
    pub fn snapshot(database: &Database, database_path: &Path) -> Result<Self, StoreError> {
        let connection = database.connection();
        Ok(Self {
            schema_version: database.schema_version()?,
            encryption: store::encryption_label(),
            database_bytes: std::fs::metadata(database_path)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
            chain: audit::head(connection)?,
            last_backup: last_backup(database_path),
            report: None,
            error: None,
            checked_at: None,
        })
    }

    /// Re-hashes every stored payload. Returns whether the archive is intact.
    ///
    /// Runs SQLite's own integrity check first: a structurally damaged file is a
    /// different problem from a payload that no longer matches its record, and
    /// the panel should name the right one.
    pub fn check(&mut self, database: &Database, now: &str) {
        self.checked_at = Some(now.to_owned());
        if let Err(error) = database.integrity_check() {
            self.report = None;
            self.error = Some(error.to_string());
            return;
        }
        match integrity::audit(database.connection()) {
            Ok(report) => {
                self.report = Some(report);
                self.error = None;
            }
            Err(error) => {
                self.report = None;
                self.error = Some(error.to_string());
            }
        }
    }

    /// True when a payload audit has run and found nothing wrong.
    pub fn is_clean(&self) -> bool {
        self.report.is_some() && self.error.is_none()
    }

    /// Short form of the chain head, for a heading that should stay readable.
    pub fn short_head(&self) -> Option<String> {
        self.chain.as_ref().map(|head| {
            let hash = head.hash.as_str();
            hash[..hash.len().min(16)].to_owned()
        })
    }
}

/// Newest `backup-*.db` beside the archive, if any.
///
/// The name carries a compact timestamp, so the newest file sorts last and there
/// is no need to trust modification times, which copies and restores rewrite.
fn last_backup(database_path: &Path) -> Option<BackupFile> {
    let directory: PathBuf = database_path.parent()?.join("backups");
    let mut newest: Option<BackupFile> = None;
    for entry in std::fs::read_dir(directory).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("backup-") || !name.ends_with(".db") {
            continue;
        }
        let bytes = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        let replace = newest.as_ref().is_none_or(|current| name > current.name);
        if replace {
            newest = Some(BackupFile { name, bytes });
        }
    }
    newest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    fn session() -> (tempfile::TempDir, Session) {
        let dir = tempfile::TempDir::new().unwrap();
        let session = Session::create(
            dir.path().join("bonk.db"),
            dir.path().join("keystore.bin"),
            "passphrase",
        )
        .unwrap();
        (dir, session)
    }

    #[test]
    fn a_fresh_archive_reports_itself() {
        let (dir, session) = session();
        let health = Health::snapshot(session.database(), session.database_path()).unwrap();

        assert_eq!(health.schema_version, 3);
        assert_eq!(health.encryption, store::encryption_label());
        assert!(health.database_bytes > 0);
        assert!(health.chain.is_none(), "nothing has happened yet");
        assert!(health.last_backup.is_none());
        assert!(!health.is_clean(), "nothing has been checked yet");
        assert!(dir.path().exists());
    }

    #[test]
    fn checking_finds_a_damaged_payload() {
        let (_dir, session) = session();
        let mut health = Health::snapshot(session.database(), session.database_path()).unwrap();
        health.check(session.database(), "2026-10-28T12:00:00Z");
        assert!(health.is_clean(), "{:?}", health.error);

        // A blob whose record does not match its bytes is corruption, and the
        // panel must say so rather than showing a green tick.
        session
            .database()
            .connection()
            .execute(
                "INSERT INTO blob (id, sha256, size_bytes, compression, payload, created_at)
                 VALUES ('blob-x', ?1, 4, NULL, X'00', '2026-10-28T12:00:00Z')",
                ["0".repeat(64)],
            )
            .unwrap();

        health.check(session.database(), "2026-10-28T12:01:00Z");
        assert!(!health.is_clean());
        assert!(health.report.is_none());
        assert!(health.error.as_deref().unwrap_or_default().contains("does not hash"));
    }

    #[test]
    fn the_newest_backup_wins() {
        let (dir, session) = session();
        let backups = dir.path().join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        std::fs::write(backups.join("backup-20261027T090000Z.db"), b"old").unwrap();
        std::fs::write(backups.join("backup-20261028T154554Z.db"), b"newer").unwrap();
        std::fs::write(backups.join("notes.txt"), b"ignored").unwrap();

        let health = Health::snapshot(session.database(), session.database_path()).unwrap();
        let backup = health.last_backup.expect("a backup is found");
        assert_eq!(backup.name, "backup-20261028T154554Z.db");
        assert_eq!(backup.bytes, 5);
    }

    #[test]
    fn a_short_head_is_readable_and_safe_on_empty_chains() {
        let (dir, session) = session();
        let health = Health::snapshot(session.database(), session.database_path()).unwrap();
        assert_eq!(health.short_head(), None);

        store::audit::record_passphrase_change(
            session.database().connection(),
            "2026-10-28T12:00:00Z",
            "test",
            "keystore.bin",
        )
        .unwrap();
        let health = Health::snapshot(session.database(), session.database_path()).unwrap();
        let head = health.short_head().expect("a head");
        assert_eq!(head.len(), 16);
        assert!(health.chain.unwrap().hash.starts_with(&head));
        assert!(dir.path().exists());
    }
}
