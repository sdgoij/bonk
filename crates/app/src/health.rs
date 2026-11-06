//! Archive health: the facts a user needs to trust the file in front of them.
//!
//! The workspace shows this on demand rather than at unlock: re-hashing every
//! stored payload costs a pass over the whole archive, and a screen that freezes
//! on open is a screen people learn to avoid. The cheap facts — schema, size,
//! chain head, last backup — are collected immediately; the full audit waits for
//! the user to ask, and runs off the UI thread.

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
    /// Audit links verified by the last check, if one has run.
    pub links: Option<usize>,
    /// Why the last check failed, if it did.
    pub error: Option<String>,
    /// When the check last ran.
    pub checked_at: Option<String>,
}

/// What a payload audit found, before a panel adopts it.
///
/// Separate from [`Health`] so the audit can run on a worker thread that owns its
/// own database connection, and the window can adopt the answer when it arrives.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CheckOutcome {
    /// Counts from a clean payload pass, or `None` when something failed.
    pub report: Option<IntegrityReport>,
    /// Audit links verified, or `None` when the chain pass did not finish.
    pub links: Option<usize>,
    /// What failed, in the order the checks ran.
    pub error: Option<String>,
}

/// Which pass a running check is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CheckPhase {
    /// Recomputing the audit chain.
    Chain,
    /// Re-hashing stored payloads.
    Payloads,
    /// Matching payloads against the artefacts and imports that refer to them.
    References,
}

/// Re-hashes every stored payload and recomputes the chain, on the calling
/// thread.
///
/// The window calls this on a worker with its own connection: on a large archive
/// the pass is long enough for the window to look hung, and the UI thread must
/// keep drawing. This is a free function so both the worker and the tests run the
/// same code.
pub fn audit_archive(database: &Database) -> CheckOutcome {
    audit_archive_with_progress(database, &mut |_, _, _| {})
}

/// The same audit, reporting `(phase, done, total)` as it goes.
pub fn audit_archive_with_progress(
    database: &Database,
    progress: &mut dyn FnMut(CheckPhase, usize, usize),
) -> CheckOutcome {
    if let Err(error) = database.integrity_check() {
        return CheckOutcome {
            report: None,
            links: None,
            error: Some(error.to_string()),
        };
    }
    // The chain first: a broken chain is evidence that no longer means anything,
    // and there is no point hashing payloads behind a story that does not hold.
    let links = match audit::verify_chain_with_progress(database.connection(), &mut |done, total| {
        progress(CheckPhase::Chain, done, total);
    }) {
        Ok(links) => links,
        Err(error) => {
            return CheckOutcome {
                report: None,
                links: None,
                error: Some(error.to_string()),
            };
        }
    };
    match integrity::audit_with_progress(database.connection(), &mut |phase, done, total| {
        let phase = match phase {
            integrity::AuditPhase::Payloads => CheckPhase::Payloads,
            integrity::AuditPhase::References => CheckPhase::References,
        };
        progress(phase, done, total);
    }) {
        Ok(report) => CheckOutcome {
            report: Some(report),
            links: Some(links),
            error: None,
        },
        Err(error) => CheckOutcome {
            report: None,
            links: None,
            error: Some(error.to_string()),
        },
    }
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
            links: None,
            error: None,
            checked_at: None,
        })
    }

    /// Runs the whole check on this thread: SQLite's integrity check, the audit
    /// chain, and every stored payload.
    ///
    /// The integrity check comes first: a structurally damaged file is a
    /// different problem from a payload that no longer matches its record, and
    /// the panel should name the right one. The chain comes before the payloads
    /// because a story that no longer links up makes the evidence behind it moot.
    pub fn check(&mut self, database: &Database, now: &str) {
        self.apply(now, audit_archive(database));
    }

    /// Adopts the result of a check that ran elsewhere, stamped with the time it
    /// was started rather than the time it finished.
    pub fn apply(&mut self, now: &str, outcome: CheckOutcome) {
        self.checked_at = Some(now.to_owned());
        self.report = outcome.report;
        self.links = outcome.links;
        self.error = outcome.error;
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

        assert_eq!(health.schema_version, 11);
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
        assert_eq!(health.links, Some(0), "an empty chain verifies");

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

    /// The window's check covers the chain too, so evidence that no longer links
    /// up fails the panel and not only the CLI's `verify`.
    #[test]
    fn checking_verifies_the_audit_chain() {
        let (_dir, session) = session();
        for _ in 0..2 {
            store::audit::record_passphrase_change(
                session.database().connection(),
                "2026-10-28T12:00:00Z",
                "test",
                "keystore.bin",
            )
            .unwrap();
        }

        let mut health = Health::snapshot(session.database(), session.database_path()).unwrap();
        health.check(session.database(), "2026-10-28T12:01:00Z");
        assert!(health.is_clean(), "{:?}", health.error);
        assert_eq!(health.links, Some(2), "both links verified");

        session
            .database()
            .connection()
            .execute(
                "UPDATE audit_event SET payload_json = '{\"tampered\":true}' WHERE seq = 1",
                [],
            )
            .unwrap();

        health.check(session.database(), "2026-10-28T12:02:00Z");
        assert!(!health.is_clean());
        assert!(health.links.is_none(), "a broken chain has no verified count");
        assert!(
            health.error.as_deref().unwrap_or_default().contains("altered"),
            "{:?}",
            health.error
        );
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
