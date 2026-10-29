//! Reading a stored document back for display.
//!
//! The archive is write-once, so this is the only way a front end can look at an
//! issued or received document: nothing here edits one. Everything is read from
//! the rows written when the document was recorded, including the snapshots of
//! the parties, so a customer renamed next month cannot change what last month's
//! invoice says.

use crux::{
    Currency, DocType, DocumentId, InvoiceTotals, LocalDate, Money, Party, Quantity, TaxBreakdown, UnitCode,
    VatCategory, VatRate,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use crate::dto;
use crate::error::StoreError;
use crate::repo::{Direction, DocumentStatus, parse_doc_type};

/// A stored document, in the shape a screen needs it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocumentView {
    /// Document identity.
    pub id: DocumentId,
    /// Which side of the books.
    pub direction: Direction,
    /// Human number (BT-1).
    pub number: String,
    /// UN/CEFACT type (BT-3): invoice or credit note.
    pub doc_type: DocType,
    /// Document currency (BT-5).
    pub currency: Currency,
    /// Issue date (BT-2).
    pub issue_date: LocalDate,
    /// Due date (BT-9).
    pub due_date: Option<LocalDate>,
    /// Delivery date (BT-72).
    pub delivery_date: Option<LocalDate>,
    /// Buyer reference (BT-10).
    pub buyer_reference: Option<String>,
    /// Order reference (BT-13).
    pub order_reference: Option<String>,
    /// Free-text note (BT-22).
    pub note: Option<String>,
    /// Totals as they were calculated at issue or import.
    pub totals: InvoiceTotals,
    /// Issuer as it was on the day.
    pub seller: Party,
    /// Buyer as it was on the day.
    pub buyer: Party,
    /// Lines, in position order.
    pub lines: Vec<StoredLine>,
    /// Current status: the latest event, or the state the document was recorded in.
    pub status: DocumentStatus,
    /// SHA-256 of the semantic record.
    pub semantic_hash: String,
    /// When the archive recorded it.
    pub recorded_at: String,
    /// Status events, oldest first.
    pub events: Vec<StatusEvent>,
    /// Artefacts (UBL, PDF) with hashes, oldest first.
    pub artefacts: Vec<ArtefactInfo>,
}

/// One stored line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StoredLine {
    /// Position on the document (BT-126).
    pub position: i64,
    /// Item name (BT-153).
    pub name: String,
    /// Item description (BT-154).
    pub description: Option<String>,
    /// Quantity (BT-129).
    pub quantity: Quantity,
    /// Unit of measure (BT-130).
    pub unit: UnitCode,
    /// Unit price (BT-146).
    pub unit_price: Money,
    /// VAT category (BT-151).
    pub vat_category: VatCategory,
    /// VAT rate (BT-152).
    pub vat_rate: VatRate,
    /// Line net amount (BT-131), as stored.
    pub line_net: Money,
    /// Tax on the line, as stored.
    pub tax: Money,
}

/// A status change, as recorded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StatusEvent {
    /// Status code.
    pub kind: String,
    /// When it happened, RFC 3339.
    pub at: String,
    /// Who decided.
    pub actor: String,
    /// Optional detail, e.g. a payment reference.
    pub detail: Option<String>,
}

/// A stored artefact's metadata; the bytes live in `blob`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ArtefactInfo {
    /// `ubl`, `pdf`, and so on.
    pub kind: String,
    /// Syntax profile, when the kind has one.
    pub syntax_profile: Option<String>,
    /// SHA-256 of the bytes.
    pub bytes_hash: String,
    /// Size in bytes.
    pub size_bytes: i64,
    /// When it was rendered.
    pub rendered_at: String,
    /// What rendered it.
    pub renderer_version: String,
}

/// Loads a stored document for display.
///
/// `Ok(None)` means the archive holds no such document, which a stale selection
/// can produce after a restore; it is not an error.
pub fn load(conn: &Connection, id: DocumentId) -> Result<Option<DocumentView>, StoreError> {
    let document = conn
        .query_row(
            "SELECT d.id, d.direction, d.number, d.doc_type, d.currency, d.issue_date_local,
                    d.due_date_local, d.delivery_date_local, d.buyer_reference, d.order_reference,
                    d.note, d.totals_json, d.issuer_snapshot_json, d.recipient_snapshot_json,
                    d.semantic_hash, d.recorded_at, d.state,
                    COALESCE((SELECT e.kind FROM document_event e
                              WHERE e.document_id = d.id
                              ORDER BY e.seq DESC LIMIT 1), d.state) AS status
             FROM document d
             WHERE d.id = ?1",
            params![id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, String>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, String>(16)?,
                    row.get::<_, String>(17)?,
                ))
            },
        )
        .optional()?;
    let Some(document) = document else {
        return Ok(None);
    };

    let currency = parse_currency(&document.4)?;
    let totals: Value = serde_json::from_str(&document.11)?;
    let seller: Value = serde_json::from_str(&document.12)?;
    let buyer: Value = serde_json::from_str(&document.13)?;

    Ok(Some(DocumentView {
        id,
        direction: Direction::parse(&document.1).ok_or_else(|| data(format!("unknown direction {}", document.1)))?,
        number: document.2,
        doc_type: parse_doc_type(&document.3)?,
        currency,
        issue_date: parse_date(&document.5)?,
        due_date: document.6.as_deref().map(parse_date).transpose()?,
        delivery_date: document.7.as_deref().map(parse_date).transpose()?,
        buyer_reference: document.8,
        order_reference: document.9,
        note: document.10,
        totals: totals_from_json(&totals, currency)?,
        seller: dto::party_from_value(&seller)?,
        buyer: dto::party_from_value(&buyer)?,
        lines: lines(conn, id, currency)?,
        status: DocumentStatus::parse(&document.17).ok_or_else(|| data(format!("unknown status {}", document.17)))?,
        semantic_hash: document.14,
        recorded_at: document.15,
        events: events(conn, id)?,
        artefacts: artefacts(conn, id)?,
    }))
}

fn lines(conn: &Connection, id: DocumentId, currency: Currency) -> Result<Vec<StoredLine>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT position, payload_json, line_net_micros, tax_micros
         FROM document_line
         WHERE document_id = ?1
         ORDER BY position",
    )?;
    let rows = statement.query_map(params![id.to_string()], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;

    let mut lines = Vec::new();
    for row in rows {
        let (position, payload, line_net, tax) = row?;
        let payload: Value = serde_json::from_str(&payload)?;
        let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_owned);
        let integer = |key: &str| payload.get(key).and_then(Value::as_i64);

        let name = text("name").ok_or_else(|| data("line has no name".to_owned()))?;
        let unit = text("unit").ok_or_else(|| data("line has no unit".to_owned()))?;
        let category = text("vat_category").ok_or_else(|| data("line has no VAT category".to_owned()))?;
        let rate = integer("vat_rate_bp").ok_or_else(|| data("line has no VAT rate".to_owned()))?;
        let quantity = integer("quantity_micros").ok_or_else(|| data("line has no quantity".to_owned()))?;
        let unit_price = integer("unit_price_micros").ok_or_else(|| data("line has no unit price".to_owned()))?;

        lines.push(StoredLine {
            position,
            name,
            description: text("description"),
            quantity: Quantity::from_micros(quantity),
            unit: UnitCode::parse(&unit).map_err(data)?,
            unit_price: Money::from_micros(unit_price, currency),
            vat_category: VatCategory::parse(&category).map_err(data)?,
            vat_rate: VatRate::from_basis_points(u16::try_from(rate).map_err(data)?).map_err(data)?,
            line_net: Money::from_micros(line_net, currency),
            tax: Money::from_micros(tax, currency),
        });
    }
    Ok(lines)
}

fn events(conn: &Connection, id: DocumentId) -> Result<Vec<StatusEvent>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT kind, at, actor, detail
         FROM document_event
         WHERE document_id = ?1
         ORDER BY seq",
    )?;
    let rows = statement.query_map(params![id.to_string()], |row| {
        Ok(StatusEvent {
            kind: row.get(0)?,
            at: row.get(1)?,
            actor: row.get(2)?,
            detail: row.get(3)?,
        })
    })?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row?);
    }
    Ok(events)
}

/// Loads the bytes of a stored artefact, checking them against the hash.
///
/// The latest rendition of `kind` wins, so a PDF attached after the UBL is the
/// one a reader gets. The hash is verified here rather than at the call site:
/// an artefact that no longer matches its record must not be handed to anyone
/// as the invoice.
pub fn artefact_bytes(conn: &Connection, id: DocumentId, kind: &str) -> Result<Vec<u8>, StoreError> {
    let found: Option<(String, Vec<u8>)> = conn
        .query_row(
            "SELECT a.bytes_hash, b.payload
             FROM artefact a
             JOIN blob b ON b.id = a.blob_id
             WHERE a.document_id = ?1 AND a.kind = ?2
             ORDER BY a.rendered_at DESC, a.id DESC
             LIMIT 1",
            params![id.to_string(), kind],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (hash, bytes) = found.ok_or_else(|| StoreError::NotFound(format!("{kind} artefact for document {id}")))?;
    if crate::audit::sha256_hex(&bytes) != hash {
        return Err(data(format!(
            "the stored {kind} no longer hashes to its record ({hash})"
        )));
    }
    Ok(bytes)
}

/// Distinct artefact kinds a document carries, in a stable order.
pub fn artefact_kinds(conn: &Connection, id: DocumentId) -> Result<Vec<String>, StoreError> {
    let mut statement = conn.prepare("SELECT DISTINCT kind FROM artefact WHERE document_id = ?1 ORDER BY kind")?;
    let rows = statement.query_map(params![id.to_string()], |row| row.get::<_, String>(0))?;
    let mut kinds = Vec::new();
    for row in rows {
        kinds.push(row?);
    }
    Ok(kinds)
}

fn artefacts(conn: &Connection, id: DocumentId) -> Result<Vec<ArtefactInfo>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT kind, syntax_profile, bytes_hash, size_bytes, rendered_at, renderer_version
         FROM artefact
         WHERE document_id = ?1
         ORDER BY rendered_at, kind",
    )?;
    let rows = statement.query_map(params![id.to_string()], |row| {
        Ok(ArtefactInfo {
            kind: row.get(0)?,
            syntax_profile: row.get(1)?,
            bytes_hash: row.get(2)?,
            size_bytes: row.get(3)?,
            rendered_at: row.get(4)?,
            renderer_version: row.get(5)?,
        })
    })?;
    let mut artefacts = Vec::new();
    for row in rows {
        artefacts.push(row?);
    }
    Ok(artefacts)
}

/// Rebuilds `totals_json`, whose amounts are decimal strings so that money never
/// round-trips through a float.
fn totals_from_json(value: &Value, currency: Currency) -> Result<InvoiceTotals, StoreError> {
    let amount = |field: &str, holder: &Value| -> Result<Money, StoreError> {
        let raw = holder
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| data(format!("totals have no {field}")))?;
        Money::parse_fixed(raw, currency).map_err(data)
    };

    let mut breakdown = Vec::new();
    for row in value
        .get("breakdown")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let category = row
            .get("category")
            .and_then(Value::as_str)
            .ok_or_else(|| data("breakdown row has no category".to_owned()))?;
        let rate_bp = row
            .get("rate_bp")
            .and_then(Value::as_u64)
            .ok_or_else(|| data("breakdown row has no rate".to_owned()))?;
        breakdown.push(TaxBreakdown {
            category: VatCategory::parse(category).map_err(data)?,
            rate_bp: u16::try_from(rate_bp).map_err(data)?,
            taxable_base: amount("taxable_base", row)?,
            tax: amount("tax", row)?,
        });
    }

    Ok(InvoiceTotals {
        net: amount("net", value)?,
        tax: amount("tax", value)?,
        gross: amount("gross", value)?,
        breakdown,
    })
}

fn parse_date(raw: &str) -> Result<LocalDate, StoreError> {
    LocalDate::parse_iso(raw).map_err(data)
}

fn parse_currency(raw: &str) -> Result<Currency, StoreError> {
    Currency::parse(raw).map_err(data)
}

fn data(message: impl ToString) -> StoreError {
    StoreError::Data(message.to_string())
}

#[cfg(test)]
mod tests {
    use crux::{Address, CountryCode};

    use super::*;

    #[test]
    fn a_party_snapshot_round_trips() {
        let mut party = Party::new("Buyer BV", Address::new("Rotterdam", CountryCode::NL));
        party.vat_id = Some("NL123456782B01".to_owned());
        party.email = Some("books@example.test".to_owned());

        let stored = dto::party_to_value(&party).unwrap();
        assert_eq!(dto::party_from_value(&stored).unwrap(), party);
    }
}
