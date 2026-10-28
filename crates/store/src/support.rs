//! Support bundles: what a maintainer needs, and nothing about the business.
//!
//! The bundle is **assembled from typed facts** — counts, versions, sizes, and
//! hashes — so there is no channel through which a party name, an amount, an
//! invoice number, or a file path could reach it. That is the redaction policy:
//! not a filter over free text that might leak, but an assembly that has nothing
//! sensitive to filter in the first place.
//!
//! One hash does travel: the audit chain head, which is a digest and is what
//! lets a maintainer compare two archives without either of them being shipped.
//!
//! Paths are never included. A path carries the operating-system user name, and a
//! support bundle is often pasted into a public issue tracker.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::Serialize;

use crate::audit::{self, ChainHead};
use crate::db::Database;
use crate::error::StoreError;
use crate::integrity::{self, IntegrityReport};

/// Marker in `summary.json`.
pub const FORMAT: &str = "bonk-support";
/// Bundle layout version.
pub const VERSION: u32 = 1;

const SUMMARY_JSON: &str = "summary.json";
const SUMMARY_TEXT: &str = "summary.txt";

/// What to describe.
pub struct SupportRequest<'a> {
    /// Archive file. Only its size is reported; the path itself is never written.
    pub database_path: &'a Path,
    /// Keystore file. Only its size is reported.
    pub keystore_path: &'a Path,
    /// Version of the running binary.
    pub app_version: &'a str,
    /// Directory to write into; it must not already hold a bundle.
    pub directory: &'a Path,
    /// Generation stamp.
    pub now: &'a str,
}

/// The bundle's contents, before rendering.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct SupportSummary {
    /// Marker.
    pub format: String,
    /// Layout version.
    pub format_version: u32,
    /// When it was generated.
    pub generated_at: String,
    /// Version of the binary that wrote it.
    pub app_version: String,
    /// Operating system and architecture.
    pub platform: String,
    /// Applied schema version.
    pub schema_version: i64,
    /// `sqlcipher` or `none (test build)`.
    pub encryption: String,
    /// Size of the archive file.
    pub database_bytes: u64,
    /// Size of the keystore file.
    pub keystore_bytes: u64,
    /// Row counts.
    pub counts: Counts,
    /// Result of the whole-archive audit.
    pub integrity: IntegrityReport,
    /// Newest audit link, when the chain has one.
    pub chain: Option<ChainHead>,
    /// Audit events by kind, sorted.
    pub audit_kinds: Vec<KindCount>,
}

/// Row counts, which are diagnostics without describing any row.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Counts {
    /// Sellers.
    pub sellers: i64,
    /// Customers and suppliers (one table).
    pub customers: i64,
    /// Open drafts.
    pub drafts: i64,
    /// Documents in either direction.
    pub documents: i64,
    /// Documents we issued.
    pub outbound: i64,
    /// Documents we received.
    pub inbound: i64,
    /// Import provenance records.
    pub imports: i64,
    /// Stored artefacts.
    pub artefacts: i64,
    /// Stored payloads.
    pub blobs: i64,
    /// Audit events.
    pub audit_events: i64,
}

/// One audit kind and how often it appears.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct KindCount {
    /// Event kind, from a fixed vocabulary.
    pub kind: String,
    /// Occurrences.
    pub count: i64,
}

/// Writes a support bundle and returns what it described.
///
/// The directory must not already hold a bundle: a support bundle is a snapshot,
/// and overwriting one silently is how a report stops matching the archive it
/// came from.
pub fn write(database: &Database, request: &SupportRequest<'_>) -> Result<SupportSummary, StoreError> {
    let directory = request.directory;
    if directory.join(SUMMARY_JSON).exists() {
        return Err(StoreError::Rejected(format!(
            "{} already contains a support bundle",
            directory.display()
        )));
    }

    let summary = describe(database.connection(), request)?;
    fs::create_dir_all(directory)?;
    fs::write(directory.join(SUMMARY_JSON), serde_json::to_string_pretty(&summary)?)?;
    fs::write(directory.join(SUMMARY_TEXT), summary.render_text())?;
    Ok(summary)
}

/// Collects the facts without writing anything.
pub fn describe(conn: &Connection, request: &SupportRequest<'_>) -> Result<SupportSummary, StoreError> {
    let counts = conn.query_row(
        "SELECT (SELECT count(*) FROM seller),
                (SELECT count(*) FROM customer),
                (SELECT count(*) FROM invoice_draft),
                (SELECT count(*) FROM document),
                (SELECT count(*) FROM document WHERE direction = 'outbound'),
                (SELECT count(*) FROM document WHERE direction = 'inbound'),
                (SELECT count(*) FROM import_record),
                (SELECT count(*) FROM artefact),
                (SELECT count(*) FROM blob),
                (SELECT count(*) FROM audit_event)",
        [],
        |row| {
            Ok(Counts {
                sellers: row.get(0)?,
                customers: row.get(1)?,
                drafts: row.get(2)?,
                documents: row.get(3)?,
                outbound: row.get(4)?,
                inbound: row.get(5)?,
                imports: row.get(6)?,
                artefacts: row.get(7)?,
                blobs: row.get(8)?,
                audit_events: row.get(9)?,
            })
        },
    )?;

    let schema_version: Option<i64> =
        conn.query_row("SELECT max(version) FROM schema_migration", [], |row| row.get(0))?;

    let mut statement = conn.prepare("SELECT kind, count(*) FROM audit_event GROUP BY kind ORDER BY kind")?;
    let audit_kinds = statement
        .query_map([], |row| {
            Ok(KindCount {
                kind: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(SupportSummary {
        format: FORMAT.to_owned(),
        format_version: VERSION,
        generated_at: request.now.to_owned(),
        app_version: request.app_version.to_owned(),
        platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        schema_version: schema_version.unwrap_or(0),
        encryption: crate::encryption_label().to_owned(),
        database_bytes: file_size(request.database_path),
        keystore_bytes: file_size(request.keystore_path),
        counts,
        integrity: integrity::audit(conn)?,
        chain: audit::head(conn)?,
        audit_kinds,
    })
}

/// The human-readable rendering, which is also what the command prints.
impl SupportSummary {
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str("bonk support bundle\n");
        out.push_str(&format!("generated:  {}\n", self.generated_at));
        out.push_str(&format!("version:    {} ({})\n", self.app_version, self.platform));
        out.push_str(&format!("schema:     {}\n", self.schema_version));
        out.push_str(&format!("encryption: {}\n", self.encryption));
        out.push_str(&format!("database:   {} bytes\n", self.database_bytes));
        out.push_str(&format!("keystore:   {} bytes\n", self.keystore_bytes));
        out.push('\n');
        out.push_str(&format!("sellers:    {}\n", self.counts.sellers));
        out.push_str(&format!("customers:  {}\n", self.counts.customers));
        out.push_str(&format!("drafts:     {}\n", self.counts.drafts));
        out.push_str(&format!(
            "documents:  {} ({} outbound, {} inbound)\n",
            self.counts.documents, self.counts.outbound, self.counts.inbound
        ));
        out.push_str(&format!("imports:    {}\n", self.counts.imports));
        out.push_str(&format!("artefacts:  {}\n", self.counts.artefacts));
        out.push_str(&format!(
            "blobs:      {} ({} unreferenced)\n",
            self.counts.blobs, self.integrity.orphaned_blobs
        ));
        match &self.chain {
            Some(head) => {
                out.push_str(&format!(
                    "audit:      {} events, head {} {} ({})\n",
                    self.counts.audit_events, head.seq, head.hash, head.at
                ));
            }
            None => out.push_str("audit:      empty chain\n"),
        }
        if !self.audit_kinds.is_empty() {
            let kinds = self
                .audit_kinds
                .iter()
                .map(|entry| format!("{} {}", entry.kind, entry.count))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("kinds:      {kinds}\n"));
        }
        out.push('\n');
        out.push_str("No names, amounts, numbers, identifiers, or paths are included.\n");
        out
    }
}

/// Size of a file, or zero when it is not readable.
///
/// A support bundle describes the trouble it was asked for; failing because the
/// keystore sits elsewhere would defeat the purpose.
fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|metadata| metadata.len()).unwrap_or(0)
}

/// Where a default bundle would go, beside an archive.
pub fn default_directory(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .map(|parent| parent.join("support-bundle"))
        .unwrap_or_else(|| PathBuf::from("support-bundle"))
}
