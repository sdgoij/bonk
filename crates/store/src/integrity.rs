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

/// Checks the whole archive and returns what it counted.
///
/// Fails on foreign-key violations, a payload whose bytes no longer hash to its
/// record, an artefact whose hash disagrees with its blob, or an import record
/// whose payload is gone. The first problem found is named; a damaged archive
/// stops the audit rather than listing every consequence of one break.
pub fn audit(conn: &Connection) -> Result<IntegrityReport, StoreError> {
    check_foreign_keys(conn)?;

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
    }

    check_artefacts(conn)?;
    let artefacts: i64 = conn.query_row("SELECT count(*) FROM artefact", [], |row| row.get(0))?;

    check_imports(conn)?;
    let imports: i64 = conn.query_row("SELECT count(*) FROM import_record", [], |row| row.get(0))?;

    let orphaned_blobs: i64 = conn.query_row(
        "SELECT count(*) FROM blob b
         WHERE NOT EXISTS (SELECT 1 FROM artefact a WHERE a.blob_id = b.id)
           AND NOT EXISTS (SELECT 1 FROM import_record i WHERE i.payload_hash = b.sha256)",
        [],
        |row| row.get(0),
    )?;

    Ok(IntegrityReport {
        blobs,
        artefacts: usize::try_from(artefacts).unwrap_or(usize::MAX),
        imports: usize::try_from(imports).unwrap_or(usize::MAX),
        orphaned_blobs: usize::try_from(orphaned_blobs).unwrap_or(usize::MAX),
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

fn check_artefacts(conn: &Connection) -> Result<(), StoreError> {
    let mismatch: Option<String> = conn
        .query_row(
            "SELECT a.id FROM artefact a
             JOIN blob b ON b.id = a.blob_id
             WHERE a.bytes_hash <> b.sha256
             LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    match mismatch {
        Some(id) => Err(StoreError::Integrity(format!(
            "artefact {id} does not match the payload it points at"
        ))),
        None => Ok(()),
    }
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
