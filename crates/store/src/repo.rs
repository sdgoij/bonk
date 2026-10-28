//! Repositories for the M1 vertical slice: parties, drafts, issuance.
//!
//! Issuing is one transaction: allocate the number, snapshot the parties, write
//! the document and its lines, store the rendered artefact, and append the
//! audit events. A crash leaves either all of it or none of it.

use crux::{
    Currency, CustomerId, DocType, DocumentId, DraftLine, InvoiceDraft, InvoiceTotals, LineId, LocalDate, Money, Party,
    Quantity, SellerId, TaxTreatment, UnitCode, VatCategory, VatRate, calculate_totals,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::audit;
use crate::dto;
use crate::error::StoreError;

/// A seller as stored.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SellerRecord {
    /// Identity.
    pub id: SellerId,
    /// Party data.
    pub party: Party,
    /// Default document currency.
    pub default_currency: Currency,
    /// Default locale.
    pub default_locale: String,
}

/// A customer as stored.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CustomerRecord {
    /// Identity.
    pub id: CustomerId,
    /// Party data.
    pub party: Party,
}

/// A recorded suppression of a non-blocking validation warning.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Suppression {
    /// Rule code, e.g. `T1-013`.
    pub code: String,
    /// Field path the finding pointed at.
    pub field: String,
    /// Why the warning was accepted.
    pub reason: String,
}

/// Request to issue a draft.
pub struct IssueRequest<'a> {
    /// Draft to issue.
    pub draft_id: DocumentId,
    /// Who triggered the issue.
    pub actor: &'a str,
    /// Device that issued it.
    pub device: &'a str,
    /// Renderer identifier recorded on the artefact.
    pub renderer_version: &'a str,
    /// RFC 3339 timestamp.
    pub now: &'a str,
}

/// Everything the renderer needs to serialise the document.
pub struct IssueView<'a> {
    /// Newly allocated document identity.
    pub id: DocumentId,
    /// Allocated human number.
    pub number: &'a str,
    /// Draft being issued.
    pub draft: &'a InvoiceDraft,
    /// Seller record.
    pub seller: &'a SellerRecord,
    /// Customer record.
    pub customer: &'a CustomerRecord,
    /// Calculated totals.
    pub totals: &'a InvoiceTotals,
}

/// Result of issuing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocumentRecord {
    /// Document identity.
    pub id: DocumentId,
    /// Human number.
    pub number: String,
    /// Issue date.
    pub issue_date: LocalDate,
    /// Totals.
    pub totals: InvoiceTotals,
    /// SHA-256 of the rendered UBL.
    pub ubl_sha256: String,
    /// New audit chain head.
    pub audit_head: String,
}

/// Inserts a seller.
pub fn insert_seller(conn: &Connection, seller: &SellerRecord, now: &str) -> Result<(), StoreError> {
    let address = dto::address_to_json(&seller.party.address)?;
    let contact = dto::contact_to_json(&seller.party)?;
    conn.execute(
        "INSERT INTO seller
             (id, legal_name, trading_name, vat_id, kvk_number, peppol_id, eas_code,
              address_json, contact_json, bank_json, default_currency, default_locale,
              logo_blob_id, created_at, updated_at, revision, hlc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, NULL, ?9, ?10, NULL, ?11, ?11, 1, '')",
        params![
            seller.id.to_string(),
            seller.party.legal_name,
            seller.party.trading_name,
            seller.party.vat_id,
            seller.party.kvk_number,
            seller.party.peppol_id,
            address,
            contact,
            seller.default_currency.as_str(),
            seller.default_locale,
            now,
        ],
    )?;
    Ok(())
}

/// Loads a seller.
pub fn load_seller(conn: &Connection, id: SellerId) -> Result<SellerRecord, StoreError> {
    let key = id.to_string();
    let row = conn
        .query_row(
            "SELECT legal_name, trading_name, vat_id, kvk_number, peppol_id,
                    address_json, contact_json, default_currency, default_locale
             FROM seller WHERE id = ?1",
            [&key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("seller {id}")))?;

    let (legal_name, trading_name, vat_id, kvk_number, peppol_id, address, contact, currency, locale) = row;
    let contact = dto::contact_from_json(contact.as_deref())?;
    Ok(SellerRecord {
        id,
        party: Party {
            legal_name,
            trading_name,
            vat_id,
            kvk_number,
            peppol_id,
            address: dto::address_from_json(&address)?,
            contact_name: contact.name,
            email: contact.email,
            phone: contact.phone,
        },
        default_currency: Currency::parse(&currency).map_err(|error| StoreError::Data(error.to_string()))?,
        default_locale: locale,
    })
}

/// Inserts a customer.
pub fn insert_customer(conn: &Connection, customer: &CustomerRecord, now: &str) -> Result<(), StoreError> {
    let address = dto::address_to_json(&customer.party.address)?;
    let contact = dto::contact_to_json(&customer.party)?;
    let display_name = customer
        .party
        .trading_name
        .clone()
        .unwrap_or_else(|| customer.party.legal_name.clone());
    conn.execute(
        "INSERT INTO customer
             (id, display_name, legal_name, vat_id, kvk_number, peppol_id,
              address_json, contact_json, buyer_reference, payment_terms_days,
              archived, created_at, updated_at, revision, hlc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, 0, ?9, ?9, 0, '')",
        params![
            customer.id.to_string(),
            display_name,
            customer.party.legal_name,
            customer.party.vat_id,
            customer.party.kvk_number,
            customer.party.peppol_id,
            address,
            contact,
            now,
        ],
    )?;
    Ok(())
}

/// Loads a customer.
pub fn load_customer(conn: &Connection, id: CustomerId) -> Result<CustomerRecord, StoreError> {
    let key = id.to_string();
    let row = conn
        .query_row(
            "SELECT legal_name, vat_id, kvk_number, peppol_id, address_json, contact_json
             FROM customer WHERE id = ?1",
            [&key],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("customer {id}")))?;

    let (legal_name, vat_id, kvk_number, peppol_id, address, contact) = row;
    let contact = dto::contact_from_json(contact.as_deref())?;
    Ok(CustomerRecord {
        id,
        party: Party {
            legal_name: legal_name.unwrap_or_default(),
            trading_name: None,
            vat_id,
            kvk_number,
            peppol_id,
            address: dto::address_from_json(&address)?,
            contact_name: contact.name,
            email: contact.email,
            phone: contact.phone,
        },
    })
}

/// Inserts a draft and its lines atomically.
pub fn insert_draft(conn: &Connection, draft: &InvoiceDraft, now: &str) -> Result<(), StoreError> {
    save_draft(conn, draft, now)
}

/// Creates or updates a draft and replaces its lines atomically.
pub fn save_draft(conn: &Connection, draft: &InvoiceDraft, now: &str) -> Result<(), StoreError> {
    let transaction = conn.unchecked_transaction()?;
    let notes = draft.note.as_ref().map(|note| json!({ "note": note }).to_string());
    let preceding = draft.preceding_invoice.as_ref();
    transaction.execute(
        "INSERT INTO invoice_draft
             (id, owner_id, counterparty_id, doc_type, currency, issue_date_local,
              due_date_local, delivery_date_local, buyer_reference, order_reference,
              project_reference, notes_json, payment_terms_days, raw_input_json,
              revision, hlc, created_at, updated_at,
              preceding_invoice_id, preceding_number, preceding_issue_date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, NULL, NULL, 0, '', ?12, ?12,
                 ?13, ?14, ?15)
         ON CONFLICT(id) DO UPDATE SET
             owner_id = excluded.owner_id,
             counterparty_id = excluded.counterparty_id,
             doc_type = excluded.doc_type,
             currency = excluded.currency,
             issue_date_local = excluded.issue_date_local,
             due_date_local = excluded.due_date_local,
             delivery_date_local = excluded.delivery_date_local,
             buyer_reference = excluded.buyer_reference,
             order_reference = excluded.order_reference,
             notes_json = excluded.notes_json,
             preceding_invoice_id = excluded.preceding_invoice_id,
             preceding_number = excluded.preceding_number,
             preceding_issue_date = excluded.preceding_issue_date,
             updated_at = excluded.updated_at,
             revision = invoice_draft.revision + 1",
        params![
            draft.id.to_string(),
            draft.seller_id.to_string(),
            draft.customer_id.to_string(),
            draft.doc_type.code(),
            draft.currency.as_str(),
            draft.issue_date.to_iso_string(),
            draft.due_date.map(LocalDate::to_iso_string),
            draft.delivery_date.map(LocalDate::to_iso_string),
            draft.buyer_reference,
            draft.order_reference,
            notes,
            now,
            preceding.map(|preceding| preceding.id.to_string()),
            preceding.map(|preceding| preceding.number.as_str()),
            preceding.map(|preceding| preceding.issue_date.to_iso_string()),
        ],
    )?;

    transaction.execute(
        "DELETE FROM invoice_draft_line WHERE draft_id = ?1",
        [draft.id.to_string()],
    )?;
    for line in &draft.lines {
        insert_line(&transaction, draft.id, line)?;
    }

    transaction.commit()?;
    Ok(())
}

/// Records a suppression for a non-blocking finding on a draft.
///
/// Suppressions are scoped to the draft, rule code, and field path. The reason
/// is mandatory and is appended to the audit chain.
pub fn save_suppression(
    conn: &Connection,
    draft_id: DocumentId,
    code: &str,
    field: &str,
    reason: &str,
    actor: &str,
    now: &str,
) -> Result<(), StoreError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(StoreError::Data("a suppression reason is required".to_owned()));
    }

    let transaction = conn.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO validation_suppression
             (id, draft_id, rule_code, field_path, reason, created_at, actor)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(draft_id, rule_code, field_path) DO UPDATE SET
             reason = excluded.reason,
             created_at = excluded.created_at,
             actor = excluded.actor",
        params![
            DocumentId::new().to_string(),
            draft_id.to_string(),
            code,
            field,
            reason,
            now,
            actor,
        ],
    )?;
    audit::append(
        &transaction,
        now,
        actor,
        "finding_suppressed",
        "invoice_draft",
        &draft_id.to_string(),
        json!({ "code": code, "field": field, "reason": reason }),
    )?;
    transaction.commit()?;
    Ok(())
}

/// Lists the recorded suppressions for a draft.
pub fn list_suppressions(conn: &Connection, draft_id: DocumentId) -> Result<Vec<Suppression>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT rule_code, field_path, reason
         FROM validation_suppression
         WHERE draft_id = ?1
         ORDER BY created_at, rule_code, field_path",
    )?;
    let mut rows = statement.query([draft_id.to_string()])?;
    let mut suppressions = Vec::new();
    while let Some(row) = rows.next()? {
        suppressions.push(Suppression {
            code: row.get(0)?,
            field: row.get(1)?,
            reason: row.get(2)?,
        });
    }
    Ok(suppressions)
}

fn insert_line(conn: &Connection, draft_id: DocumentId, line: &DraftLine) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO invoice_draft_line
             (id, draft_id, position, name, description, quantity_micros, unit_code,
              unit_price_micros, price_base_qty_micros, vat_category, vat_rate_bp,
              vat_exemption_reason, vat_exemption_reason_code, tax_code_id, item_ref,
              deleted, revision, hlc)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL, NULL, 0, 0, '')",
        params![
            line.id.to_string(),
            draft_id.to_string(),
            line.position,
            line.name,
            line.description,
            line.quantity.micros(),
            line.unit.as_str(),
            line.unit_price.micros(),
            line.price_base_quantity.micros(),
            line.treatment.category.code(),
            line.treatment.rate.basis_points(),
            line.treatment.exemption_reason,
            line.treatment.exemption_reason_code,
        ],
    )?;
    Ok(())
}

/// Loads the first seller, if any.
pub fn first_seller(conn: &Connection) -> Result<Option<SellerRecord>, StoreError> {
    let id: Option<String> = conn
        .query_row("SELECT id FROM seller ORDER BY created_at LIMIT 1", [], |row| {
            row.get(0)
        })
        .optional()?;
    match id {
        Some(raw) => {
            let id = SellerId::parse_str(&raw).map_err(|error| StoreError::Data(error.to_string()))?;
            Ok(Some(load_seller(conn, id)?))
        }
        None => Ok(None),
    }
}

/// Lists customers, ordered by display name.
pub fn list_customers(conn: &Connection) -> Result<Vec<CustomerRecord>, StoreError> {
    let mut statement = conn.prepare("SELECT id FROM customer WHERE archived = 0 ORDER BY display_name")?;
    let mut rows = statement.query([])?;
    let mut customers = Vec::new();
    while let Some(row) = rows.next()? {
        let raw: String = row.get(0)?;
        let id = CustomerId::parse_str(&raw).map_err(|error| StoreError::Data(error.to_string()))?;
        customers.push(load_customer(conn, id)?);
    }
    Ok(customers)
}

/// Loads a draft, if it exists.
pub fn load_draft(conn: &Connection, id: DocumentId) -> Result<Option<InvoiceDraft>, StoreError> {
    let key = id.to_string();
    let header = conn
        .query_row(
            "SELECT owner_id, counterparty_id, doc_type, currency, issue_date_local,
                    due_date_local, delivery_date_local, buyer_reference, order_reference, notes_json,
                    preceding_invoice_id, preceding_number, preceding_issue_date
             FROM invoice_draft WHERE id = ?1",
            [&key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                ))
            },
        )
        .optional()?;

    let Some((
        seller_id,
        customer_id,
        doc_type,
        currency,
        issue_date,
        due_date,
        delivery_date,
        buyer_reference,
        order_reference,
        notes,
        preceding_id,
        preceding_number,
        preceding_issue_date,
    )) = header
    else {
        return Ok(None);
    };

    let note = notes.and_then(|raw| {
        serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|value| value.get("note")?.as_str().map(str::to_owned))
    });

    let mut draft = InvoiceDraft {
        id,
        seller_id: SellerId::parse_str(&seller_id).map_err(|error| StoreError::Data(error.to_string()))?,
        customer_id: CustomerId::parse_str(&customer_id).map_err(|error| StoreError::Data(error.to_string()))?,
        doc_type: parse_doc_type(&doc_type)?,
        currency: Currency::parse(&currency).map_err(|error| StoreError::Data(error.to_string()))?,
        issue_date: parse_date(&issue_date)?,
        due_date: due_date.as_deref().map(parse_date).transpose()?,
        delivery_date: delivery_date.as_deref().map(parse_date).transpose()?,
        buyer_reference,
        order_reference,
        note,
        preceding_invoice: None,
        lines: Vec::new(),
    };

    draft.preceding_invoice = match (preceding_id, preceding_number, preceding_issue_date) {
        (Some(id), Some(number), Some(date)) => Some(crux::PrecedingInvoice::new(
            DocumentId::parse_str(&id).map_err(|error| StoreError::Data(error.to_string()))?,
            number,
            parse_date(&date)?,
        )),
        _ => None,
    };

    let mut statement = conn.prepare(
        "SELECT id, position, name, description, quantity_micros, unit_code,
                unit_price_micros, price_base_qty_micros, vat_category, vat_rate_bp,
                vat_exemption_reason, vat_exemption_reason_code
         FROM invoice_draft_line
         WHERE draft_id = ?1 AND deleted = 0
         ORDER BY position",
    )?;
    let mut rows = statement.query([&key])?;
    while let Some(row) = rows.next()? {
        let line_id: String = row.get(0)?;
        let category: String = row.get(8)?;
        let rate: u16 = row.get(9)?;
        draft.lines.push(DraftLine {
            id: LineId::parse_str(&line_id).map_err(|error| StoreError::Data(error.to_string()))?,
            position: row.get(1)?,
            name: row.get(2)?,
            description: row.get(3)?,
            quantity: Quantity::from_micros(row.get(4)?),
            unit: UnitCode::parse(&row.get::<_, String>(5)?).map_err(|error| StoreError::Data(error.to_string()))?,
            unit_price: Money::from_micros(row.get(6)?, draft.currency),
            price_base_quantity: Quantity::from_micros(row.get(7)?),
            treatment: TaxTreatment {
                category: VatCategory::parse(&category).map_err(|error| StoreError::Data(error.to_string()))?,
                rate: VatRate::from_basis_points(rate).map_err(|error| StoreError::Data(error.to_string()))?,
                exemption_reason: row.get(10)?,
                exemption_reason_code: row.get(11)?,
            },
        });
    }

    Ok(Some(draft))
}

/// Minimal view of an issued document, used to build references.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct IssuedReference {
    /// Document identity.
    pub id: DocumentId,
    /// Human number (BT-1).
    pub number: String,
    /// Issue date (BT-2).
    pub issue_date: LocalDate,
    /// Invoice or credit note.
    pub doc_type: DocType,
    /// Customer the document was issued to.
    pub customer_id: CustomerId,
    /// Buyer reference carried by the document (BT-10).
    pub buyer_reference: Option<String>,
    /// Order reference carried by the document (BT-13).
    pub order_reference: Option<String>,
    /// Document currency.
    pub currency: Currency,
}

/// Looks up an issued document of `seller_id` by its number.
pub fn find_issued_reference(
    conn: &Connection,
    seller_id: SellerId,
    number: &str,
) -> Result<Option<IssuedReference>, StoreError> {
    let raw = conn
        .query_row(
            "SELECT id, number, issue_date_local, doc_type, counterparty_id, currency,
                    buyer_reference, order_reference
             FROM document WHERE owner_id = ?1 AND number = ?2 AND direction = 'outbound'",
            params![seller_id.to_string(), number],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .optional()?;

    let Some((id, number, issue_date, doc_type, customer_id, currency, buyer_reference, order_reference)) = raw else {
        return Ok(None);
    };
    let customer_id = customer_id.ok_or_else(|| StoreError::Data(format!("invoice {number} has no customer")))?;

    Ok(Some(IssuedReference {
        id: DocumentId::parse_str(&id).map_err(|error| StoreError::Data(error.to_string()))?,
        number,
        issue_date: parse_date(&issue_date)?,
        doc_type: parse_doc_type(&doc_type)?,
        customer_id: CustomerId::parse_str(&customer_id).map_err(|error| StoreError::Data(error.to_string()))?,
        buyer_reference,
        order_reference,
        currency: Currency::parse(&currency).map_err(|error| StoreError::Data(error.to_string()))?,
    }))
}

/// Builds a credit-note draft that copies the issued invoice `number`.
///
/// The draft keeps the original customer, currency, and lines so the user only
/// edits what the correction changes. Returns `None` when the seller has no
/// issued document with that number.
pub fn credit_note_for(
    conn: &Connection,
    seller_id: SellerId,
    number: &str,
    issue_date: LocalDate,
) -> Result<Option<InvoiceDraft>, StoreError> {
    let Some(reference) = find_issued_reference(conn, seller_id, number)? else {
        return Ok(None);
    };
    let mut draft = InvoiceDraft::credit_note(
        seller_id,
        reference.customer_id,
        reference.currency,
        issue_date,
        crux::PrecedingInvoice::new(reference.id, reference.number.clone(), reference.issue_date),
    );
    draft.buyer_reference = reference.buyer_reference;
    draft.order_reference = reference.order_reference;
    draft.lines = issued_lines(conn, reference.id, reference.currency)?;
    Ok(Some(draft))
}

fn issued_lines(conn: &Connection, invoice_id: DocumentId, currency: Currency) -> Result<Vec<DraftLine>, StoreError> {
    let mut statement =
        conn.prepare("SELECT id, position, payload_json FROM document_line WHERE document_id = ?1 ORDER BY position")?;
    let mut rows = statement.query([invoice_id.to_string()])?;
    let mut lines = Vec::new();
    while let Some(row) = rows.next()? {
        let _stored_id: String = row.get(0)?;
        let payload: String = row.get(2)?;
        let value: serde_json::Value = serde_json::from_str(&payload)?;
        lines.push(DraftLine {
            id: LineId::new(),
            position: row.get(1)?,
            name: text(&value, "name")?,
            description: value
                .get("description")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            quantity: Quantity::from_micros(integer(&value, "quantity_micros")?),
            unit: UnitCode::parse(&text(&value, "unit")?).map_err(|error| StoreError::Data(error.to_string()))?,
            unit_price: Money::from_micros(integer(&value, "unit_price_micros")?, currency),
            price_base_quantity: Quantity::from_micros(integer(&value, "price_base_quantity_micros")?),
            treatment: TaxTreatment {
                category: VatCategory::parse(&text(&value, "vat_category")?)
                    .map_err(|error| StoreError::Data(error.to_string()))?,
                rate: VatRate::from_basis_points(
                    u16::try_from(integer(&value, "vat_rate_bp")?)
                        .map_err(|_| StoreError::Data("stored VAT rate is out of range".to_owned()))?,
                )
                .map_err(|error| StoreError::Data(error.to_string()))?,
                exemption_reason: value
                    .get("exemption_reason")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                exemption_reason_code: value
                    .get("exemption_reason_code")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
            },
        });
    }
    Ok(lines)
}

fn integer(value: &serde_json::Value, key: &str) -> Result<i64, StoreError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| StoreError::Data(format!("stored line is missing {key}")))
}

fn text(value: &serde_json::Value, key: &str) -> Result<String, StoreError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| StoreError::Data(format!("stored line is missing {key}")))
}

fn verify_preceding(
    conn: &Connection,
    seller_id: SellerId,
    preceding: &crux::PrecedingInvoice,
) -> Result<(), StoreError> {
    let stored = conn
        .query_row(
            "SELECT number, issue_date_local FROM document
             WHERE id = ?1 AND owner_id = ?2 AND direction = 'outbound'",
            params![preceding.id.to_string(), seller_id.to_string()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;

    match stored {
        Some((number, date)) if number == preceding.number && date == preceding.issue_date.to_iso_string() => Ok(()),
        Some(_) => Err(StoreError::Data(
            "the referenced invoice no longer matches the credit note".to_owned(),
        )),
        None => Err(StoreError::Data(format!(
            "invoice {} is not an issued document of this seller",
            preceding.number
        ))),
    }
}

/// Issues a draft, invoking `render` inside the transaction.
pub fn issue_draft<R>(conn: &mut Connection, request: IssueRequest<'_>, render: R) -> Result<DocumentRecord, StoreError>
where
    R: FnOnce(&IssueView<'_>) -> Result<Vec<u8>, StoreError>,
{
    let transaction = conn.transaction()?;

    let draft = load_draft(&transaction, request.draft_id)?
        .ok_or_else(|| StoreError::NotFound(format!("draft {}", request.draft_id)))?;
    let seller = load_seller(&transaction, draft.seller_id)?;
    let customer = load_customer(&transaction, draft.customer_id)?;
    let totals = calculate_totals(&draft.lines).map_err(|error| StoreError::Data(error.to_string()))?;

    if let Some(preceding) = &draft.preceding_invoice {
        verify_preceding(&transaction, draft.seller_id, preceding)?;
    }

    let year = draft.issue_date.as_date().year();
    // Invoices and credit notes keep separate counters and a distinct number
    // prefix, so one never consumes the other's numbers.
    let credit = draft.doc_type.is_credit_note();
    let series = if credit { "C" } else { "A" };
    let sequence = next_number(&transaction, draft.seller_id, draft.doc_type.code(), year)?;
    let number = if credit {
        format!("{year}-C{sequence:04}")
    } else {
        format!("{year}-{sequence:04}")
    };
    let issued_id = DocumentId::new();

    let view = IssueView {
        id: issued_id,
        number: &number,
        draft: &draft,
        seller: &seller,
        customer: &customer,
        totals: &totals,
    };
    let ubl = render(&view)?;
    let ubl_hash = audit::sha256_hex(&ubl);

    let totals_json = totals_to_json(&totals)?;
    let breakdown_json = serde_json::to_string(&totals_json["breakdown"])?;
    let semantic = json!({
        "document": {
            "id": issued_id.to_string(),
            "number": number,
            "type": draft.doc_type.code(),
            "currency": draft.currency.as_str(),
            "issue_date": draft.issue_date.to_iso_string(),
            "due_date": draft.due_date.map(LocalDate::to_iso_string),
            "delivery_date": draft.delivery_date.map(LocalDate::to_iso_string),
        },
        "seller": dto::party_to_value(&seller.party)?,
        "customer": dto::party_to_value(&customer.party)?,
        "lines": draft.lines.iter().map(|line| json!({
            "id": line.id.to_string(),
            "position": line.position,
            "name": line.name,
            "description": line.description,
            "quantity": line.quantity.to_trimmed_string(),
            "unit": line.unit.as_str(),
            "unit_price": line.unit_price.to_trimmed_string(),
            "price_base_quantity": line.price_base_quantity.to_trimmed_string(),
            "vat_category": line.treatment.category.code(),
            "vat_rate_bp": line.treatment.rate.basis_points(),
        })).collect::<Vec<_>>(),
        "totals": totals_json,
    });
    let semantic_text = serde_json::to_string(&semantic)?;
    let semantic_hash = audit::sha256_hex(semantic_text.as_bytes());

    transaction.execute(
        "INSERT INTO document
             (id, direction, owner_id, counterparty_id, doc_type, series, number,
              sequence_value, currency, issue_date_local, due_date_local,
              delivery_date_local, buyer_reference, order_reference, note, totals_json,
              tax_breakdown_json, issuer_snapshot_json, recipient_snapshot_json,
              semantic_json, semantic_hash, state, recorded_at, recorded_by_device,
              created_at, preceding_invoice_id, preceding_number, preceding_issue_date)
         VALUES (?1, 'outbound', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 ?14, ?15, ?16, ?17, ?18, ?19, ?20, 'issued', ?21, ?22, ?21, ?23, ?24, ?25)",
        params![
            issued_id.to_string(),
            draft.seller_id.to_string(),
            draft.customer_id.to_string(),
            draft.doc_type.code(),
            series,
            number,
            sequence,
            draft.currency.as_str(),
            draft.issue_date.to_iso_string(),
            draft.due_date.map(LocalDate::to_iso_string),
            draft.delivery_date.map(LocalDate::to_iso_string),
            draft.buyer_reference,
            draft.order_reference,
            draft.note,
            serde_json::to_string(&totals_json)?,
            breakdown_json,
            serde_json::to_string(&dto::party_to_value(&seller.party)?)?,
            serde_json::to_string(&dto::party_to_value(&customer.party)?)?,
            semantic_text,
            semantic_hash,
            request.now,
            request.device,
            draft.preceding_invoice.as_ref().map(|value| value.id.to_string()),
            draft.preceding_invoice.as_ref().map(|value| value.number.clone()),
            draft
                .preceding_invoice
                .as_ref()
                .map(|value| value.issue_date.to_iso_string()),
        ],
    )?;

    for line in &draft.lines {
        let line_net = line.net().map_err(|error| StoreError::Data(error.to_string()))?;
        let tax =
            crux::tax_amount(line_net, line.treatment.rate).map_err(|error| StoreError::Data(error.to_string()))?;
        let payload = json!({
            "id": line.id.to_string(),
            "position": line.position,
            "name": line.name,
            "description": line.description,
            "quantity_micros": line.quantity.micros(),
            "unit": line.unit.as_str(),
            "unit_price_micros": line.unit_price.micros(),
            "price_base_quantity_micros": line.price_base_quantity.micros(),
            "vat_category": line.treatment.category.code(),
            "vat_rate_bp": line.treatment.rate.basis_points(),
            "exemption_reason": line.treatment.exemption_reason,
            "exemption_reason_code": line.treatment.exemption_reason_code,
        });
        transaction.execute(
            "INSERT INTO document_line
                 (id, document_id, position, payload_json, line_net_micros, tax_micros, currency)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                line.id.to_string(),
                issued_id.to_string(),
                line.position,
                serde_json::to_string(&payload)?,
                line_net.micros(),
                tax.micros(),
                draft.currency.as_str(),
            ],
        )?;
    }

    let blob_id = DocumentId::new().to_string();
    transaction.execute(
        "INSERT INTO blob (id, sha256, size_bytes, compression, payload, created_at)
         VALUES (?1, ?2, ?3, NULL, ?4, ?5)",
        params![blob_id, ubl_hash, ubl.len() as i64, ubl.as_slice(), request.now],
    )?;
    transaction.execute(
        "INSERT INTO artefact
             (id, document_id, kind, syntax_profile, bytes_hash,
              size_bytes, blob_id, rendered_at, renderer_version)
         VALUES (?1, ?2, 'ubl', 'peppol-bis-3.0', ?3, ?4, ?5, ?6, ?7)",
        params![
            DocumentId::new().to_string(),
            issued_id.to_string(),
            ubl_hash,
            ubl.len() as i64,
            blob_id,
            request.now,
            request.renderer_version,
        ],
    )?;

    audit::append(
        &transaction,
        request.now,
        request.actor,
        "issued",
        "invoice",
        &issued_id.to_string(),
        json!({
            "number": number,
            "sequence": sequence,
            "semantic_hash": semantic_hash,
            "ubl_sha256": ubl_hash,
        }),
    )?;
    let head = audit::append(
        &transaction,
        request.now,
        request.actor,
        "artefact_rendered",
        "invoice",
        &issued_id.to_string(),
        json!({ "kind": "ubl", "sha256": ubl_hash, "size": ubl.len() }),
    )?;

    transaction.commit()?;

    Ok(DocumentRecord {
        id: issued_id,
        number,
        issue_date: draft.issue_date,
        totals,
        ubl_sha256: ubl_hash,
        audit_head: head,
    })
}

/// Attaches an additional artefact (for example a PDF) to an issued invoice.
///
/// Artefacts are additive and immutable; attaching one never changes the
/// document itself. Returns the SHA-256 of the bytes.
#[allow(clippy::too_many_arguments)]
pub fn attach_artefact(
    conn: &Connection,
    document_id: DocumentId,
    kind: &str,
    bytes: &[u8],
    renderer_version: &str,
    now: &str,
) -> Result<String, StoreError> {
    let hash = audit::sha256_hex(bytes);
    let blob_id = DocumentId::new().to_string();
    let transaction = conn.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO blob (id, sha256, size_bytes, compression, payload, created_at)
         VALUES (?1, ?2, ?3, NULL, ?4, ?5)",
        params![blob_id, hash, bytes.len() as i64, bytes, now],
    )?;
    transaction.execute(
        "INSERT INTO artefact
             (id, document_id, kind, syntax_profile, bytes_hash,
              size_bytes, blob_id, rendered_at, renderer_version)
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8)",
        params![
            DocumentId::new().to_string(),
            document_id.to_string(),
            kind,
            hash,
            bytes.len() as i64,
            blob_id,
            now,
            renderer_version,
        ],
    )?;
    transaction.commit()?;
    Ok(hash)
}

/// Verifies every stored artefact against its hashes.
pub fn verify_artefacts(conn: &Connection) -> Result<usize, StoreError> {
    let mut statement = conn.prepare(
        "SELECT a.id, a.bytes_hash, b.sha256, b.payload
         FROM artefact a JOIN blob b ON b.id = a.blob_id",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0_usize;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let bytes_hash: String = row.get(1)?;
        let blob_hash: String = row.get(2)?;
        let payload: Vec<u8> = row.get(3)?;
        let actual = audit::sha256_hex(&payload);
        if actual != bytes_hash || actual != blob_hash {
            return Err(StoreError::Integrity(format!(
                "artefact {id} does not match its recorded hash"
            )));
        }
        count += 1;
    }
    Ok(count)
}

fn next_number(conn: &Connection, seller: SellerId, doc_type: &str, year: i32) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT INTO sequence (owner_id, doc_type, series, year, next_value)
         VALUES (?1, ?2, 'A', ?3, 1)
         ON CONFLICT DO NOTHING",
        params![seller.to_string(), doc_type, year],
    )?;
    let allocated: i64 = conn.query_row(
        "UPDATE sequence SET next_value = next_value + 1
         WHERE owner_id = ?1 AND doc_type = ?2 AND series = 'A' AND year = ?3
         RETURNING next_value - 1",
        params![seller.to_string(), doc_type, year],
        |row| row.get(0),
    )?;
    Ok(allocated)
}

fn totals_to_json(totals: &InvoiceTotals) -> Result<serde_json::Value, StoreError> {
    let money = |value: Money| -> Result<String, StoreError> {
        value
            .to_fixed_string(2)
            .map_err(|error| StoreError::Data(error.to_string()))
    };
    let mut breakdown = Vec::with_capacity(totals.breakdown.len());
    for row in &totals.breakdown {
        breakdown.push(json!({
            "category": row.category.code(),
            "rate_bp": row.rate_bp,
            "taxable_base": money(row.taxable_base)?,
            "tax": money(row.tax)?,
        }));
    }
    Ok(json!({
        "net": money(totals.net)?,
        "tax": money(totals.tax)?,
        "gross": money(totals.gross)?,
        "breakdown": breakdown,
    }))
}

fn parse_date(raw: &str) -> Result<LocalDate, StoreError> {
    LocalDate::parse_iso(raw).map_err(|error| StoreError::Data(error.to_string()))
}

fn parse_doc_type(raw: &str) -> Result<DocType, StoreError> {
    match raw {
        "380" => Ok(DocType::Invoice),
        "381" => Ok(DocType::CreditNote),
        other => Err(StoreError::Data(format!("unknown document type {other}"))),
    }
}

/// Current UTC time in RFC 3339 form.
pub fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}
