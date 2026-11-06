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
//!
//! A posted document also carries its accountancy: the journal entries that
//! belong to it, any correction of those, and every stored VAT return whose
//! basis names one of them. A return is only explainable together with the whole
//! basis it was filed on, so including it pulls in the other postings of the
//! period as well. The ledger is written as JSON lines and verified the same way
//! the chain is: the entry's content hash is recomputed from the numbers in the
//! bundle, and that hash must also be the one the verified chain recorded for
//! the posting. A forger who rewrites a line and rebuilds every hash in the
//! manifest still has to rewrite the chain, and the chain is recomputed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crux::{
    AccountCode, Currency, DocumentId, Journal, LedgerEntry, LedgerLine, LocalDate, Money, SellerId, Side, VatCategory,
    VatRate,
};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::audit::{self, Event};
use crate::error::StoreError;
use crate::ledger::PostingSource;
use crate::vat::Rubriek;

/// Marker in the manifest.
pub const FORMAT: &str = "bonk-evidence";
/// Bundle layout version.
pub const VERSION: u32 = 1;

const MANIFEST: &str = "manifest.json";
const DOCUMENT_FILE: &str = "document.json";
const CHAIN_FILE: &str = "audit-chain.json";
const LEDGER_FILE: &str = "ledger.jsonl";
const RETURNS_DIR: &str = "vat-returns";
/// Identifiers per `IN (...)` list. Well below every build's parameter limit and
/// large enough that a busy month is a handful of statements.
const ID_CHUNK: usize = 500;

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
    /// Journal entries the bundle carries.
    pub ledger_entries: usize,
    /// Stored VAT returns whose basis the bundle carries.
    pub vat_returns: usize,
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
        "SELECT a.kind, a.bytes_hash, a.template_digest, b.payload
         FROM artefact a JOIN blob b ON b.id = a.blob_id
         WHERE a.document_id = ?1
         ORDER BY a.kind",
    )?;
    let mut rows = statement.query([document_id.to_string()])?;
    while let Some(row) = rows.next()? {
        let kind: String = row.get(0)?;
        let bytes_hash: String = row.get(1)?;
        let template_digest: Option<String> = row.get(2)?;
        let payload: Vec<u8> = row.get(3)?;
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
            template_digest,
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

    // What the posting did to the books, and the returns filed on it.
    let ledger = export_ledger(conn, document_id, directory, &mut files)?;

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
        ledger: Some(ledger),
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

    // The accountancy that belongs to the document, when the bundle carries it.
    let mut ledger_hashes: BTreeMap<String, String> = BTreeMap::new();
    if let Some(ledger) = &manifest.ledger {
        let listed = manifest
            .files
            .iter()
            .find(|file| file.path == ledger.file)
            .ok_or_else(|| StoreError::Integrity(format!("the ledger file {} is not in the manifest", ledger.file)))?;
        if listed.sha256 != ledger.sha256 {
            return Err(StoreError::Integrity(
                "the manifest's ledger digest disagrees with the file it names".to_owned(),
            ));
        }
        let records = parse_ledger(&read_listed(directory, &manifest.files, &ledger.file)?)?;
        if records.len() != ledger.entries {
            return Err(StoreError::Integrity(format!(
                "the ledger holds {} entries, the manifest says {}",
                records.len(),
                ledger.entries
            )));
        }

        // What the verified chain says each posting was. A ledger line is only
        // worth anything if the chain already recorded that posting, with that
        // content hash.
        let mut posted: BTreeMap<String, String> = BTreeMap::new();
        for event in &events {
            if event.kind == "posted"
                && let Some(hash) = event.payload.get("content_hash").and_then(Value::as_str)
            {
                posted.insert(event.subject_id.clone(), hash.to_owned());
            }
        }

        for record in &records {
            if ledger_hashes
                .insert(record.id.clone(), record.content_hash.clone())
                .is_some()
            {
                return Err(StoreError::Integrity(format!(
                    "the ledger lists entry {} twice",
                    record.id
                )));
            }
            if record_content_hash(record)? != record.content_hash {
                return Err(StoreError::Integrity(format!(
                    "ledger entry {} does not hash to what it records",
                    record.number
                )));
            }
            match posted.get(&record.id) {
                Some(recorded) if recorded == &record.content_hash => {}
                Some(_) => {
                    return Err(StoreError::Integrity(format!(
                        "the chain records a different posting for entry {}",
                        record.number
                    )));
                }
                None => {
                    return Err(StoreError::Integrity(format!(
                        "the chain does not name ledger entry {}",
                        record.number
                    )));
                }
            }
        }

        if content_chain(&records) != ledger.content_hashes {
            return Err(StoreError::Integrity(
                "the ledger does not match the content hashes the manifest records".to_owned(),
            ));
        }
        if !records.is_empty()
            && !records
                .iter()
                .any(|record| record.source_id.as_deref() == Some(manifest.document.id.as_str()))
        {
            return Err(StoreError::Integrity(
                "the ledger does not carry the posting the document produced".to_owned(),
            ));
        }

        for stored_return in &ledger.vat_returns {
            verify_return(directory, &manifest.files, stored_return, &ledger_hashes)?;
        }
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
        ledger_entries: manifest.ledger.as_ref().map_or(0, |ledger| ledger.entries),
        vat_returns: manifest.ledger.as_ref().map_or(0, |ledger| ledger.vat_returns.len()),
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
    /// The accountancy that belongs to the document. Absent in a bundle written
    /// before the ledger travelled with the export, which still verifies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ledger: Option<ManifestLedger>,
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
    /// SHA-256 of the template that laid the artefact out, when one did. Written
    /// only when it is known, so a bundle made before this was recorded still
    /// reads, and one made by the built-in layout says nothing rather than
    /// claiming a template it never used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    template_digest: Option<String>,
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

#[derive(Serialize, Deserialize)]
struct ManifestLedger {
    /// Journal entries in the bundle.
    entries: usize,
    /// SHA-256 over the ordered `(entry, content hash)` list, the same shape the
    /// VAT basis hash uses.
    content_hashes: String,
    file: String,
    sha256: String,
    #[serde(default)]
    vat_returns: Vec<ManifestVatReturn>,
}

/// A stored return whose basis the bundle carries.
#[derive(Serialize, Deserialize)]
struct ManifestVatReturn {
    id: String,
    year: i32,
    period: u8,
    status: String,
    prepared_at: String,
    mapping: String,
    basis_hash: String,
    payable_micros: i64,
    file: String,
    sha256: String,
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

/// One journal entry as the bundle stores it, one JSON object per line.
#[derive(Serialize, Deserialize)]
struct LedgerRecord {
    id: String,
    journal: String,
    number: String,
    date: String,
    description: String,
    currency: String,
    source_kind: String,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    reverses: Option<String>,
    posting_profile: String,
    rule_version: String,
    created_at: String,
    created_by: String,
    content_hash: String,
    lines: Vec<LedgerLineRecord>,
}

#[derive(Serialize, Deserialize)]
struct LedgerLineRecord {
    line_no: i64,
    account: String,
    side: String,
    amount_micros: i64,
    currency: String,
    base_amount_micros: i64,
    #[serde(default)]
    fx_rate_micros: Option<i64>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    party_id: Option<String>,
    #[serde(default)]
    tax_code_id: Option<String>,
    #[serde(default)]
    vat: Option<LedgerVat>,
}

#[derive(Serialize, Deserialize)]
struct LedgerVat {
    category: String,
    rate_bp: u16,
}

/// A stored VAT return as the bundle carries it.
#[derive(Serialize, Deserialize)]
struct StoredVatReturn {
    id: String,
    seller_id: String,
    year: i32,
    period: u8,
    status: String,
    prepared_at: String,
    basis_hash: String,
    totals: StoredVatTotals,
    lines: Vec<StoredVatLine>,
    /// The entries the figures came from, in the order the basis hash covers.
    basis: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredVatTotals {
    mapping: String,
    vat_due: i64,
    vat_deductible: i64,
    payable: i64,
    #[serde(default)]
    unmapped: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredVatLine {
    rubriek: String,
    base_micros: i64,
    vat_micros: i64,
}

/// Which entries and returns a document's bundle has to carry.
struct LedgerSelection {
    entries: BTreeSet<String>,
    returns: Vec<String>,
}

fn export_ledger(
    conn: &Connection,
    document_id: DocumentId,
    directory: &Path,
    files: &mut Vec<ManifestFile>,
) -> Result<ManifestLedger, StoreError> {
    let selection = select_ledger(conn, document_id)?;
    let records = read_ledger(conn, &selection.entries)?;

    let mut body = String::new();
    for record in &records {
        body.push_str(&serde_json::to_string(record)?);
        body.push('\n');
    }
    let ledger_file = write_file(directory, LEDGER_FILE, body.as_bytes())?;
    files.push(ledger_file.clone());

    let mut vat_returns = Vec::new();
    for id in &selection.returns {
        let stored = read_return(conn, id)?;
        fs::create_dir_all(directory.join(RETURNS_DIR))?;
        let path = format!("{RETURNS_DIR}/{}.json", safe_name(&stored.id));
        let json = serde_json::to_string_pretty(&stored)?;
        let file = write_file(directory, &path, json.as_bytes())?;
        files.push(file.clone());
        vat_returns.push(ManifestVatReturn {
            id: stored.id,
            year: stored.year,
            period: stored.period,
            status: stored.status,
            prepared_at: stored.prepared_at,
            mapping: stored.totals.mapping,
            basis_hash: stored.basis_hash,
            payable_micros: stored.totals.payable,
            file: file.path,
            sha256: file.sha256,
        });
    }
    vat_returns.sort_by(|left, right| {
        (left.year, left.period, &left.prepared_at, &left.id).cmp(&(
            right.year,
            right.period,
            &right.prepared_at,
            &right.id,
        ))
    });

    Ok(ManifestLedger {
        entries: records.len(),
        content_hashes: content_chain(&records),
        file: ledger_file.path,
        sha256: ledger_file.sha256,
        vat_returns,
    })
}

/// The entries and returns that explain this document.
///
/// The starting point is the posting the document itself produced. From there the
/// set grows both ways along corrections, then to every stored return whose basis
/// names an entry in it. A return is only explainable with its whole basis, so
/// including one pulls in the rest of its period, and those entries may name
/// returns of their own; the loop runs until neither side grows.
fn select_ledger(conn: &Connection, document_id: DocumentId) -> Result<LedgerSelection, StoreError> {
    let mut entries: BTreeSet<String> = BTreeSet::new();
    {
        let mut statement = conn.prepare(
            "SELECT id FROM journal_entry
             WHERE source_id = ?1 AND source_kind IN ('outbound_document', 'inbound_document')",
        )?;
        let mut rows = statement.query([document_id.to_string()])?;
        while let Some(row) = rows.next()? {
            entries.insert(row.get(0)?);
        }
    }

    // A reversal names the entry it undoes, and a reversal can itself be reversed.
    {
        let mut statement = conn.prepare(
            "SELECT id FROM journal_entry WHERE reverses_entry_id = ?1
             UNION
             SELECT reverses_entry_id FROM journal_entry WHERE id = ?1 AND reverses_entry_id IS NOT NULL",
        )?;
        loop {
            let mut added = false;
            for id in entries.iter().cloned().collect::<Vec<_>>() {
                let mut rows = statement.query([&id])?;
                while let Some(row) = rows.next()? {
                    added |= entries.insert(row.get(0)?);
                }
            }
            if !added {
                break;
            }
        }
    }

    let mut returns: Vec<String> = Vec::new();
    let mut naming = conn.prepare(
        "SELECT r.id FROM vat_return r
         JOIN vat_basis_posting b ON b.return_id = r.id
         WHERE b.entry_id = ?1",
    )?;
    loop {
        let mut found: Vec<String> = Vec::new();
        for id in &entries {
            let mut rows = naming.query([id])?;
            while let Some(row) = rows.next()? {
                let return_id: String = row.get(0)?;
                if !returns.contains(&return_id) && !found.contains(&return_id) {
                    found.push(return_id);
                }
            }
        }
        if found.is_empty() {
            break;
        }
        found.sort();
        for return_id in found {
            let mut basis = conn.prepare("SELECT entry_id FROM vat_basis_posting WHERE return_id = ?1")?;
            let mut rows = basis.query([&return_id])?;
            while let Some(row) = rows.next()? {
                entries.insert(row.get(0)?);
            }
            returns.push(return_id);
        }
    }

    Ok(LedgerSelection { entries, returns })
}

/// Reads the selected entries in a stable order, lines included.
fn read_ledger(conn: &Connection, ids: &BTreeSet<String>) -> Result<Vec<LedgerRecord>, StoreError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = ids.iter().cloned().collect();
    let mut records = Vec::new();
    for chunk in ids.chunks(ID_CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let sql = format!(
            "SELECT id, journal_code, number, entry_date_local, description, currency,
                    source_kind, source_id, reverses_entry_id, posting_profile_id, rule_version,
                    created_at, created_by, content_hash
             FROM journal_entry WHERE id IN ({placeholders})"
        );
        let mut statement = conn.prepare(&sql)?;
        let mut rows = statement.query(params_from_iter(chunk.iter()))?;
        while let Some(row) = rows.next()? {
            records.push(LedgerRecord {
                id: row.get(0)?,
                journal: row.get(1)?,
                number: row.get(2)?,
                date: row.get(3)?,
                description: row.get(4)?,
                currency: row.get(5)?,
                source_kind: row.get(6)?,
                source_id: row.get(7)?,
                reverses: row.get(8)?,
                posting_profile: row.get(9)?,
                rule_version: row.get(10)?,
                created_at: row.get(11)?,
                created_by: row.get(12)?,
                content_hash: row.get(13)?,
                lines: Vec::new(),
            });
        }
    }

    let by_id: BTreeMap<String, usize> = records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.id.clone(), index))
        .collect();
    for chunk in ids.chunks(ID_CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let sql = format!(
            "SELECT entry_id, line_no, account_code, side, amount_micros, currency,
                    base_amount_micros, fx_rate_micros, description, party_id, tax_code_id,
                    vat_category, vat_rate_bp
             FROM journal_line WHERE entry_id IN ({placeholders})
             ORDER BY entry_id, line_no"
        );
        let mut statement = conn.prepare(&sql)?;
        let mut rows = statement.query(params_from_iter(chunk.iter()))?;
        while let Some(row) = rows.next()? {
            let entry_id: String = row.get(0)?;
            let Some(index) = by_id.get(&entry_id).copied() else {
                continue;
            };
            let category: Option<String> = row.get(11)?;
            let rate_bp: Option<i64> = row.get(12)?;
            let vat = match (category, rate_bp) {
                (Some(category), Some(rate_bp)) => Some(LedgerVat {
                    category,
                    rate_bp: u16::try_from(rate_bp)
                        .map_err(|_| StoreError::Data(format!("VAT rate {rate_bp} does not fit in basis points")))?,
                }),
                _ => None,
            };
            records[index].lines.push(LedgerLineRecord {
                line_no: row.get(1)?,
                account: row.get(2)?,
                side: row.get(3)?,
                amount_micros: row.get(4)?,
                currency: row.get(5)?,
                base_amount_micros: row.get(6)?,
                fx_rate_micros: row.get(7)?,
                description: row.get(8)?,
                party_id: row.get(9)?,
                tax_code_id: row.get(10)?,
                vat,
            });
        }
    }

    records.sort_by(|left, right| {
        (&left.date, &left.journal, &left.number, &left.id).cmp(&(
            &right.date,
            &right.journal,
            &right.number,
            &right.id,
        ))
    });
    Ok(records)
}

fn read_return(conn: &Connection, id: &str) -> Result<StoredVatReturn, StoreError> {
    let (seller_id, year, period_no, status, prepared_at, basis_hash, totals): (
        String,
        i32,
        i64,
        String,
        String,
        String,
        String,
    ) = conn.query_row(
        "SELECT seller_id, year, period_no, status, prepared_at, basis_hash, totals_json
         FROM vat_return WHERE id = ?1",
        [id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;

    let mut statement =
        conn.prepare("SELECT rubriek, base_micros, vat_micros FROM vat_return_line WHERE return_id = ?1")?;
    let mut rows = statement.query([id])?;
    let mut lines = Vec::new();
    while let Some(row) = rows.next()? {
        lines.push(StoredVatLine {
            rubriek: row.get(0)?,
            base_micros: row.get(1)?,
            vat_micros: row.get(2)?,
        });
    }
    drop(rows);
    drop(statement);

    let mut statement =
        conn.prepare("SELECT entry_id FROM vat_basis_posting WHERE return_id = ?1 ORDER BY entry_id")?;
    let mut rows = statement.query([id])?;
    let mut basis = Vec::new();
    while let Some(row) = rows.next()? {
        basis.push(row.get::<_, String>(0)?);
    }

    Ok(StoredVatReturn {
        id: id.to_owned(),
        seller_id,
        year,
        period: u8::try_from(period_no)
            .map_err(|_| StoreError::Data(format!("VAT return {id} has period {period_no}")))?,
        status,
        prepared_at,
        basis_hash,
        totals: serde_json::from_str(&totals)?,
        lines,
        basis,
    })
}

/// Hash over an ordered `(entry, content hash)` list.
fn content_chain(records: &[LedgerRecord]) -> String {
    let mut canonical = String::new();
    for record in records {
        canonical.push_str(&record.id);
        canonical.push(':');
        canonical.push_str(&record.content_hash);
        canonical.push('\n');
    }
    audit::sha256_hex(canonical.as_bytes())
}

/// Re-derives a ledger line's content hash from the record itself.
fn record_content_hash(record: &LedgerRecord) -> Result<String, StoreError> {
    let journal = Journal::parse(&record.journal)
        .map_err(|error| StoreError::Integrity(format!("ledger entry {} names journal: {error}", record.number)))?;
    let date = LocalDate::parse_iso(&record.date)
        .map_err(|error| StoreError::Integrity(format!("ledger entry {} has a date: {error}", record.number)))?;
    let currency = Currency::parse(&record.currency)
        .map_err(|error| StoreError::Integrity(format!("ledger entry {} has a currency: {error}", record.number)))?;

    let mut lines = Vec::new();
    for line in &record.lines {
        let account = AccountCode::parse(&line.account).map_err(|error| {
            StoreError::Integrity(format!("ledger entry {} names an account: {error}", record.number))
        })?;
        let side = Side::parse(&line.side)
            .map_err(|error| StoreError::Integrity(format!("ledger entry {} has a side: {error}", record.number)))?;
        let mut ledger_line = LedgerLine::new(account, side, Money::from_micros(line.amount_micros, currency))
            .map_err(|error| StoreError::Integrity(format!("ledger entry {} has a line: {error}", record.number)))?;
        if let Some(vat) = &line.vat {
            let category = VatCategory::parse(&vat.category).map_err(|error| {
                StoreError::Integrity(format!("ledger entry {} has a VAT category: {error}", record.number))
            })?;
            let rate = VatRate::from_basis_points(vat.rate_bp).map_err(|error| {
                StoreError::Integrity(format!("ledger entry {} has a VAT rate: {error}", record.number))
            })?;
            ledger_line = ledger_line.tagged(category, rate);
        }
        lines.push(ledger_line);
    }

    let entry = LedgerEntry::new(journal, record.description.as_str(), lines).map_err(|error| {
        StoreError::Integrity(format!("ledger entry {} cannot be read back: {error}", record.number))
    })?;
    let source = posting_source(&record.source_kind, record.source_id.as_deref())?;
    crate::ledger::content_hash(&entry, &record.number, date, source)
}

/// The posting source a record names, as the hash rule sees it.
fn posting_source(kind: &str, id: Option<&str>) -> Result<PostingSource, StoreError> {
    let document = |raw: &str| {
        DocumentId::parse_str(raw).map_err(|error| StoreError::Integrity(format!("the ledger names a source: {error}")))
    };
    match (kind, id) {
        ("manual", _) => Ok(PostingSource::Manual),
        ("opening", _) => Ok(PostingSource::Opening),
        ("bank", None) => Ok(PostingSource::Bank),
        ("bank", Some(raw)) => Ok(PostingSource::BankTransaction(document(raw)?)),
        ("outbound_document", Some(raw)) => Ok(PostingSource::OutboundDocument(document(raw)?)),
        ("inbound_document", Some(raw)) => Ok(PostingSource::InboundDocument(document(raw)?)),
        (other, _) => Err(StoreError::Integrity(format!(
            "the ledger names an unknown posting source {other:?}"
        ))),
    }
}

/// Parses the ledger file. One JSON object per non-empty line.
fn parse_ledger(bytes: &[u8]) -> Result<Vec<LedgerRecord>, StoreError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| StoreError::Integrity(format!("the ledger is not text: {error}")))?;
    let mut records = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        records.push(serde_json::from_str(line)?);
    }
    Ok(records)
}

/// Checks one stored return against the ledger the bundle carries.
///
/// Three things have to hold: the file is the one the manifest names, the basis
/// hash is the hash of the entries it points at, and the figures add up the way
/// the form does. The last one is what makes the check more than a copy: a
/// forger who rebuilds every hash still has to make 5a the sum of its boxes.
fn verify_return(
    directory: &Path,
    files: &[ManifestFile],
    listed: &ManifestVatReturn,
    ledger: &BTreeMap<String, String>,
) -> Result<(), StoreError> {
    let file = files
        .iter()
        .find(|file| file.path == listed.file)
        .ok_or_else(|| StoreError::Integrity(format!("VAT return file {} is not in the manifest", listed.file)))?;
    if file.sha256 != listed.sha256 {
        return Err(StoreError::Integrity(format!(
            "VAT return {} disagrees with the file it names",
            listed.id
        )));
    }
    let stored: StoredVatReturn = serde_json::from_slice(&read_listed(directory, files, &listed.file)?)?;

    if stored.id != listed.id
        || stored.year != listed.year
        || stored.period != listed.period
        || stored.status != listed.status
        || stored.prepared_at != listed.prepared_at
        || stored.totals.mapping != listed.mapping
        || stored.basis_hash != listed.basis_hash
        || stored.totals.payable != listed.payable_micros
    {
        return Err(StoreError::Integrity(format!(
            "the VAT return file for {} does not match the manifest",
            listed.id
        )));
    }

    // The basis hash, recomputed from the entries this bundle actually carries.
    let mut canonical = String::new();
    for entry in &stored.basis {
        let Some(content) = ledger.get(entry) else {
            return Err(StoreError::Integrity(format!(
                "VAT return {} names entry {entry}, which the bundle does not carry",
                stored.id
            )));
        };
        canonical.push_str(entry);
        canonical.push(':');
        canonical.push_str(content);
        canonical.push('\n');
    }
    if audit::sha256_hex(canonical.as_bytes()) != stored.basis_hash {
        return Err(StoreError::Integrity(format!(
            "the basis of VAT return {} does not match the entries it names",
            stored.id
        )));
    }

    for line in &stored.lines {
        if Rubriek::parse(&line.rubriek).is_none() {
            return Err(StoreError::Integrity(format!(
                "VAT return {} names rubriek {:?}",
                stored.id, line.rubriek
            )));
        }
    }
    let vat_for = |rubriek: Rubriek| {
        stored
            .lines
            .iter()
            .find(|line| line.rubriek == rubriek.code())
            .map_or(0, |line| line.vat_micros)
    };

    let due_from_boxes: i64 = Rubriek::ALL
        .into_iter()
        .filter(|rubriek| rubriek.counts_toward_due())
        .map(vat_for)
        .sum();
    if due_from_boxes != stored.totals.vat_due || vat_for(Rubriek::VatDue) != stored.totals.vat_due {
        return Err(StoreError::Integrity(format!(
            "the boxes of VAT return {} add up to {due_from_boxes}, the return says {}",
            stored.id, stored.totals.vat_due
        )));
    }
    if vat_for(Rubriek::VatDeductible) != stored.totals.vat_deductible {
        return Err(StoreError::Integrity(format!(
            "the deduction of VAT return {} is {} in the boxes and {} in the totals",
            stored.id,
            vat_for(Rubriek::VatDeductible),
            stored.totals.vat_deductible
        )));
    }
    if stored.totals.payable != stored.totals.vat_due - stored.totals.vat_deductible
        || vat_for(Rubriek::VatPayable) != stored.totals.payable
    {
        return Err(StoreError::Integrity(format!(
            "the totals of VAT return {} do not add up",
            stored.id
        )));
    }

    Ok(())
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
