//! Open items: the detail behind the debtors and creditors control accounts.
//!
//! An open item is written when a document is posted and never updated. What is
//! still owed on it is derived from allocations, which is the whole point: a
//! stored "paid" flag is how a subledger ends up disagreeing with the journal it
//! is supposed to explain, and nobody notices until the quarter is filed.
//!
//! The amount is signed. A credit note is a negative open item until it is
//! allocated against the invoice it corrects, and that sign is what keeps the
//! control account equal to the sum of the subledger across a correction.

use crux::{Currency, CustomerId, DocumentId, LocalDate, Money, SellerId};
use rusqlite::{Connection, params};
use serde_json::json;

use crate::error::StoreError;

/// Which control account an open item belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpenItemKind {
    /// Owed to us.
    Receivable,
    /// Owed by us.
    Payable,
}

impl OpenItemKind {
    /// The stable storage token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Receivable => "receivable",
            Self::Payable => "payable",
        }
    }

    /// Parses the storage token.
    pub fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "receivable" => Ok(Self::Receivable),
            "payable" => Ok(Self::Payable),
            other => Err(StoreError::Data(format!("unknown open item kind {other:?}"))),
        }
    }

    /// The control account the items of this kind are reconciled against.
    pub const fn control_account(self) -> &'static str {
        match self {
            Self::Receivable => crate::posting::DEBTORS,
            Self::Payable => crate::posting::CREDITORS,
        }
    }
}

/// What to write when a document is posted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NewOpenItem {
    /// Receivable or payable.
    pub kind: OpenItemKind,
    /// The counterparty.
    pub party_id: CustomerId,
    /// Signed amount: negative for a credit note.
    pub amount: Money,
    /// When it falls due, if the document says.
    pub due_date: Option<LocalDate>,
    /// The document date the item belongs to.
    pub original_date: LocalDate,
}

/// An open item as stored, with what is still owed derived.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OpenItem {
    /// Identifier.
    pub id: String,
    /// Receivable or payable.
    pub kind: OpenItemKind,
    /// The counterparty.
    pub party_id: CustomerId,
    /// The document that produced it.
    pub source: DocumentId,
    /// The document's number, for a list a person reads.
    pub number: String,
    /// Signed original amount.
    pub amount: Money,
    /// When it falls due.
    pub due_date: Option<LocalDate>,
    /// The document date.
    pub original_date: LocalDate,
    /// Signed amount still open: the amount minus what is allocated.
    pub outstanding: Money,
}

impl OpenItem {
    /// True when the item is still open in either direction.
    pub fn is_open(&self) -> bool {
        !self.outstanding.is_zero()
    }

    /// Days past due at a date; negative when not yet due.
    pub fn days_past_due(&self, as_of: LocalDate) -> Option<i64> {
        let due = self.due_date?;
        Some(i64::from(
            as_of
                .as_date()
                .to_julian_day()
                .saturating_sub(due.as_date().to_julian_day()),
        ))
    }
}

/// One aging bucket.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AgingBucket {
    /// Lower bound in days past due, inclusive; `None` for "not yet due".
    pub from_days: Option<i64>,
    /// Upper bound in days past due, inclusive; `None` for "and beyond".
    pub to_days: Option<i64>,
    /// Signed total outstanding in the bucket.
    pub outstanding_micros: i64,
}

/// Aging of a subledger at a date.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Aging {
    /// Receivable or payable.
    pub kind: OpenItemKind,
    /// Reporting currency of the books.
    pub currency: Currency,
    /// The date the buckets were cut at.
    pub as_of: LocalDate,
    /// Not yet due.
    pub current: Money,
    /// 1–30 days past due.
    pub days_1_30: Money,
    /// 31–60 days past due.
    pub days_31_60: Money,
    /// 61–90 days past due.
    pub days_61_90: Money,
    /// More than 90 days past due.
    pub over_90: Money,
}

impl Aging {
    /// Signed total outstanding across every bucket.
    pub fn total(&self) -> Result<Money, StoreError> {
        let mut total = Money::zero(self.currency);
        for part in [
            self.current,
            self.days_1_30,
            self.days_31_60,
            self.days_61_90,
            self.over_90,
        ] {
            total = total
                .checked_add(part)
                .map_err(|error| StoreError::Data(error.to_string()))?;
        }
        Ok(total)
    }
}

/// Writes an open item inside the posting's transaction.
pub(crate) fn insert(
    conn: &Connection,
    seller_id: SellerId,
    source: &str,
    source_id: DocumentId,
    item: &NewOpenItem,
    now: &str,
) -> Result<(), StoreError> {
    if item.amount.is_zero() {
        return Err(StoreError::Rejected(format!("open item for {source_id} has no amount")));
    }
    conn.execute(
        "INSERT INTO open_item
             (id, seller_id, kind, party_id, source_kind, source_id, amount_micros,
              currency, due_date_local, original_date_local, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            DocumentId::new().to_string(),
            seller_id.to_string(),
            item.kind.as_str(),
            item.party_id.to_string(),
            source,
            source_id.to_string(),
            item.amount.micros(),
            item.amount.currency().as_str(),
            item.due_date.map(LocalDate::to_iso_string),
            item.original_date.to_iso_string(),
            now,
        ],
    )?;
    Ok(())
}

/// Every item of one kind, oldest first, settled ones included.
///
/// Settled items are returned because a quarter's subledger has to be explainable
/// after the fact; callers that want the work queue filter on [`OpenItem::is_open`].
pub fn list(conn: &Connection, seller_id: SellerId, kind: OpenItemKind) -> Result<Vec<OpenItem>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT o.id, o.kind, o.party_id, o.source_id, d.number, o.amount_micros, o.currency,
                o.due_date_local, o.original_date_local,
                o.amount_micros + COALESCE(
                    (SELECT sum(a.amount_micros) FROM allocation a WHERE a.open_item_id = o.id), 0
                ) AS outstanding
         FROM open_item o
         JOIN document d ON d.id = o.source_id
         WHERE o.seller_id = ?1 AND o.kind = ?2
         ORDER BY o.original_date_local, d.number",
    )?;
    let mut rows = statement.query(params![seller_id.to_string(), kind.as_str()])?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let kind_raw: String = row.get(1)?;
        let party: String = row.get(2)?;
        let source: String = row.get(3)?;
        let currency_raw: String = row.get(6)?;
        let due: Option<String> = row.get(7)?;
        let original: String = row.get(8)?;

        let currency = Currency::parse(&currency_raw).map_err(|error| StoreError::Data(error.to_string()))?;
        let amount = Money::from_micros(row.get(5)?, currency);
        items.push(OpenItem {
            id,
            kind: OpenItemKind::parse(&kind_raw)?,
            party_id: CustomerId::parse_str(&party).map_err(|error| StoreError::Data(error.to_string()))?,
            source: DocumentId::parse_str(&source).map_err(|error| StoreError::Data(error.to_string()))?,
            number: row.get(4)?,
            amount,
            due_date: due
                .map(|value| LocalDate::parse_iso(&value))
                .transpose()
                .map_err(|error| StoreError::Data(error.to_string()))?,
            original_date: LocalDate::parse_iso(&original).map_err(|error| StoreError::Data(error.to_string()))?,
            outstanding: Money::from_micros(row.get(9)?, currency),
        });
    }
    Ok(items)
}

/// Signed total still outstanding of one kind, allocations included.
pub fn total(conn: &Connection, seller_id: SellerId, kind: OpenItemKind) -> Result<Money, StoreError> {
    let currency = crate::report::reporting_currency(conn, seller_id)?;
    let total: Option<i64> = conn.query_row(
        "SELECT sum(o.amount_micros + COALESCE(
                    (SELECT sum(a.amount_micros) FROM allocation a WHERE a.open_item_id = o.id), 0))
         FROM open_item o WHERE o.seller_id = ?1 AND o.kind = ?2",
        params![seller_id.to_string(), kind.as_str()],
        |row| row.get(0),
    )?;
    Ok(Money::from_micros(total.unwrap_or(0), currency))
}

/// Ages the open items of one kind at a date.
pub fn aging(
    conn: &Connection,
    seller_id: SellerId,
    kind: OpenItemKind,
    as_of: LocalDate,
) -> Result<Aging, StoreError> {
    let currency = crate::report::reporting_currency(conn, seller_id)?;
    let mut aging = Aging {
        kind,
        currency,
        as_of,
        current: Money::zero(currency),
        days_1_30: Money::zero(currency),
        days_31_60: Money::zero(currency),
        days_61_90: Money::zero(currency),
        over_90: Money::zero(currency),
    };

    for item in list(conn, seller_id, kind)?.iter().filter(|item| item.is_open()) {
        let days = item.days_past_due(as_of);
        let slot = match days {
            None | Some(i64::MIN..=0) => &mut aging.current,
            Some(1..=30) => &mut aging.days_1_30,
            Some(31..=60) => &mut aging.days_31_60,
            Some(61..=90) => &mut aging.days_61_90,
            Some(_) => &mut aging.over_90,
        };
        *slot = slot
            .checked_add(item.outstanding)
            .map_err(|error| StoreError::Data(error.to_string()))?;
    }
    Ok(aging)
}

/// The audit payload for one open item, so a posting names what it created.
pub(crate) fn audit_payload(item: &NewOpenItem) -> serde_json::Value {
    json!({
        "kind": item.kind.as_str(),
        "party_id": item.party_id.to_string(),
        "amount_micros": item.amount.micros(),
        "due_date": item.due_date.map(LocalDate::to_iso_string),
    })
}
