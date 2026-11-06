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

use crux::{AccountCode, AccountKind, FiscalPeriodId, LocalDate, SellerId};
use rusqlite::{Connection, OptionalExtension, params};

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
