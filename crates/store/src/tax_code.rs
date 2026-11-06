//! Tax codes: the treatment a purchase is filed under.
//!
//! An EN 16931 category is not always enough to place a purchase's VAT. `AE`
//! covers reverse-charged goods and services alike, and the Dutch return asks for
//! them in different boxes (4b and 4a), while nothing in the document says which
//! one it is. So a person decides, and the decision gets a name: a tax code.
//!
//! Sales need no code. The category and the rate already decide the box, and a
//! code that only repeated them would be a column nobody chooses from.
//!
//! The catalogue is a versioned artefact rather than constants in code, because
//! the boxes a tax form asks for change with the tax year. Which treatment a
//! document is filed under is a separate fact with an author and a time, like the
//! supplier match: the document itself is immutable. The posting snapshots the
//! resolved code on its lines, so a report never has to ask the document again.

use crux::{DocumentId, Journal, VatCategory};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::audit;
use crate::error::StoreError;
use crate::vat::Rubriek;

/// One entry of the tax-code catalogue, as stored.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TaxCodeRecord {
    /// The stable identifier, e.g. `NL-AE-GOODS`.
    pub code: crux::TaxCode,
    /// What a person reads.
    pub name: String,
    /// The EN 16931 category it resolves.
    pub category: VatCategory,
    /// The journal whose postings it applies to.
    pub journal: Journal,
    /// The box its base and VAT land in.
    pub rubriek: Rubriek,
    /// The release of the mapping that introduced it.
    pub mapping_version: String,
}

/// Every code in the catalogue, by identifier.
pub fn list(conn: &Connection) -> Result<Vec<TaxCodeRecord>, StoreError> {
    let mut statement =
        conn.prepare("SELECT id, label, vat_category, journal, rubriek, mapping_version FROM tax_code ORDER BY id")?;
    let mut rows = statement.query([])?;
    let mut codes = Vec::new();
    while let Some(row) = rows.next()? {
        codes.push(read(row)?);
    }
    Ok(codes)
}

/// One code, if the catalogue has it.
pub fn load(conn: &Connection, code: &crux::TaxCode) -> Result<Option<TaxCodeRecord>, StoreError> {
    let mut statement =
        conn.prepare("SELECT id, label, vat_category, journal, rubriek, mapping_version FROM tax_code WHERE id = ?1")?;
    let mut rows = statement.query([code.as_str()])?;
    match rows.next()? {
        Some(row) => Ok(Some(read(row)?)),
        None => Ok(None),
    }
}

/// The treatment a document is filed under, when one was chosen.
pub fn assigned(conn: &Connection, document_id: DocumentId) -> Result<Option<TaxCodeRecord>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT t.id, t.label, t.vat_category, t.journal, t.rubriek, t.mapping_version
         FROM document_tax_code d JOIN tax_code t ON t.id = d.code
         WHERE d.document_id = ?1",
    )?;
    let mut rows = statement.query([document_id.to_string()])?;
    match rows.next()? {
        Some(row) => Ok(Some(read(row)?)),
        None => Ok(None),
    }
}

/// Chooses the treatment a received document is filed under.
///
/// Refused once the document is posted: the treatment is part of the entry that
/// was written, and entries are append-only. The moment to classify a purchase is
/// between recording it and posting it, which is also when a person has the
/// invoice in front of them.
pub fn assign(
    conn: &Connection,
    document_id: DocumentId,
    code: &str,
    actor: &str,
    now: &str,
) -> Result<TaxCodeRecord, StoreError> {
    let code = crux::TaxCode::parse(code).map_err(|error| StoreError::Rejected(error.to_string()))?;

    let transaction = conn.unchecked_transaction()?;
    let breakdown = unposted_inbound(&transaction, document_id)?;
    let record = load(&transaction, &code)?.ok_or_else(|| StoreError::NotFound(format!("tax code {code}")))?;
    if record.journal != Journal::Purchase {
        return Err(StoreError::Rejected(format!(
            "tax code {code} applies to the {} journal and cannot be put on a purchase",
            record.journal.code()
        )));
    }
    if !has_category(&breakdown, record.category)? {
        return Err(StoreError::Rejected(format!(
            "document {document_id} has no {} line, so {code} would change nothing",
            record.category.code()
        )));
    }

    transaction.execute(
        "INSERT INTO document_tax_code (document_id, code, decided_by, decided_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(document_id) DO UPDATE SET
             code = excluded.code,
             decided_by = excluded.decided_by,
             decided_at = excluded.decided_at",
        params![document_id.to_string(), code.as_str(), actor, now],
    )?;
    audit::append(
        &transaction,
        now,
        actor,
        "tax_code_assigned",
        "document",
        &document_id.to_string(),
        json!({ "code": code.as_str(), "rubriek": record.rubriek.code() }),
    )?;
    transaction.commit()?;
    Ok(record)
}

/// Returns a document to the treatment its own category implies.
pub fn clear(conn: &Connection, document_id: DocumentId, actor: &str, now: &str) -> Result<(), StoreError> {
    let transaction = conn.unchecked_transaction()?;
    unposted_inbound(&transaction, document_id)?;
    let removed = transaction.execute(
        "DELETE FROM document_tax_code WHERE document_id = ?1",
        [document_id.to_string()],
    )?;
    if removed == 0 {
        return Err(StoreError::NotFound(format!(
            "document {document_id} has no tax code assigned"
        )));
    }
    audit::append(
        &transaction,
        now,
        actor,
        "tax_code_cleared",
        "document",
        &document_id.to_string(),
        json!({}),
    )?;
    transaction.commit()?;
    Ok(())
}

/// Reads a document's breakdown, refusing one that cannot be classified.
fn unposted_inbound(conn: &Connection, document_id: DocumentId) -> Result<String, StoreError> {
    let (direction, breakdown): (String, String) = conn
        .query_row(
            "SELECT direction, tax_breakdown_json FROM document WHERE id = ?1",
            [document_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("document {document_id}")))?;
    if direction != "inbound" {
        return Err(StoreError::Rejected(
            "only received documents have a treatment to choose; a sale is placed by its own category and rate"
                .to_owned(),
        ));
    }
    let posted: Option<String> = conn
        .query_row(
            "SELECT id FROM journal_entry WHERE source_kind = 'inbound_document' AND source_id = ?1",
            [document_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    if posted.is_some() {
        return Err(StoreError::Rejected(format!(
            "document {document_id} is already posted; its treatment is fixed in the entry that was written"
        )));
    }
    Ok(breakdown)
}

/// True when the stored breakdown has a row of this category.
fn has_category(breakdown: &str, category: VatCategory) -> Result<bool, StoreError> {
    let rows: serde_json::Value = serde_json::from_str(breakdown)?;
    let rows = rows
        .as_array()
        .ok_or_else(|| StoreError::Data("stored tax breakdown is not a list".to_owned()))?;
    Ok(rows
        .iter()
        .any(|row| row.get("category").and_then(serde_json::Value::as_str) == Some(category.code())))
}

fn read(row: &rusqlite::Row<'_>) -> Result<TaxCodeRecord, StoreError> {
    let code: String = row.get(0)?;
    let category: String = row.get(2)?;
    let journal: String = row.get(3)?;
    let rubriek: String = row.get(4)?;
    Ok(TaxCodeRecord {
        code: crux::TaxCode::parse(&code).map_err(|error| StoreError::Data(error.to_string()))?,
        name: row.get(1)?,
        category: VatCategory::parse(&category).map_err(|error| StoreError::Data(error.to_string()))?,
        journal: Journal::parse(&journal).map_err(|error| StoreError::Data(error.to_string()))?,
        rubriek: Rubriek::parse(&rubriek)
            .ok_or_else(|| StoreError::Data(format!("tax code {code} names rubriek {rubriek:?}")))?,
        mapping_version: row.get(5)?,
    })
}
