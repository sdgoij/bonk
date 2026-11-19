//! A synthetic ledger for measuring the reports.
//!
//! This archive is a measuring instrument, not a set of books: it writes no audit
//! events and gives its entries a placeholder content hash. It exists so the
//! trial-balance benchmark runs against the table shape the report queries — the
//! joins, the index, and the line count — which is what the exit criterion
//! measures. It refuses a ledger that already has entries, because a benchmark
//! archive is seeded once from a fresh database.

use crux::{AccountCode, JournalEntryId, LocalDate, SellerId};
use rusqlite::{Connection, params};

use crate::error::StoreError;

/// The fixed timestamp on every synthetic entry, so a seed is reproducible.
const CREATED_AT: &str = "2026-01-01T00:00:00.000000Z";

/// One line of the synthetic sale: account, side, amount in micro-units.
const LINES: [(i64, &str, &str, i64); 3] = [
    (1, "1300", "debit", 121_000_000),
    (2, "8000", "credit", 100_000_000),
    (3, "1600", "credit", 21_000_000),
];

/// Writes `entries` synthetic three-line sales, dated across `year`.
///
/// Each entry debits debtors with 121.00, credits revenue with 100.00, and
/// credits VAT with 21.00, so every entry balances and the trial balance nets to
/// zero. The chart must already exist and the ledger must be empty. Returns the
/// number of journal lines written.
pub fn seed_ledger(conn: &Connection, seller_id: SellerId, year: i32, entries: usize) -> Result<usize, StoreError> {
    if entries == 0 {
        return Err(StoreError::Rejected(
            "a benchmark ledger needs at least one entry".to_owned(),
        ));
    }
    let existing: i64 = conn.query_row("SELECT count(*) FROM journal_entry", [], |row| row.get(0))?;
    if existing > 0 {
        return Err(StoreError::Rejected(
            "the benchmark ledger must start empty; create a fresh archive".to_owned(),
        ));
    }
    for (_, account, _, _) in LINES {
        let code = AccountCode::parse(account).map_err(|error| StoreError::Data(error.to_string()))?;
        if crate::ledger::load_account(conn, &code)?.is_none() {
            return Err(StoreError::Rejected(format!(
                "account {account} does not exist; run `bonk ledger setup` first"
            )));
        }
    }

    let start = LocalDate::from_ymd(year, 1, 1).map_err(|error| StoreError::Data(error.to_string()))?;
    let mut periods = Vec::with_capacity(12);
    for month in 1..=12_u8 {
        let date = LocalDate::from_ymd(year, month, 1).map_err(|error| StoreError::Data(error.to_string()))?;
        periods.push(crate::ledger::ensure_period(conn, seller_id, date)?.id.to_string());
    }

    let transaction = conn.unchecked_transaction()?;
    {
        let mut entry = transaction.prepare(
            "INSERT INTO journal_entry
                 (id, journal_code, period_id, number, entry_date_local, description,
                  source_kind, source_id, reverses_entry_id, posting_profile_id, rule_version,
                  currency, base_currency, created_at, created_by, content_hash)
             VALUES (?1, 'VK', ?2, ?3, ?4, 'Benchmark sale', 'manual', NULL, NULL,
                     'nl-sme', '1', 'EUR', NULL, ?5, 'bench', ?6)",
        )?;
        let mut line = transaction.prepare(
            "INSERT INTO journal_line
                 (id, entry_id, line_no, account_code, side, amount_micros, currency,
                  fx_rate_micros, base_amount_micros, party_id, tax_code_id, description,
                  vat_category, vat_rate_bp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'EUR', NULL, ?6, NULL, NULL, NULL, NULL, NULL)",
        )?;
        for index in 0..entries {
            let days = i64::try_from(index % 365).map_err(|_| StoreError::Data("too many entries".to_owned()))?;
            let date = start
                .plus_days(days)
                .map_err(|error| StoreError::Data(error.to_string()))?;
            let month = u8::from(date.as_date().month());
            let id = JournalEntryId::new().to_string();
            let number = format!("BENCH-{index:08}");
            let hash = crate::audit::sha256_hex(format!("bench-{index}").as_bytes());
            entry.execute(params![
                id,
                periods[usize::from(month) - 1],
                number,
                date.to_iso_string(),
                CREATED_AT,
                hash
            ])?;
            for (line_no, account, side, amount) in LINES {
                line.execute(params![format!("{id}-{line_no}"), id, line_no, account, side, amount])?;
            }
        }
    }
    transaction.commit()?;
    Ok(entries * LINES.len())
}

/// The journal lines in a window, for the benchmark's report line.
pub fn line_count(conn: &Connection, seller_id: SellerId, from: LocalDate, to: LocalDate) -> Result<usize, StoreError> {
    let count: i64 = conn.query_row(
        "SELECT count(*)
         FROM journal_line l
         JOIN journal_entry e ON e.id = l.entry_id
         JOIN fiscal_period p ON p.id = e.period_id
         WHERE p.seller_id = ?1 AND e.entry_date_local >= ?2 AND e.entry_date_local <= ?3",
        params![seller_id.to_string(), from.to_iso_string(), to.to_iso_string()],
        |row| row.get(0),
    )?;
    Ok(usize::try_from(count).unwrap_or(0))
}

/// The plan the trial balance uses, so a slow measurement can be explained.
pub fn trial_balance_plan(
    conn: &Connection,
    seller_id: SellerId,
    from: LocalDate,
    to: LocalDate,
) -> Result<Vec<String>, StoreError> {
    let sql = format!("EXPLAIN QUERY PLAN {}", crate::report::BALANCES_SQL);
    let mut statement = conn.prepare(&sql)?;
    let mut rows = statement.query(params![seller_id.to_string(), from.to_iso_string(), to.to_iso_string()])?;
    let mut plan = Vec::new();
    while let Some(row) = rows.next()? {
        plan.push(row.get(3)?);
    }
    Ok(plan)
}
