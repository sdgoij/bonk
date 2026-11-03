//! Whole-archive integrity: every stored payload against its recorded hash.
//!
//! `verify_artefacts` answers a narrow question about issued files. This module
//! answers the wider one the `verify` command claims to answer: are all the
//! bytes still the bytes that were recorded, does every import still have its
//! payload, and does anything point at a row that is not there?
//!
//! An orphaned blob is reported but is not corruption: nothing depends on it, it
//! just wastes space. A hash mismatch or a missing payload is corruption, because
//! the archive would be lying about evidence it claims to hold.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::audit;
use crate::error::StoreError;

/// What the audit found.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize)]
pub struct IntegrityReport {
    /// Blob payloads re-hashed and compared with their records.
    pub blobs: usize,
    /// Artefact rows whose recorded hash agrees with their blob.
    pub artefacts: usize,
    /// Import records whose payload is present in the archive.
    pub imports: usize,
    /// Blobs that nothing refers to. Reported, never treated as corruption.
    pub orphaned_blobs: usize,
}

/// Which pass a long audit is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AuditPhase {
    /// Payload bytes re-hashed against their recorded digests.
    Payloads,
    /// Payloads matched against the artefacts and imports that refer to them.
    References,
}

/// Checks the whole archive and returns what it counted.
///
/// Fails on foreign-key violations, a payload whose bytes no longer hash to its
/// record, an artefact whose hash disagrees with its blob, or an import record
/// whose payload is gone. The first problem found is named; a damaged archive
/// stops the audit rather than listing every consequence of one break.
pub fn audit(conn: &Connection) -> Result<IntegrityReport, StoreError> {
    audit_with_progress(conn, &mut |_, _, _| {})
}

/// The same audit, reporting `(phase, done, total)` as it goes.
///
/// Re-hashing a large archive takes minutes, and a caller with a user in front of
/// it needs to show that something is happening. Both passes report; the
/// reference sweep is separate because it is a second walk over every payload and
/// a bar that sits at 100 % while work continues is a lie.
///
pub fn audit_with_progress(
    conn: &Connection,
    progress: &mut dyn FnMut(AuditPhase, usize, usize),
) -> Result<IntegrityReport, StoreError> {
    check_foreign_keys(conn)?;

    let total: i64 = conn.query_row("SELECT count(*) FROM blob", [], |row| row.get(0))?;
    let total = usize::try_from(total).unwrap_or(usize::MAX);

    let mut blobs = 0_usize;
    let mut statement = conn.prepare("SELECT id, sha256, payload FROM blob")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let recorded: String = row.get(1)?;
        let payload: Vec<u8> = row.get(2)?;
        let actual = audit::sha256_hex(&payload);
        if actual != recorded {
            return Err(StoreError::Integrity(format!(
                "blob {id} does not hash to its recorded digest"
            )));
        }
        blobs += 1;
        progress(AuditPhase::Payloads, blobs, total);
    }

    // One pass over the payloads again, this time asking what points at each of
    // them: the artefact hashes have to match, and a payload nothing refers to
    // is wasted space. Walking the payloads and probing the small indexes is
    // sequential where a join from the artefact side re-reads a payload page per
    // artefact, which on an encrypted archive is the whole cost — and it gives
    // this second pass a progress figure to report.
    let mut orphaned_blobs = 0_usize;
    let mut swept = 0_usize;
    let mut statement = conn.prepare("SELECT id, sha256 FROM blob")?;
    let mut artefact_lookup = conn.prepare("SELECT id, bytes_hash FROM artefact WHERE blob_id = ?1")?;
    let mut import_lookup = conn.prepare("SELECT 1 FROM import_record WHERE payload_hash = ?1 LIMIT 1")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let blob_id: String = row.get(0)?;
        let sha256: String = row.get(1)?;

        let mut referenced = false;
        let mut artefacts = artefact_lookup.query([&blob_id])?;
        while let Some(artefact) = artefacts.next()? {
            let artefact_id: String = artefact.get(0)?;
            let bytes_hash: String = artefact.get(1)?;
            if bytes_hash != sha256 {
                return Err(StoreError::Integrity(format!(
                    "artefact {artefact_id} does not match the payload it points at"
                )));
            }
            referenced = true;
        }
        if !referenced {
            referenced = import_lookup.query_row([&sha256], |_| Ok(())).optional()?.is_some();
        }
        if !referenced {
            orphaned_blobs += 1;
        }

        swept += 1;
        progress(AuditPhase::References, swept, total);
    }

    let artefacts: i64 = conn.query_row("SELECT count(*) FROM artefact", [], |row| row.get(0))?;
    check_imports(conn)?;
    let imports: i64 = conn.query_row("SELECT count(*) FROM import_record", [], |row| row.get(0))?;

    Ok(IntegrityReport {
        blobs,
        artefacts: usize::try_from(artefacts).unwrap_or(usize::MAX),
        imports: usize::try_from(imports).unwrap_or(usize::MAX),
        orphaned_blobs,
    })
}

fn check_foreign_keys(conn: &Connection) -> Result<(), StoreError> {
    let mut statement = conn.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    if let Some(row) = rows.next()? {
        let table: String = row.get(0)?;
        let rowid: i64 = row.get(1)?;
        let parent: String = row.get(2)?;
        return Err(StoreError::Integrity(format!(
            "{table} row {rowid} points at a missing row in {parent}"
        )));
    }
    Ok(())
}

fn check_imports(conn: &Connection) -> Result<(), StoreError> {
    let missing: Option<String> = conn
        .query_row(
            "SELECT i.id FROM import_record i
             WHERE NOT EXISTS (SELECT 1 FROM blob b WHERE b.sha256 = i.payload_hash)
             LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    match missing {
        Some(id) => Err(StoreError::Integrity(format!("import {id} has no payload on file"))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_starts_at_zero() {
        let report = IntegrityReport::default();
        assert_eq!(report.blobs, 0);
        assert_eq!(report.orphaned_blobs, 0);
    }
}
