//! Evidence bundles: a document, its bytes, and the chain, in one verifiable
//! package.
//!
//! A bundle is a directory, not a single archive file: every part stays readable
//! with ordinary tools, and nothing depends on a zip implementation staying
//! byte-stable. The manifest lists every file with its SHA-256, and the whole
//! audit chain travels with the export so the offline verifier can recompute it
//! rather than take the exporter's word for it.
//!
//! What a bundle proves: these bytes are the ones the archive recorded for this
//! document, and this chain is internally consistent, ending at the recorded
//! head. What it does not prove: that the chain is the only history — an archive
//! could always have been forked before the export. Say so when handing a bundle
//! to someone.
//!
//! The chain is exported whole, so a bundle grows with the archive. That is the
//! price of an offline verifier being able to recompute it; a per-document
//! proof would need checkpoints the product does not have.

use std::fs;
use std::path::{Path, PathBuf};

use crux::{Currency, DocumentId, SellerId};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::audit::{self, Event};
use crate::error::StoreError;

/// Marker in the manifest.
pub const FORMAT: &str = "bonk-evidence";
/// Bundle layout version.
pub const VERSION: u32 = 1;

const MANIFEST: &str = "manifest.json";
const DOCUMENT_FILE: &str = "document.json";
const CHAIN_FILE: &str = "audit-chain.json";

/// What an export wrote, or what a verification found.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EvidenceReport {
    /// The document the bundle is about.
    pub document_id: DocumentId,
    /// Its number.
    pub number: String,
    /// Direction: `outbound` or `inbound`.
    pub direction: String,
    /// Current status.
    pub status: String,
    /// Files covered by the manifest.
    pub files: usize,
    /// Total bytes of those files.
    pub bytes: u64,
    /// Audit events in the bundled chain.
    pub events: usize,
    /// Hash of the newest event, when the chain has one.
    pub head: Option<String>,
}

/// Writes a verified evidence bundle for one document.
///
/// The directory must not already contain a bundle; evidence is never silently
/// overwritten. The export verifies what it wrote before reporting success.
pub fn export(
    conn: &Connection,
    owner_id: SellerId,
    document_id: DocumentId,
    directory: impl AsRef<Path>,
    actor: &str,
    now: &str,
) -> Result<EvidenceReport, StoreError> {
    let directory = directory.as_ref();
    if directory.join(MANIFEST).exists() {
        return Err(StoreError::Rejected(format!(
            "{} already contains an evidence bundle",
            directory.display()
        )));
    }
    fs::create_dir_all(directory)?;
    fs::create_dir_all(directory.join("artefacts"))?;

    let row = conn
        .query_row(
            "SELECT direction, number, issue_date_local, currency, semantic_json, semantic_hash,
                    state, recorded_at,
                    COALESCE((SELECT e.kind FROM document_event e WHERE e.document_id = d.id
                              ORDER BY e.seq DESC LIMIT 1), d.state)
             FROM document d
             WHERE d.id = ?1 AND d.owner_id = ?2",
            params![document_id.to_string(), owner_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("document {document_id}")))?;
    let (direction, number, issue_date, currency_text, semantic_json, semantic_hash, _, recorded_at, status) = row;
    let currency = Currency::parse(&currency_text).map_err(|error| StoreError::Data(error.to_string()))?;

    let mut files = Vec::new();

    // The document's own record, as it was stored.
    let document_json = serde_json::to_string_pretty(&serde_json::json!({
        "id": document_id.to_string(),
        "semantic": serde_json::from_str::<Value>(&semantic_json)?,
        "semantic_hash": semantic_hash,
        "status": status,
        "recorded_at": recorded_at,
    }))?;
    files.push(write_file(directory, DOCUMENT_FILE, document_json.as_bytes())?);

    // Every artefact, and the received bytes for an inbound document.
    let mut artefacts = Vec::new();
    let mut statement = conn.prepare(
        "SELECT a.kind, a.bytes_hash, b.payload
         FROM artefact a JOIN blob b ON b.id = a.blob_id
         WHERE a.document_id = ?1
         ORDER BY a.kind",
    )?;
    let mut rows = statement.query([document_id.to_string()])?;
    while let Some(row) = rows.next()? {
        let kind: String = row.get(0)?;
        let bytes_hash: String = row.get(1)?;
        let payload: Vec<u8> = row.get(2)?;
        let path = format!(
            "artefacts/{}-{}.{}",
            safe_name(&number),
            safe_name(&kind),
            extension(&kind)
        );
        let file = write_file(directory, &path, &payload)?;
        artefacts.push(ManifestArtefact {
            kind,
            file: file.path.clone(),
            sha256: bytes_hash,
            size: file.size,
        });
        files.push(file);
    }

    let mut payload_hash = None;
    let mut payload_file = None;
    if let Some((import_hash, payload)) = conn
        .query_row(
            "SELECT i.payload_hash, b.payload
             FROM import_record i LEFT JOIN blob b ON b.sha256 = i.payload_hash
             WHERE i.document_id = ?1 LIMIT 1",
            [document_id.to_string()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
        )
        .optional()?
    {
        let payload =
            payload.ok_or_else(|| StoreError::Integrity(format!("import of {document_id} has no payload on file")))?;
        let kind: String = conn.query_row(
            "SELECT source_kind FROM import_record WHERE document_id = ?1 LIMIT 1",
            [document_id.to_string()],
            |row| row.get(0),
        )?;
        let path = format!(
            "artefacts/{}-received.{}",
            safe_name(&number),
            extension_for_source(&kind)
        );
        let file = write_file(directory, &path, &payload)?;
        payload_file = Some(file.path.clone());
        files.push(file);
        payload_hash = Some(import_hash);
    }

    // The chain, whole, so the verifier can recompute it.
    let events = read_chain(conn)?;
    let chain_json = serde_json::to_string_pretty(&events)?;
    let chain_file = write_file(directory, CHAIN_FILE, chain_json.as_bytes())?;
    let head = events.last().map(|event| ManifestHead {
        seq: event.seq,
        hash: event.hash.clone(),
        at: event.at.clone(),
    });
    let event_count = events.len();
    files.push(chain_file.clone());

    let manifest = Manifest {
        format: FORMAT.to_owned(),
        version: VERSION,
        exported_at: now.to_owned(),
        exported_by: actor.to_owned(),
        document: ManifestDocument {
            id: document_id.to_string(),
            number: number.clone(),
            direction,
            status,
            issue_date,
            currency: currency.as_str().to_owned(),
            semantic_hash,
            payload_hash,
            payload_file,
            artefacts,
        },
        audit: ManifestAudit {
            events: event_count,
            head,
            file: chain_file.path,
            sha256: chain_file.sha256,
        },
        files,
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(directory.join(MANIFEST), manifest_json)?;

    // Read the bundle back the way a recipient would.
    let report = verify(directory)?;
    Ok(report)
}

/// Verifies a bundle directory without any access to the archive.
pub fn verify(directory: impl AsRef<Path>) -> Result<EvidenceReport, StoreError> {
    let directory = directory.as_ref();
    let manifest_bytes = fs::read(directory.join(MANIFEST))?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;

    if manifest.format != FORMAT {
        return Err(StoreError::Integrity(format!(
            "not an evidence bundle: format is {:?}",
            manifest.format
        )));
    }
    if manifest.version != VERSION {
        return Err(StoreError::Integrity(format!(
            "unsupported evidence bundle version {}",
            manifest.version
        )));
    }
    if manifest.exported_at.is_empty() || manifest.exported_by.is_empty() {
        return Err(StoreError::Integrity(
            "the manifest does not say when or by whom the bundle was written".to_owned(),
        ));
    }
    crux::LocalDate::parse_iso(&manifest.document.issue_date)
        .map_err(|error| StoreError::Integrity(format!("the manifest's issue date is invalid: {error}")))?;
    Currency::parse(&manifest.document.currency)
        .map_err(|error| StoreError::Integrity(format!("the manifest's currency is invalid: {error}")))?;

    // Every listed file must match, and nothing may be hiding beside them.
    let mut listed = std::collections::BTreeSet::new();
    for file in &manifest.files {
        safe_relative(&file.path)?;
        if !listed.insert(file.path.clone()) {
            return Err(StoreError::Integrity(format!("the manifest lists {} twice", file.path)));
        }
        let bytes = fs::read(directory.join(&file.path))?;
        let actual = audit::sha256_hex(&bytes);
        if actual != file.sha256 || bytes.len() as u64 != file.size {
            return Err(StoreError::Integrity(format!(
                "{} does not match the manifest",
                file.path
            )));
        }
    }
    for entry in walk(directory)? {
        if entry == "manifest.json" {
            continue;
        }
        if !listed.contains(&entry) {
            return Err(StoreError::Integrity(format!(
                "the bundle contains {entry}, which the manifest does not list"
            )));
        }
    }

    // The document record must be the one the manifest names.
    let document_bytes = read_listed(directory, &manifest.files, DOCUMENT_FILE)?;
    let document: Value = serde_json::from_slice(&document_bytes)?;
    if document.get("id").and_then(Value::as_str) != Some(manifest.document.id.as_str()) {
        return Err(StoreError::Integrity(
            "document.json does not belong to the document the manifest names".to_owned(),
        ));
    }
    if document.get("semantic_hash").and_then(Value::as_str) != Some(manifest.document.semantic_hash.as_str()) {
        return Err(StoreError::Integrity(
            "document.json does not carry the recorded semantic hash".to_owned(),
        ));
    }

    // Artefact hashes must agree with the files they name.
    for artefact in &manifest.document.artefacts {
        let listed_file = manifest
            .files
            .iter()
            .find(|file| file.path == artefact.file)
            .ok_or_else(|| StoreError::Integrity(format!("artefact file {} is not in the manifest", artefact.file)))?;
        if listed_file.sha256 != artefact.sha256 {
            return Err(StoreError::Integrity(format!(
                "artefact {} disagrees with the file it names",
                artefact.kind
            )));
        }
    }

    // The received bytes, when the document was imported, must be present and be
    // the bytes the import recorded.
    match (&manifest.document.payload_hash, &manifest.document.payload_file) {
        (None, None) => {}
        (Some(hash), Some(file)) => {
            let listed_file = manifest
                .files
                .iter()
                .find(|entry| &entry.path == file)
                .ok_or_else(|| StoreError::Integrity(format!("payload file {file} is not in the manifest")))?;
            if &listed_file.sha256 != hash {
                return Err(StoreError::Integrity(
                    "the received payload does not hash to what the import recorded".to_owned(),
                ));
            }
        }
        _ => {
            return Err(StoreError::Integrity(
                "the manifest names a payload without a file, or a file without a hash".to_owned(),
            ));
        }
    }

    // Recompute the chain with the same code the archive uses.
    let chain_entry = manifest
        .files
        .iter()
        .find(|file| file.path == manifest.audit.file)
        .ok_or_else(|| StoreError::Integrity("the chain file is not in the manifest".to_owned()))?;
    if chain_entry.sha256 != manifest.audit.sha256 {
        return Err(StoreError::Integrity(
            "the manifest's chain digest disagrees with the file it names".to_owned(),
        ));
    }
    let chain_bytes = read_listed(directory, &manifest.files, &manifest.audit.file)?;
    let events: Vec<StoredEvent> = serde_json::from_slice(&chain_bytes)?;
    if events.len() != manifest.audit.events {
        return Err(StoreError::Integrity(format!(
            "the chain holds {} events, the manifest says {}",
            events.len(),
            manifest.audit.events
        )));
    }
    let mut expected_previous = audit::genesis().to_owned();
    for event in &events {
        let value = event.payload.clone();
        let link = Event {
            id: &event.id,
            at: &event.at,
            actor: &event.actor,
            kind: &event.kind,
            subject_kind: &event.subject_kind,
            subject_id: &event.subject_id,
            payload: &value,
            prev_hash: &event.prev_hash,
            hash: &event.hash,
        };
        expected_previous = audit::verify_one(&link, &expected_previous)?;
    }
    match (&manifest.audit.head, events.last()) {
        (None, None) => {}
        (Some(head), Some(last)) if head.seq == last.seq && head.hash == last.hash && head.at == last.at => {}
        _ => {
            return Err(StoreError::Integrity(
                "the bundled chain does not end at the recorded head".to_owned(),
            ));
        }
    }
    // The chain must talk about this document. Matching on the identifier alone
    // keeps bundles exported before subject kinds were unified verifiable.
    if !events.iter().any(|event| event.subject_id == manifest.document.id) {
        return Err(StoreError::Integrity(
            "the bundled chain says nothing about this document".to_owned(),
        ));
    }

    Ok(EvidenceReport {
        document_id: DocumentId::parse_str(&manifest.document.id)
            .map_err(|error| StoreError::Data(error.to_string()))?,
        number: manifest.document.number,
        direction: manifest.document.direction,
        status: manifest.document.status,
        files: manifest.files.len(),
        bytes: manifest.files.iter().map(|file| file.size).sum(),
        events: events.len(),
        head: manifest.audit.head.map(|head| head.hash),
    })
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    format: String,
    version: u32,
    exported_at: String,
    exported_by: String,
    document: ManifestDocument,
    audit: ManifestAudit,
    files: Vec<ManifestFile>,
}

#[derive(Serialize, Deserialize)]
struct ManifestDocument {
    id: String,
    number: String,
    direction: String,
    status: String,
    issue_date: String,
    currency: String,
    semantic_hash: String,
    /// SHA-256 of the bytes as received, for an imported document.
    payload_hash: Option<String>,
    /// File holding those bytes, when there is one.
    payload_file: Option<String>,
    artefacts: Vec<ManifestArtefact>,
}

#[derive(Serialize, Deserialize)]
struct ManifestArtefact {
    kind: String,
    file: String,
    sha256: String,
    size: u64,
}

#[derive(Serialize, Deserialize)]
struct ManifestAudit {
    events: usize,
    head: Option<ManifestHead>,
    file: String,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
struct ManifestHead {
    seq: i64,
    hash: String,
    at: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct ManifestFile {
    path: String,
    sha256: String,
    size: u64,
}

/// One audit event as the bundle stores it.
#[derive(Serialize, Deserialize)]
struct StoredEvent {
    seq: i64,
    id: String,
    at: String,
    actor: String,
    kind: String,
    subject_kind: String,
    subject_id: String,
    payload: Value,
    prev_hash: String,
    hash: String,
}

fn read_chain(conn: &Connection) -> Result<Vec<StoredEvent>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT seq, id, at, actor, kind, subject_kind, subject_id, payload_json, prev_hash, hash
         FROM audit_event ORDER BY seq",
    )?;
    let mut rows = statement.query([])?;
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        let payload_json: String = row.get(7)?;
        events.push(StoredEvent {
            seq: row.get(0)?,
            id: row.get(1)?,
            at: row.get(2)?,
            actor: row.get(3)?,
            kind: row.get(4)?,
            subject_kind: row.get(5)?,
            subject_id: row.get(6)?,
            payload: serde_json::from_str(&payload_json)?,
            prev_hash: row.get(8)?,
            hash: row.get(9)?,
        });
    }
    Ok(events)
}

fn write_file(directory: &Path, relative: &str, bytes: &[u8]) -> Result<ManifestFile, StoreError> {
    fs::write(directory.join(relative), bytes)?;
    Ok(ManifestFile {
        path: relative.to_owned(),
        sha256: audit::sha256_hex(bytes),
        size: bytes.len() as u64,
    })
}

fn read_listed(directory: &Path, files: &[ManifestFile], wanted: &str) -> Result<Vec<u8>, StoreError> {
    if !files.iter().any(|file| file.path == wanted) {
        return Err(StoreError::Integrity(format!("{wanted} is not in the manifest")));
    }
    Ok(fs::read(directory.join(wanted))?)
}

/// Rejects a manifest path that could point outside the bundle.
fn safe_relative(path: &str) -> Result<(), StoreError> {
    let candidate = PathBuf::from(path);
    let escapes = candidate.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::RootDir
        )
    });
    if path.is_empty() || escapes || candidate.is_absolute() {
        return Err(StoreError::Integrity(format!(
            "{path:?} is not a path inside the bundle"
        )));
    }
    Ok(())
}

/// Lists every file below `directory`, as `/`-separated paths relative to it.
///
/// Windows accepts both separators but compares them differently, and the
/// manifest is written with `/`, so the comparison happens in that form.
fn walk(directory: &Path) -> Result<Vec<String>, StoreError> {
    let mut out = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        for entry in fs::read_dir(directory.join(&relative))? {
            let entry = entry?;
            let path = relative.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                pending.push(path);
            } else {
                out.push(
                    path.components()
                        .map(|component| component.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
    }
    Ok(out)
}

fn safe_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn extension(kind: &str) -> &'static str {
    match kind {
        "ubl" | "cii" => "xml",
        "pdf" | "facturx" => "pdf",
        _ => "bin",
    }
}

fn extension_for_source(source_kind: &str) -> &'static str {
    match source_kind {
        "ubl" | "cii" => "xml",
        "pdf" => "pdf",
        _ => "bin",
    }
}
