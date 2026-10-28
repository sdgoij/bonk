//! Append-only audit chain.
//!
//! Each event is hashed together with the previous hash, so removing or editing
//! a historical row breaks verification. This is the evidence backbone for both
//! invoices and (later) ledger postings.

use std::fmt::Write as _;

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::StoreError;

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// The newest link in the chain.
///
/// Two copies of the same archive should agree on this pair; a different head
/// for the same history is the first sign that they do not.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChainHead {
    /// Sequence number of the newest event.
    pub seq: i64,
    /// Hash of the newest event.
    pub hash: String,
    /// When that event was recorded.
    pub at: String,
}

/// Returns the newest link, or `None` when nothing has been recorded yet.
pub fn head(conn: &Connection) -> Result<Option<ChainHead>, StoreError> {
    conn.query_row(
        "SELECT seq, hash, at FROM audit_event ORDER BY seq DESC LIMIT 1",
        [],
        |row| {
            Ok(ChainHead {
                seq: row.get(0)?,
                hash: row.get(1)?,
                at: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(StoreError::from)
}

/// Appends one event and returns its hash (the new chain head).
#[allow(clippy::too_many_arguments)]
pub fn append(
    conn: &Connection,
    at: &str,
    actor: &str,
    kind: &str,
    subject_kind: &str,
    subject_id: &str,
    payload: Value,
) -> Result<String, StoreError> {
    let id = crux::DocumentId::new().to_string();

    let previous: Option<String> = conn
        .query_row("SELECT hash FROM audit_event ORDER BY seq DESC LIMIT 1", [], |row| {
            row.get(0)
        })
        .optional()?;
    let previous = previous.unwrap_or_else(|| GENESIS.to_owned());

    let canonical = canonical_event(&id, at, actor, kind, subject_kind, subject_id, &payload)?;
    let hash = hash_pair(&previous, &canonical);

    conn.execute(
        "INSERT INTO audit_event
             (id, at, actor, kind, subject_kind, subject_id, payload_json, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            at,
            actor,
            kind,
            subject_kind,
            subject_id,
            serde_json::to_string(&payload)?,
            previous,
            hash
        ],
    )?;

    Ok(hash)
}

/// Records that the keystore was rewrapped.
///
/// The subject is the keystore file, never key material: the chain shows when
/// the key changed, not what it is.
pub fn record_passphrase_change(conn: &Connection, at: &str, actor: &str, keystore: &str) -> Result<(), StoreError> {
    append(
        conn,
        at,
        actor,
        "passphrase_changed",
        "keystore",
        keystore,
        serde_json::json!({}),
    )?;
    Ok(())
}

/// Recomputes the whole chain. Returns the number of events verified.
pub fn verify_chain(conn: &Connection) -> Result<usize, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, at, actor, kind, subject_kind, subject_id, payload_json, prev_hash, hash
         FROM audit_event ORDER BY seq",
    )?;
    let mut rows = statement.query([])?;

    let mut expected_previous = GENESIS.to_owned();
    let mut count = 0_usize;

    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let at: String = row.get(1)?;
        let actor: String = row.get(2)?;
        let kind: String = row.get(3)?;
        let subject_kind: String = row.get(4)?;
        let subject_id: String = row.get(5)?;
        let payload_json: String = row.get(6)?;
        let prev_hash: String = row.get(7)?;
        let stored_hash: String = row.get(8)?;

        if prev_hash != expected_previous {
            return Err(StoreError::Integrity(format!(
                "audit event {id} does not link to its predecessor"
            )));
        }

        let payload: Value = serde_json::from_str(&payload_json)?;
        let canonical = canonical_event(&id, &at, &actor, &kind, &subject_kind, &subject_id, &payload)?;
        let hash = hash_pair(&prev_hash, &canonical);
        if hash != stored_hash {
            return Err(StoreError::Integrity(format!("audit event {id} has been altered")));
        }

        expected_previous = hash;
        count += 1;
    }

    Ok(count)
}

/// SHA-256 of `previous || canonical`, hex encoded.
fn hash_pair(previous: &str, canonical: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(previous.as_bytes());
    hasher.update(canonical.as_bytes());
    hex(hasher.finalize().as_slice())
}

fn canonical_event(
    id: &str,
    at: &str,
    actor: &str,
    kind: &str,
    subject_kind: &str,
    subject_id: &str,
    payload: &Value,
) -> Result<String, StoreError> {
    let event = serde_json::json!({
        "actor": actor,
        "at": at,
        "id": id,
        "kind": kind,
        "payload": payload,
        "subject_id": subject_id,
        "subject_kind": subject_kind,
    });
    // serde_json maps are BTreeMap-backed by default, so key order is stable.
    Ok(serde_json::to_string(&event)?)
}

/// SHA-256 as lowercase hex.
///
/// Shared with callers that need to hash provenance (import payloads, artefact
/// bytes) with the same function the chain uses.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex(hasher.finalize().as_slice())
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
