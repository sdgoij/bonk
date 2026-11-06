//! Ledger storage: the chart of accounts, fiscal periods, and postings.
//!
//! The domain rules live in [`crux::ledger`]; this module is where they meet a
//! schema. Three things are load-bearing here:
//!
//! - Accounts are rows, so a user can rename and re-map without waiting for a
//!   release, and RGS codes ride along as optional data.
//! - Periods are created on demand for the month a fact belongs to, and a locked
//!   period refuses postings at the database level as well as here.
//! - Postings are append-only. There is no update and no delete in this module,
//!   because there is none in the schema either.

use crux::{
    AccountCode, AccountKind, DocumentId, FiscalPeriodId, Journal, JournalEntryId, LedgerEntry, LedgerLine, LocalDate,
    SellerId, TaxCode, VatCategory, VatRate,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::audit;
use crate::error::StoreError;

/// An account in the chart of accounts.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Account {
    /// User-visible and stable.
    pub code: AccountCode,
    /// The name the user reads.
    pub name: String,
    /// What the account is, which decides its normal side.
    pub kind: AccountKind,
    /// Optional Referentie Grootboek Schema mapping.
    pub rgs_code: Option<String>,
    /// Parent in the reporting hierarchy.
    pub parent: Option<AccountCode>,
    /// AR/AP control accounts, reconciled against the open items.
    pub is_control: bool,
    /// `None` means the reporting currency of the books.
    pub currency: Option<String>,
    /// Archived accounts keep their history and take no new postings.
    pub archived: bool,
}

/// Whether a fiscal period accepts postings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PeriodStatus {
    /// Postings are allowed.
    Open,
    /// Postings are refused; reopening appends an event rather than deleting this.
    Locked,
}

impl PeriodStatus {
    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "open" => Ok(Self::Open),
            "locked" => Ok(Self::Locked),
            other => Err(StoreError::Data(format!("unknown period status {other:?}"))),
        }
    }
}

/// One month of one seller's books.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FiscalPeriod {
    /// Identifier, referenced by every entry in the period.
    pub id: FiscalPeriodId,
    /// The books this period belongs to.
    pub seller_id: SellerId,
    /// Calendar year.
    pub year: i32,
    /// 1–12, with 13 reserved for a year-end adjustment period.
    pub period_no: u8,
    /// First day, inclusive.
    pub starts_on: LocalDate,
    /// Last day, inclusive.
    pub ends_on: LocalDate,
    /// Open or locked.
    pub status: PeriodStatus,
}

impl FiscalPeriod {
    /// True when the period accepts new postings.
    pub fn is_open(&self) -> bool {
        self.status == PeriodStatus::Open
    }
}

/// Inserts an account.
///
/// Rejected when the code is taken: silently replacing an account would move
/// every posting that references it to a chart the user did not choose.
pub fn insert_account(conn: &Connection, account: &Account, now: &str) -> Result<(), StoreError> {
    let changed = conn.execute(
        "INSERT INTO account (code, name, kind, rgs_code, parent_code, is_control, currency, archived, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (code) DO NOTHING",
        params![
            account.code.as_str(),
            account.name,
            account.kind.as_str(),
            account.rgs_code,
            account.parent.as_ref().map(AccountCode::as_str),
            i64::from(account.is_control),
            account.currency,
            i64::from(account.archived),
            now,
        ],
    )?;
    if changed == 0 {
        return Err(StoreError::Rejected(format!("account {} already exists", account.code)));
    }
    Ok(())
}

/// Loads one account.
pub fn load_account(conn: &Connection, code: &AccountCode) -> Result<Option<Account>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT code, name, kind, rgs_code, parent_code, is_control, currency, archived
         FROM account WHERE code = ?1",
    )?;
    let mut rows = statement.query(params![code.as_str()])?;
    match rows.next()? {
        Some(row) => Ok(Some(read_account(row)?)),
        None => Ok(None),
    }
}

/// Every account, in code order, archived ones included.
///
/// Archived accounts are returned because a report over a past period must still
/// name the account it was posted to; a caller that offers a choice of account
/// filters them out itself.
pub fn list_accounts(conn: &Connection) -> Result<Vec<Account>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT code, name, kind, rgs_code, parent_code, is_control, currency, archived
         FROM account ORDER BY code",
    )?;
    let mut rows = statement.query([])?;
    let mut accounts = Vec::new();
    while let Some(row) = rows.next()? {
        accounts.push(read_account(row)?);
    }
    Ok(accounts)
}

/// Renames an account, and lets the caller change its RGS mapping and archive flag.
///
/// The code and the kind never change: the code is what postings reference and
/// what an accountant reads, and the kind decides the normal side of every
/// balance ever reported.
pub fn update_account(conn: &Connection, account: &Account, now: &str) -> Result<(), StoreError> {
    let changed = conn.execute(
        "UPDATE account SET name = ?2, rgs_code = ?3, parent_code = ?4, is_control = ?5,
                            currency = ?6, archived = ?7, updated_at = ?8
         WHERE code = ?1",
        params![
            account.code.as_str(),
            account.name,
            account.rgs_code,
            account.parent.as_ref().map(AccountCode::as_str),
            i64::from(account.is_control),
            account.currency,
            i64::from(account.archived),
            now,
        ],
    )?;
    if changed == 0 {
        return Err(StoreError::NotFound(format!("account {}", account.code)));
    }
    Ok(())
}

/// Returns the period a date falls into, creating it when it does not exist yet.
///
/// Creating on demand is deliberate: a business starts posting in the middle of
/// a year, and asking it to declare twelve months up front only produces months
/// that exist for no reason.
pub fn ensure_period(conn: &Connection, seller_id: SellerId, date: LocalDate) -> Result<FiscalPeriod, StoreError> {
    let (year, month) = (date.as_date().year(), u8::from(date.as_date().month()));
    let (starts_on, ends_on) = month_bounds(year, month)?;

    conn.execute(
        "INSERT INTO fiscal_period (id, seller_id, year, period_no, starts_on, ends_on, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open')
         ON CONFLICT (seller_id, year, period_no) DO NOTHING",
        params![
            FiscalPeriodId::new().to_string(),
            seller_id.to_string(),
            year,
            month,
            starts_on.to_iso_string(),
            ends_on.to_iso_string(),
        ],
    )?;

    load_period(conn, seller_id, year, month)?
        .ok_or_else(|| StoreError::Data(format!("period {year}-{month:02} vanished after insert")))
}

/// Loads the period for a year and month, if it exists.
pub fn load_period(
    conn: &Connection,
    seller_id: SellerId,
    year: i32,
    period_no: u8,
) -> Result<Option<FiscalPeriod>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, seller_id, year, period_no, starts_on, ends_on, status
         FROM fiscal_period WHERE seller_id = ?1 AND year = ?2 AND period_no = ?3",
    )?;
    let mut rows = statement.query(params![seller_id.to_string(), year, period_no])?;
    match rows.next()? {
        Some(row) => Ok(Some(read_period(row)?)),
        None => Ok(None),
    }
}

/// Every period of one year, in order.
pub fn list_periods(conn: &Connection, seller_id: SellerId, year: i32) -> Result<Vec<FiscalPeriod>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, seller_id, year, period_no, starts_on, ends_on, status
         FROM fiscal_period WHERE seller_id = ?1 AND year = ?2 ORDER BY period_no",
    )?;
    let mut rows = statement.query(params![seller_id.to_string(), year])?;
    let mut periods = Vec::new();
    while let Some(row) = rows.next()? {
        periods.push(read_period(row)?);
    }
    Ok(periods)
}

/// Locks a period: from here on, postings into it are refused.
///
/// The lock is a fact (`period_event`), not an edit, so the books still show when
/// the period was closed and by whom.
pub fn lock_period(conn: &Connection, period_id: FiscalPeriodId, actor: &str, at: &str) -> Result<(), StoreError> {
    let transaction = conn.unchecked_transaction()?;
    let status: Option<String> = transaction
        .query_row(
            "SELECT status FROM fiscal_period WHERE id = ?1",
            params![period_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    match status.as_deref() {
        None => return Err(StoreError::NotFound(format!("period {period_id}"))),
        Some("locked") => return Err(StoreError::Rejected("the period is already locked".to_owned())),
        Some(_) => {}
    }

    transaction.execute(
        "UPDATE fiscal_period SET status = 'locked' WHERE id = ?1",
        params![period_id.to_string()],
    )?;
    transaction.execute(
        "INSERT INTO period_event (id, period_id, kind, reason, at, actor) VALUES (?1, ?2, 'locked', NULL, ?3, ?4)",
        params![FiscalPeriodId::new().to_string(), period_id.to_string(), at, actor],
    )?;
    transaction.commit()?;
    Ok(())
}

/// The guard account for lines nobody has classified yet.
///
/// Postings may use it, but a period close does not accept it: the point of the
/// account is to be emptied, not to be lived in.
pub const UNCLASSIFIED_ACCOUNT: &str = "9999";

/// The starter chart: what a Dutch small business needs to post a quarter.
///
/// Codes follow the conventional Dutch numbering, and the ones the posting rules
/// name (1300 debtors, 1400 creditors, 1500/1600 VAT, 8000 revenue, 7000 costs)
/// are fixed points of those rules.
///
/// `rgs_code` is left empty on purpose. The Referentie Grootboek Schema is a
/// published release with its own versioning, and a mapping written from memory
/// would be worse than none: it would look authoritative and be wrong. The
/// mapping ships as a checked artefact, not as a guess.
pub fn starter_chart() -> Result<Vec<Account>, StoreError> {
    // Read as a table: code, name, kind, control account. Ordering is the order
    // the interface shows, grouped the way a chart of accounts is presented.
    const CHART: [(&str, &str, AccountKind, bool); 18] = [
        ("1000", "Kas", AccountKind::Asset, false),
        ("1100", "Bank", AccountKind::Asset, false),
        ("1300", "Handelsdebiteuren", AccountKind::Asset, true),
        ("1500", "Te vorderen btw", AccountKind::Asset, false),
        ("0500", "Eigen vermogen", AccountKind::Equity, false),
        ("1400", "Handelscrediteuren", AccountKind::Liability, true),
        ("1600", "Af te dragen btw hoog tarief", AccountKind::Liability, false),
        ("1610", "Af te dragen btw laag tarief", AccountKind::Liability, false),
        ("1620", "Af te dragen btw overige", AccountKind::Liability, false),
        ("8000", "Omzet hoog tarief", AccountKind::Revenue, false),
        ("8010", "Omzet laag tarief", AccountKind::Revenue, false),
        ("8020", "Omzet verlegd", AccountKind::Revenue, false),
        ("8030", "Omzet intracommunautair", AccountKind::Revenue, false),
        ("8040", "Omzet buiten de EU", AccountKind::Revenue, false),
        ("8050", "Omzet vrijgesteld", AccountKind::Revenue, false),
        ("7000", "Algemene kosten", AccountKind::Expense, false),
        ("7100", "Bankkosten", AccountKind::Expense, false),
        (UNCLASSIFIED_ACCOUNT, "Nog te rubriceren", AccountKind::Expense, false),
    ];

    CHART
        .iter()
        .map(|(code, name, kind, is_control)| {
            Ok(Account {
                code: AccountCode::parse(code).map_err(|error| StoreError::Data(error.to_string()))?,
                name: (*name).to_owned(),
                kind: *kind,
                rgs_code: None,
                parent: None,
                is_control: *is_control,
                currency: None,
                archived: false,
            })
        })
        .collect()
}

/// Inserts the starter chart, leaving accounts that already exist alone.
///
/// Returns how many accounts were added, so a caller can tell "nothing to do"
/// from "set up". Re-running after a user has renamed an account must not undo
/// the rename, which is why this is `DO NOTHING` and not an upsert.
pub fn seed_starter_chart(conn: &Connection, now: &str) -> Result<usize, StoreError> {
    let accounts = starter_chart()?;
    let transaction = conn.unchecked_transaction()?;
    let mut added = 0_usize;
    for account in &accounts {
        added += transaction.execute(
            "INSERT INTO account (code, name, kind, rgs_code, parent_code, is_control, currency, archived, updated_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8)
             ON CONFLICT (code) DO NOTHING",
            params![
                account.code.as_str(),
                account.name,
                account.kind.as_str(),
                account.rgs_code,
                i64::from(account.is_control),
                account.currency,
                i64::from(account.archived),
                now,
            ],
        )?;
    }
    transaction.commit()?;
    Ok(added)
}

/// The first and last day of a month.
fn month_bounds(year: i32, month: u8) -> Result<(LocalDate, LocalDate), StoreError> {
    let start = LocalDate::from_ymd(year, month, 1).map_err(|error| StoreError::Data(error.to_string()))?;
    let (next_year, next_month) = if month == 12 {
        (
            year.checked_add(1)
                .ok_or_else(|| StoreError::Data("year overflow".to_owned()))?,
            1,
        )
    } else {
        (year, month + 1)
    };
    let first_of_next =
        LocalDate::from_ymd(next_year, next_month, 1).map_err(|error| StoreError::Data(error.to_string()))?;
    let end = first_of_next
        .plus_days(-1)
        .map_err(|error| StoreError::Data(error.to_string()))?;
    Ok((start, end))
}

fn read_account(row: &rusqlite::Row<'_>) -> Result<Account, StoreError> {
    let code: String = row.get(0)?;
    let kind: String = row.get(2)?;
    let parent: Option<String> = row.get(4)?;
    Ok(Account {
        code: AccountCode::parse(&code).map_err(|error| StoreError::Data(error.to_string()))?,
        name: row.get(1)?,
        kind: AccountKind::parse(&kind).map_err(|error| StoreError::Data(error.to_string()))?,
        rgs_code: row.get(3)?,
        parent: match parent {
            Some(code) => Some(AccountCode::parse(&code).map_err(|error| StoreError::Data(error.to_string()))?),
            None => None,
        },
        is_control: row.get::<_, i64>(5)? != 0,
        currency: row.get(6)?,
        archived: row.get::<_, i64>(7)? != 0,
    })
}

fn read_period(row: &rusqlite::Row<'_>) -> Result<FiscalPeriod, StoreError> {
    let id: String = row.get(0)?;
    let seller: String = row.get(1)?;
    let starts_on: String = row.get(4)?;
    let ends_on: String = row.get(5)?;
    let status: String = row.get(6)?;
    Ok(FiscalPeriod {
        id: FiscalPeriodId::parse_str(&id).map_err(|error| StoreError::Data(error.to_string()))?,
        seller_id: SellerId::parse_str(&seller).map_err(|error| StoreError::Data(error.to_string()))?,
        year: row.get(2)?,
        period_no: row.get(3)?,
        starts_on: LocalDate::parse_iso(&starts_on).map_err(|error| StoreError::Data(error.to_string()))?,
        ends_on: LocalDate::parse_iso(&ends_on).map_err(|error| StoreError::Data(error.to_string()))?,
        status: PeriodStatus::parse(&status)?,
    })
}

/// What produced a posting.
///
/// The source is what makes posting idempotent: one document produces one entry,
/// and a second attempt finds the first instead of doubling the books. Manual and
/// opening entries carry no identifier — several of those in a month are normal —
/// so they are exempt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PostingSource {
    /// Someone typed it in.
    Manual,
    /// An invoice or credit note we issued.
    OutboundDocument(DocumentId),
    /// An invoice we received and recorded.
    InboundDocument(DocumentId),
    /// A bank statement line, by identifier.
    BankTransaction(DocumentId),
    /// A bank entry with no source fact, such as an opening statement balance.
    Bank,
    /// An opening balance at the accounting start date.
    Opening,
}

impl PostingSource {
    /// The storage token for the kind of source.
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::OutboundDocument(_) => "outbound_document",
            Self::InboundDocument(_) => "inbound_document",
            Self::BankTransaction(_) => "bank",
            Self::Bank => "bank",
            Self::Opening => "opening",
        }
    }

    /// The identifier of the fact, when it has one.
    pub fn id(self) -> Option<String> {
        match self {
            Self::OutboundDocument(id) | Self::InboundDocument(id) | Self::BankTransaction(id) => Some(id.to_string()),
            Self::Manual | Self::Bank | Self::Opening => None,
        }
    }
}

/// Everything a posting needs besides the entry itself.
pub struct PostingRequest<'a> {
    /// The balanced entry to write.
    pub entry: &'a LedgerEntry,
    /// The books being posted into.
    pub seller_id: SellerId,
    /// Business date; decides the period.
    pub entry_date: LocalDate,
    /// What produced it.
    pub source: PostingSource,
    /// Who posted it.
    pub actor: &'a str,
    /// When it was written, RFC 3339.
    pub now: &'a str,
    /// Which set of posting rules produced it.
    pub posting_profile_id: &'a str,
    /// The version of that rule set.
    pub rule_version: &'a str,
    /// The entry this one undoes, when it is a reversal.
    pub reverses: Option<JournalEntryId>,
    /// The open item this posting creates, when it creates one.
    pub open_item: Option<crate::open_items::NewOpenItem>,
}

/// A written posting.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PostedEntry {
    /// Identifier of the entry.
    pub id: JournalEntryId,
    /// The journal it was posted into.
    pub journal: Journal,
    /// Rendered per journal and year, e.g. `2026-0001`.
    pub number: String,
    /// The period it landed in.
    pub period_id: FiscalPeriodId,
    /// Hash of the entry's content, chained into the audit log.
    pub content_hash: String,
    /// The audit chain head after the posting.
    pub audit_head: String,
    /// False when the entry already existed and this call found it.
    pub created: bool,
}

/// Writes a posting, or returns the one that source already produced.
///
/// The whole operation is one transaction: a half-written entry would be a
/// half-true set of books. A locked period is refused here and again by the
/// schema, and an archived account is refused before anything is written.
pub fn post_entry(conn: &Connection, request: PostingRequest<'_>) -> Result<PostedEntry, StoreError> {
    let transaction = conn.unchecked_transaction()?;
    let posted = post_entry_in(&transaction, request)?;
    transaction.commit()?;
    Ok(posted)
}

/// The body of [`post_entry`], inside a transaction the caller owns.
///
/// A correction that touches more than one entry — a reversal and the entry that
/// replaces it — has to write both or neither, so the body is available to a
/// caller that already opened a transaction.
fn post_entry_in(conn: &Connection, request: PostingRequest<'_>) -> Result<PostedEntry, StoreError> {
    if let Some(existing) = find_posted(conn, request.source, request.entry.journal())? {
        return Ok(existing);
    }

    for line in request.entry.lines() {
        let account = load_account(conn, line.account())?
            .ok_or_else(|| StoreError::Rejected(format!("account {} does not exist", line.account())))?;
        if account.archived {
            return Err(StoreError::Rejected(format!(
                "account {} is archived and takes no postings",
                line.account()
            )));
        }
    }

    let period = ensure_period(conn, request.seller_id, request.entry_date)?;
    if !period.is_open() {
        return Err(StoreError::Rejected(format!(
            "fiscal period {}-{:02} is locked",
            period.year, period.period_no
        )));
    }

    let number = next_entry_number(
        conn,
        request.seller_id,
        request.entry.journal(),
        request.entry_date.as_date().year(),
    )?;
    let content_hash = content_hash(request.entry, &number, request.entry_date, request.source)?;
    let id = JournalEntryId::new();

    conn.execute(
        "INSERT INTO journal_entry
             (id, journal_code, period_id, number, entry_date_local, description,
              source_kind, source_id, reverses_entry_id, posting_profile_id, rule_version,
              currency, created_at, created_by, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            id.to_string(),
            request.entry.journal().code(),
            period.id.to_string(),
            number,
            request.entry_date.to_iso_string(),
            request.entry.description(),
            request.source.kind(),
            request.source.id(),
            request.reverses.map(|entry| entry.to_string()),
            request.posting_profile_id,
            request.rule_version,
            request.entry.currency().as_str(),
            request.now,
            request.actor,
            content_hash,
        ],
    )?;

    for (index, line) in request.entry.lines().iter().enumerate() {
        let line_no = i64::try_from(index + 1).map_err(|_| StoreError::Data("too many lines".to_owned()))?;
        let amount = line.amount().micros();
        conn.execute(
            "INSERT INTO journal_line
                 (id, entry_id, line_no, account_code, side, amount_micros, currency,
                  fx_rate_micros, base_amount_micros, party_id, tax_code_id, description,
                  vat_category, vat_rate_bp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?6, NULL, ?10, NULL, ?8, ?9)",
            params![
                JournalEntryId::new().to_string(),
                id.to_string(),
                line_no,
                line.account().as_str(),
                line.side().as_str(),
                amount,
                request.entry.currency().as_str(),
                line.vat().map(|tag| tag.category.code()),
                line.vat().map(|tag| tag.rate_bp),
                line.tax_code().map(crux::TaxCode::as_str),
            ],
        )?;
    }

    if let (Some(item), Some(source_id)) = (&request.open_item, request.source.id()) {
        let document = DocumentId::parse_str(&source_id).map_err(|error| StoreError::Data(error.to_string()))?;
        crate::open_items::insert(
            conn,
            request.seller_id,
            request.source.kind(),
            document,
            item,
            request.now,
        )?;
    }

    let audit_head = audit::append(
        conn,
        request.now,
        request.actor,
        "posted",
        "journal_entry",
        &id.to_string(),
        json!({
            "journal": request.entry.journal().code(),
            "number": number,
            "period": format!("{}-{:02}", period.year, period.period_no),
            "source_kind": request.source.kind(),
            "source_id": request.source.id(),
            "reverses": request.reverses.map(|entry| entry.to_string()),
            "open_item": request.open_item.as_ref().map(crate::open_items::audit_payload),
            "content_hash": content_hash,
        }),
    )?;

    Ok(PostedEntry {
        id,
        journal: request.entry.journal(),
        number,
        period_id: period.id,
        content_hash,
        audit_head,
        created: true,
    })
}

/// Posts an entry a person wrote, in the memorial journal.
///
/// The memorial journal is the accountant's escape hatch: a correction no rule can
/// derive. It is the one journal whose lines come from a human, so the description
/// is required, the entry must balance before anything is written, and every code
/// it names must exist and agree with the line's category. It carries no source
/// fact, so posting one twice writes two entries — which is what a human writing
/// an entry means, unlike posting a document twice.
pub fn post_manual_entry(
    conn: &Connection,
    seller_id: SellerId,
    entry_date: LocalDate,
    description: &str,
    lines: Vec<LedgerLine>,
    actor: &str,
    now: &str,
) -> Result<PostedEntry, StoreError> {
    let currency = crate::report::reporting_currency(conn, seller_id)?;
    let entry = LedgerEntry::new(Journal::Memorial, description, lines).map_err(|error| match error {
        crux::DomainError::EntryUnbalanced {
            debit_micros,
            credit_micros,
        } => StoreError::Rejected(format!(
            "the entry does not balance: {} debited, {} credited",
            money_text(crux::Money::from_micros(debit_micros, currency)),
            money_text(crux::Money::from_micros(credit_micros, currency)),
        )),
        other => StoreError::Rejected(other.to_string()),
    })?;
    check_manual_codes(conn, entry.lines())?;

    post_entry(
        conn,
        PostingRequest {
            entry: &entry,
            seller_id,
            entry_date,
            source: PostingSource::Manual,
            actor,
            now,
            posting_profile_id: "manual",
            rule_version: "1",
            reverses: None,
            open_item: None,
        },
    )
}

/// An amount for a sentence, without the currency code.
fn money_text(amount: crux::Money) -> String {
    amount
        .to_fixed_string(2)
        .unwrap_or_else(|_| amount.micros().to_string())
}

/// Checks the treatments a memorial entry names.
///
/// A line on the account whose box cannot be derived from the category says so by
/// needing a code: without one it would land in 1c whatever the treatment was,
/// which is a number in a box nobody chose.
fn check_manual_codes(conn: &Connection, lines: &[LedgerLine]) -> Result<(), StoreError> {
    for line in lines {
        let Some(code) = line.tax_code() else {
            if line.account().as_str() == crate::posting::VAT_PAYABLE_OTHER {
                return Err(StoreError::Rejected(
                    "a memorial line on 1620 needs a tax code: its box depends on the treatment".to_owned(),
                ));
            }
            continue;
        };
        let record = crate::tax_code::load(conn, code)?
            .ok_or_else(|| StoreError::Rejected(format!("tax code {code} is not in the catalogue")))?;
        match line.vat() {
            Some(vat) if vat.category == record.category => {}
            Some(vat) => {
                return Err(StoreError::Rejected(format!(
                    "tax code {code} resolves category {} but the line carries {}",
                    record.category.code(),
                    vat.category.code()
                )));
            }
            None => {
                return Err(StoreError::Rejected(format!(
                    "tax code {code} needs a VAT category and rate on the line"
                )));
            }
        }
    }
    Ok(())
}

/// A posting as it is stored, with the context around it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct StoredEntry {
    /// Identifier.
    pub id: JournalEntryId,
    /// Rendered per journal and year.
    pub number: String,
    /// Journal it belongs to.
    pub journal: Journal,
    /// Business date.
    pub entry_date: LocalDate,
    /// The entry itself.
    pub entry: LedgerEntry,
    /// Hash recorded at posting time.
    pub content_hash: String,
    /// The entry this one reverses, when it is a reversal.
    pub reverses: Option<JournalEntryId>,
    /// Who posted it.
    pub created_by: String,
    /// Which set of posting rules produced it.
    pub posting_profile_id: String,
    /// Version of that rule set.
    pub rule_version: String,
}

/// Reads one entry back, lines included.
pub fn load_entry(conn: &Connection, id: JournalEntryId) -> Result<Option<StoredEntry>, StoreError> {
    let stored = {
        let mut statement = conn.prepare(
            "SELECT number, journal_code, entry_date_local, description, currency,
                    content_hash, reverses_entry_id, created_by, posting_profile_id, rule_version
             FROM journal_entry WHERE id = ?1",
        )?;
        let mut rows = statement.query([id.to_string()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        (
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
        )
    };

    let (
        number,
        journal_code,
        entry_date_raw,
        description,
        currency_raw,
        content_hash,
        reverses_raw,
        created_by,
        posting_profile_id,
        rule_version,
    ) = stored;

    let journal = Journal::parse(&journal_code).map_err(|error| StoreError::Data(error.to_string()))?;
    let entry_date = LocalDate::parse_iso(&entry_date_raw).map_err(|error| StoreError::Data(error.to_string()))?;
    let currency = crux::Currency::parse(&currency_raw).map_err(|error| StoreError::Data(error.to_string()))?;

    let mut lines_statement = conn.prepare(
        "SELECT account_code, side, amount_micros, vat_category, vat_rate_bp, tax_code_id
             FROM journal_line WHERE entry_id = ?1 ORDER BY line_no",
    )?;
    let mut line_rows = lines_statement.query([id.to_string()])?;
    let mut lines = Vec::new();
    while let Some(row) = line_rows.next()? {
        let account: String = row.get(0)?;
        let side: String = row.get(1)?;
        let micros: i64 = row.get(2)?;
        let category: Option<String> = row.get(3)?;
        let rate_bp: Option<i64> = row.get(4)?;
        let code: Option<String> = row.get(5)?;
        let mut line = LedgerLine::new(
            AccountCode::parse(&account).map_err(|error| StoreError::Data(error.to_string()))?,
            crux::Side::parse(&side).map_err(|error| StoreError::Data(error.to_string()))?,
            crux::Money::from_micros(micros, currency),
        )
        .map_err(|error| StoreError::Data(error.to_string()))?;
        if let (Some(category), Some(rate_bp)) = (category, rate_bp) {
            let category = VatCategory::parse(&category).map_err(|error| StoreError::Data(error.to_string()))?;
            let rate = VatRate::from_basis_points(
                u16::try_from(rate_bp).map_err(|_| StoreError::Data(format!("VAT rate {rate_bp} out of range")))?,
            )
            .map_err(|error| StoreError::Data(error.to_string()))?;
            line = line.tagged(category, rate);
        }
        if let Some(code) = code {
            line = line.coded(TaxCode::parse(&code).map_err(|error| StoreError::Data(error.to_string()))?);
        }
        lines.push(line);
    }

    let entry = LedgerEntry::new(journal, description, lines)
        .map_err(|error| StoreError::Data(format!("stored entry {id} cannot be read back: {error}")))?;

    Ok(Some(StoredEntry {
        id,
        number,
        journal,
        entry_date,
        entry,
        content_hash,
        reverses: reverses_raw
            .map(|value| JournalEntryId::parse_str(&value))
            .transpose()
            .map_err(|error| StoreError::Data(error.to_string()))?,
        created_by,
        posting_profile_id,
        rule_version,
    }))
}

/// Finds an entry by the journal and number a person reads.
///
/// Numbers are unique per journal and year, so the journal is part of the name:
/// `VK:2026-0001` is one entry, and `2026-0001` alone is ambiguous as soon as a
/// second journal has one.
pub fn find_entry(conn: &Connection, journal: Journal, number: &str) -> Result<Option<JournalEntryId>, StoreError> {
    let mut statement = conn.prepare("SELECT id FROM journal_entry WHERE journal_code = ?1 AND number = ?2")?;
    let mut rows = statement.query(params![journal.code(), number])?;
    match rows.next()? {
        Some(row) => {
            let id: String = row.get(0)?;
            Ok(Some(
                JournalEntryId::parse_str(&id).map_err(|error| StoreError::Data(error.to_string()))?,
            ))
        }
        None => Ok(None),
    }
}

/// The entry that reverses `entry_id`, in the same journal.
///
/// A correction is never an edit: the original stays where it is, the reversal
/// names it, and together they net to zero. The reversal is dated by the caller,
/// because the month the mistake was made in may already be locked and the
/// correction belongs in the open period.
///
/// The reason is required and travels in the description, which is what an
/// accountant reading the journal sees; the link to the original is recorded in
/// the entry and in the audit event.
pub fn reverse_entry(
    conn: &Connection,
    entry_id: JournalEntryId,
    reason: &str,
    entry_date: LocalDate,
    actor: &str,
    now: &str,
) -> Result<PostedEntry, StoreError> {
    let transaction = conn.unchecked_transaction()?;
    let posted = reverse_entry_in(&transaction, entry_id, reason, entry_date, actor, now)?;
    transaction.commit()?;
    Ok(posted)
}

/// The body of [`reverse_entry`], inside a transaction the caller owns.
fn reverse_entry_in(
    conn: &Connection,
    entry_id: JournalEntryId,
    reason: &str,
    entry_date: LocalDate,
    actor: &str,
    now: &str,
) -> Result<PostedEntry, StoreError> {
    if reason.trim().is_empty() {
        return Err(StoreError::Rejected("a reversal needs a reason".to_owned()));
    }
    let stored = load_entry(conn, entry_id)?.ok_or_else(|| StoreError::NotFound(format!("entry {entry_id}")))?;

    if let Some(existing) = reversal_of(conn, entry_id)? {
        return Err(StoreError::Rejected(format!(
            "entry {} is already reversed by {}",
            stored.number, existing
        )));
    }

    let description = format!("Tegenboeking {} ({})", stored.number, reason.trim());
    let entry = stored
        .entry
        .reversed(description)
        .map_err(|error| StoreError::Data(error.to_string()))?;

    post_entry_in(
        conn,
        PostingRequest {
            entry: &entry,
            seller_id: seller_of(conn, entry_id)?,
            entry_date,
            source: PostingSource::Manual,
            actor,
            now,
            posting_profile_id: &stored.posting_profile_id,
            rule_version: &stored.rule_version,
            reverses: Some(entry_id),
            open_item: None,
        },
    )
}

/// What a reclassification wrote.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reclassified {
    /// The number of the entry that was corrected, without its journal.
    pub original: String,
    /// The entry that undoes it.
    pub reversal: PostedEntry,
    /// The entry that replaces it, under the new code.
    pub corrected: PostedEntry,
}

/// Files a posted purchase under a different treatment.
///
/// A code cannot be changed after the fact — the entry is append-only and the
/// reviewable facts are what they are — so the correction is the pair the books
/// already understand: the reversal, then the same amounts under the code that
/// names the right box. Both are written in one transaction, because half a
/// correction would leave the supply either counted twice or missing.
///
/// Only lines whose category the code resolves take the new code; anything else on
/// the entry keeps what it had. A code that matches no line is refused rather than
/// written, since it would look like a correction and change no box.
pub fn reclassify(
    conn: &Connection,
    entry_id: JournalEntryId,
    code: &str,
    reason: &str,
    entry_date: LocalDate,
    actor: &str,
    now: &str,
) -> Result<Reclassified, StoreError> {
    if reason.trim().is_empty() {
        return Err(StoreError::Rejected("a reclassification needs a reason".to_owned()));
    }
    let code = crux::TaxCode::parse(code).map_err(|error| StoreError::Rejected(error.to_string()))?;
    let record = crate::tax_code::load(conn, &code)?.ok_or_else(|| StoreError::NotFound(format!("tax code {code}")))?;
    let stored = load_entry(conn, entry_id)?.ok_or_else(|| StoreError::NotFound(format!("entry {entry_id}")))?;
    if stored.journal != Journal::Purchase {
        return Err(StoreError::Rejected(format!(
            "{} is a {} entry; only a purchase has a box a tax code can change",
            stored.number,
            stored.journal.code()
        )));
    }
    if let Some(existing) = reversal_of(conn, entry_id)? {
        return Err(StoreError::Rejected(format!(
            "entry {} is already reversed by {existing}; the correction is its own entry",
            stored.number
        )));
    }

    let mut matched = false;
    let mut lines = Vec::with_capacity(stored.entry.lines().len());
    for line in stored.entry.lines() {
        let mut rebuilt = LedgerLine::new(line.account().clone(), line.side(), line.amount())
            .map_err(|error| StoreError::Data(error.to_string()))?;
        if let Some(vat) = line.vat() {
            let rate = VatRate::from_basis_points(vat.rate_bp).map_err(|error| StoreError::Data(error.to_string()))?;
            rebuilt = rebuilt.tagged(vat.category, rate);
            if vat.category == record.category {
                rebuilt = rebuilt.coded(code.clone());
                matched = true;
            } else if let Some(existing) = line.tax_code() {
                rebuilt = rebuilt.coded(existing.clone());
            }
        }
        lines.push(rebuilt);
    }
    if !matched {
        return Err(StoreError::Rejected(format!(
            "{} has no {} line, so {code} would change nothing",
            stored.number,
            record.category.code()
        )));
    }

    let description = format!("Herindeling {} ({})", stored.number, reason.trim());
    let corrected =
        LedgerEntry::new(stored.journal, description, lines).map_err(|error| StoreError::Data(error.to_string()))?;

    let transaction = conn.unchecked_transaction()?;
    let seller_id = seller_of(&transaction, entry_id)?;
    let reversal = reverse_entry_in(&transaction, entry_id, reason, entry_date, actor, now)?;
    let corrected = post_entry_in(
        &transaction,
        PostingRequest {
            entry: &corrected,
            seller_id,
            entry_date,
            source: PostingSource::Manual,
            actor,
            now,
            posting_profile_id: stored.posting_profile_id.as_str(),
            rule_version: stored.rule_version.as_str(),
            reverses: None,
            open_item: None,
        },
    )?;
    audit::append(
        &transaction,
        now,
        actor,
        "entry_reclassified",
        "journal_entry",
        &entry_id.to_string(),
        json!({
            "reversal": reversal.id.to_string(),
            "corrected": corrected.id.to_string(),
            "code": code.as_str(),
            "rubriek": record.rubriek.code(),
            "reason": reason.trim(),
        }),
    )?;
    transaction.commit()?;

    Ok(Reclassified {
        original: stored.number,
        reversal,
        corrected,
    })
}

/// The entry that already reverses `entry_id`, if one does.
pub fn reversal_of(conn: &Connection, entry_id: JournalEntryId) -> Result<Option<String>, StoreError> {
    let mut statement = conn.prepare("SELECT number FROM journal_entry WHERE reverses_entry_id = ?1")?;
    let mut rows = statement.query([entry_id.to_string()])?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

/// The seller whose books an entry sits in, via its period.
fn seller_of(conn: &Connection, entry_id: JournalEntryId) -> Result<SellerId, StoreError> {
    let raw: String = conn.query_row(
        "SELECT p.seller_id FROM journal_entry e JOIN fiscal_period p ON p.id = e.period_id WHERE e.id = ?1",
        [entry_id.to_string()],
        |row| row.get(0),
    )?;
    SellerId::parse_str(&raw).map_err(|error| StoreError::Data(error.to_string()))
}

/// One line of an opening balance: where the business starts.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OpeningBalance {
    /// The balance-sheet account.
    pub account: AccountCode,
    /// Which side the balance sits on.
    pub side: crux::Side,
    /// Positive amount.
    pub amount: crux::Money,
}

/// Posts the opening balances of a set of books.
///
/// Only balance-sheet accounts may carry one: a revenue or expense balance at a
/// start date means the P&L of a year nobody posted, which is a migration mistake
/// rather than a starting position. The set must balance, and it is posted once —
/// later years inherit the closing position instead of restating it.
pub fn post_opening_balances(
    conn: &Connection,
    seller_id: SellerId,
    date: LocalDate,
    balances: &[OpeningBalance],
    actor: &str,
    now: &str,
) -> Result<PostedEntry, StoreError> {
    let existing: i64 = conn.query_row(
        "SELECT count(*) FROM journal_entry WHERE source_kind = 'opening'",
        [],
        |row| row.get(0),
    )?;
    if existing > 0 {
        return Err(StoreError::Rejected(
            "opening balances are already posted; later years continue from the closing position".to_owned(),
        ));
    }

    for balance in balances {
        let account = load_account(conn, &balance.account)?
            .ok_or_else(|| StoreError::Rejected(format!("account {} does not exist", balance.account)))?;
        if !account.kind.is_balance_sheet() {
            return Err(StoreError::Rejected(format!(
                "account {} is a {} account; an opening balance belongs on the balance sheet",
                balance.account,
                account.kind.as_str()
            )));
        }
    }

    let lines = balances
        .iter()
        .map(|balance| {
            LedgerLine::new(balance.account.clone(), balance.side, balance.amount)
                .map_err(|error| StoreError::Data(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let entry = LedgerEntry::new(
        Journal::Opening,
        format!("Beginbalans {}", date.as_date().year()),
        lines,
    )
    .map_err(|error| match error {
        crux::DomainError::EntryUnbalanced {
            debit_micros,
            credit_micros,
        } => StoreError::Rejected(format!(
            "the opening balances do not balance: {debit_micros} debited, {credit_micros} credited"
        )),
        other => StoreError::Data(other.to_string()),
    })?;

    post_entry(
        conn,
        PostingRequest {
            entry: &entry,
            seller_id,
            entry_date: date,
            source: PostingSource::Opening,
            actor,
            now,
            posting_profile_id: "nl-sme",
            rule_version: "1",
            reverses: None,
            open_item: None,
        },
    )
}

/// Recomputes the hash a posting of this entry would carry.
///
/// Exposed so a verifier can re-derive it from the stored facts and compare, the
/// same way invoice artefacts are re-derived. Everything that decides what the
/// entry means is in the input; everything else (ids, timestamps, actor, and the
/// link to a reversed entry, which the audit event carries) is context and stays
/// out, so the same fact posted twice hashes the same.
pub fn content_hash(
    entry: &LedgerEntry,
    number: &str,
    entry_date: LocalDate,
    source: PostingSource,
) -> Result<String, StoreError> {
    let lines: Vec<serde_json::Value> = entry
        .lines()
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let mut value = json!({
                "account": line.account().as_str(),
                "amount_micros": line.amount().micros(),
                "line_no": index + 1,
                "side": line.side().as_str(),
                "vat": line.vat().map(|tag| json!({
                    "category": tag.category.code(),
                    "rate_bp": tag.rate_bp,
                })),
            });
            // Added only when a code was chosen, so an entry posted before tax
            // codes existed hashes exactly as it did the day it was written.
            if let Some(code) = line.tax_code() {
                value["tax_code"] = json!(code.as_str());
            }
            value
        })
        .collect();

    let canonical = json!({
        "currency": entry.currency().as_str(),
        "date": entry_date.to_iso_string(),
        "description": entry.description(),
        "journal": entry.journal().code(),
        "lines": lines,
        "number": number,
        "source": { "id": source.id(), "kind": source.kind() },
    });
    // serde_json maps are BTreeMap-backed, so key order is stable and the hash is
    // a function of the values rather than of the order they were written in.
    Ok(audit::sha256_hex(serde_json::to_string(&canonical)?.as_bytes()))
}

/// The entry a source already produced, if any.
fn find_posted(conn: &Connection, source: PostingSource, journal: Journal) -> Result<Option<PostedEntry>, StoreError> {
    let Some(source_id) = source.id() else {
        return Ok(None);
    };
    let mut statement = conn.prepare(
        "SELECT id, number, period_id, content_hash, journal_code
         FROM journal_entry WHERE source_kind = ?1 AND source_id = ?2",
    )?;
    let mut rows = statement.query(params![source.kind(), source_id])?;
    match rows.next()? {
        Some(row) => {
            let stored_journal: String = row.get(4)?;
            if Journal::parse(&stored_journal) != Ok(journal) {
                return Err(StoreError::Data(format!(
                    "source {} {} is already posted to journal {stored_journal}",
                    source.kind(),
                    source_id
                )));
            }
            let id: String = row.get(0)?;
            let period: String = row.get(2)?;
            Ok(Some(PostedEntry {
                id: JournalEntryId::parse_str(&id).map_err(|error| StoreError::Data(error.to_string()))?,
                journal,
                number: row.get(1)?,
                period_id: FiscalPeriodId::parse_str(&period).map_err(|error| StoreError::Data(error.to_string()))?,
                content_hash: row.get(3)?,
                audit_head: String::new(),
                created: false,
            }))
        }
        None => Ok(None),
    }
}

/// Allocates the next number in a journal's year.
///
/// The counters live in the same `sequence` table as invoice numbers, under the
/// journal code and the `J` series, so the accounting layer inherits the lease
/// machinery that will be needed when more than one device can write.
fn next_entry_number(
    conn: &Connection,
    seller_id: SellerId,
    journal: Journal,
    year: i32,
) -> Result<String, StoreError> {
    conn.execute(
        "INSERT INTO sequence (owner_id, doc_type, series, year, next_value)
         VALUES (?1, ?2, 'J', ?3, 1)
         ON CONFLICT DO NOTHING",
        params![seller_id.to_string(), journal.code(), year],
    )?;
    let allocated: i64 = conn.query_row(
        "UPDATE sequence SET next_value = next_value + 1
         WHERE owner_id = ?1 AND doc_type = ?2 AND series = 'J' AND year = ?3
         RETURNING next_value - 1",
        params![seller_id.to_string(), journal.code(), year],
        |row| row.get(0),
    )?;
    Ok(format!("{year}-{allocated:04}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_bounds_cover_leap_february_and_year_end() {
        let (start, end) = month_bounds(2024, 2).unwrap();
        assert_eq!(start.to_iso_string(), "2024-02-01");
        assert_eq!(end.to_iso_string(), "2024-02-29");

        let (start, end) = month_bounds(2025, 2).unwrap();
        assert_eq!(start.to_iso_string(), "2025-02-01");
        assert_eq!(end.to_iso_string(), "2025-02-28");

        let (start, end) = month_bounds(2026, 12).unwrap();
        assert_eq!(start.to_iso_string(), "2026-12-01");
        assert_eq!(end.to_iso_string(), "2026-12-31");
    }

    #[test]
    fn the_starter_chart_is_coherent() {
        let chart = starter_chart().unwrap();
        let mut codes: Vec<&str> = chart.iter().map(|account| account.code.as_str()).collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total, "two accounts share a code");

        // The posting rules name these, so their kind is not negotiable.
        let find = |code: &str| {
            chart
                .iter()
                .find(|account| account.code.as_str() == code)
                .unwrap()
                .clone()
        };
        assert_eq!(find("1300").kind, AccountKind::Asset);
        assert!(find("1300").is_control);
        assert_eq!(find("1400").kind, AccountKind::Liability);
        assert!(find("1400").is_control);
        assert_eq!(find("8000").kind, AccountKind::Revenue);
        assert_eq!(find("7000").kind, AccountKind::Expense);
        assert_eq!(find(UNCLASSIFIED_ACCOUNT).kind, AccountKind::Expense);
        assert_eq!(
            chart.iter().filter(|account| account.is_control).count(),
            2,
            "exactly the AR and AP control accounts"
        );
    }
}
