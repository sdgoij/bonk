//! Bank statements: what the bank said, stored once, and what was matched to it.
//!
//! Two rules shape this module. A statement is stored with the hash of the bytes
//! it came from, so importing the same file twice is a no-op. A transaction is
//! stored with a fingerprint of what identifies it, because banks send
//! overlapping ranges: the same payment appears in two files and is still one
//! payment.
//!
//! The parser lives in `import`; nothing here knows what CAMT is. What arrives is
//! a [`ParsedStatement`], which keeps the store testable without an XML file and
//! lets a second format be added without touching this module.

use crux::{AccountCode, Currency, LocalDate, Money, SellerId};
use rusqlite::{Connection, params};

use crate::audit;
use crate::error::StoreError;

/// A bank account whose statements are imported.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BankAccount {
    /// Identifier.
    pub id: String,
    /// The books it belongs to.
    pub seller_id: SellerId,
    /// What the user calls it.
    pub name: String,
    /// IBAN, when the bank gives one.
    pub iban: Option<String>,
    /// The account's currency.
    pub currency: Currency,
    /// The ledger account the balance lives on, usually `1100`.
    pub ledger_account_code: AccountCode,
    /// Stored mapping for a CSV profile, unused by CAMT.
    pub import_profile: Option<String>,
}

/// One line of a statement, as parsed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParsedTransaction {
    /// The date the bank booked it.
    pub booking_date: LocalDate,
    /// The date it counts for, when the bank says.
    pub value_date: Option<LocalDate>,
    /// Signed amount in micro-units: money in is positive.
    pub amount_micros: i64,
    /// The transaction's currency.
    pub currency: Currency,
    /// Who paid or was paid.
    pub counterparty_name: Option<String>,
    /// Their account, when the bank includes it.
    pub counterparty_iban: Option<String>,
    /// Free-text remittance information, which is where invoice numbers live.
    pub remittance_info: Option<String>,
    /// The bank's or the sender's reference.
    pub external_ref: Option<String>,
}

/// A statement as parsed, ready to store.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParsedStatement {
    /// `camt053`, `mt940`, or `csv`.
    pub format: &'static str,
    /// The file it came from, for a human.
    pub source_name: Option<String>,
    /// The account's currency.
    pub currency: Currency,
    /// First day the statement covers.
    pub period_start: Option<LocalDate>,
    /// Last day the statement covers.
    pub period_end: Option<LocalDate>,
    /// Opening balance in micro-units, when the bank states one.
    pub opening_balance_micros: Option<i64>,
    /// Closing balance in micro-units, when the bank states one.
    pub closing_balance_micros: Option<i64>,
    /// The lines, in the order the bank sent them.
    pub transactions: Vec<ParsedTransaction>,
}

/// What an import did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImportReport {
    /// The stored statement, or the one this file already produced.
    pub statement_id: String,
    /// False when the same bytes had already been imported.
    pub created: bool,
    /// Lines written by this import.
    pub imported: usize,
    /// Lines the statement carried that were already known.
    pub duplicates: usize,
    /// Closing balance minus opening balance minus the sum of the lines.
    ///
    /// Zero for a statement that agrees with itself; a non-zero value is refused
    /// before anything is written.
    pub balance_difference_micros: i64,
}

/// Inserts a bank account.
pub fn insert_bank_account(conn: &Connection, account: &BankAccount, now: &str) -> Result<(), StoreError> {
    let changed = conn.execute(
        "INSERT INTO bank_account
             (id, seller_id, name, iban, currency, ledger_account_code, import_profile)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (id) DO NOTHING",
        params![
            account.id,
            account.seller_id.to_string(),
            account.name,
            account.iban,
            account.currency.as_str(),
            account.ledger_account_code.as_str(),
            account.import_profile,
        ],
    )?;
    if changed == 0 {
        return Err(StoreError::Rejected(format!(
            "bank account {} already exists",
            account.id
        )));
    }
    let _ = now;
    Ok(())
}

/// Every bank account of these books.
pub fn list_bank_accounts(conn: &Connection, seller_id: SellerId) -> Result<Vec<BankAccount>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, seller_id, name, iban, currency, ledger_account_code, import_profile
         FROM bank_account WHERE seller_id = ?1 ORDER BY name",
    )?;
    let mut rows = statement.query([seller_id.to_string()])?;
    let mut accounts = Vec::new();
    while let Some(row) = rows.next()? {
        let seller: String = row.get(1)?;
        let currency: String = row.get(4)?;
        let ledger: String = row.get(5)?;
        accounts.push(BankAccount {
            id: row.get(0)?,
            seller_id: SellerId::parse_str(&seller).map_err(|error| StoreError::Data(error.to_string()))?,
            name: row.get(2)?,
            iban: row.get(3)?,
            currency: Currency::parse(&currency).map_err(|error| StoreError::Data(error.to_string()))?,
            ledger_account_code: AccountCode::parse(&ledger).map_err(|error| StoreError::Data(error.to_string()))?,
            import_profile: row.get(6)?,
        });
    }
    Ok(accounts)
}

/// Stores a statement and the lines it carries that are not already known.
///
/// The whole import is one transaction: a statement whose lines were only half
/// written is worse than one that was refused. Balances are checked first, and a
/// statement that does not add up is refused with the difference named.
pub fn import_statement(
    conn: &Connection,
    bank_account_id: &str,
    statement: &ParsedStatement,
    payload_hash: &str,
    now: &str,
) -> Result<ImportReport, StoreError> {
    let difference = balance_difference(statement);
    if let Some(difference) = difference
        && difference != 0
    {
        return Err(StoreError::Rejected(format!(
            "the statement does not add up: opening plus movements differs from the closing balance by {difference} micro-units"
        )));
    }

    let account_exists: Option<i64> = conn
        .query_row("SELECT 1 FROM bank_account WHERE id = ?1", [bank_account_id], |row| {
            row.get(0)
        })
        .ok();
    if account_exists.is_none() {
        return Err(StoreError::NotFound(format!("bank account {bank_account_id}")));
    }

    let transaction = conn.unchecked_transaction()?;

    let existing: Option<String> = transaction
        .query_row(
            "SELECT id FROM bank_statement WHERE bank_account_id = ?1 AND payload_hash = ?2",
            params![bank_account_id, payload_hash],
            |row| row.get(0),
        )
        .ok();
    if let Some(id) = existing {
        let known: i64 = transaction.query_row(
            "SELECT count(*) FROM bank_transaction WHERE statement_id = ?1",
            [&id],
            |row| row.get(0),
        )?;
        transaction.commit()?;
        return Ok(ImportReport {
            statement_id: id,
            created: false,
            imported: 0,
            duplicates: usize::try_from(known).unwrap_or(usize::MAX),
            balance_difference_micros: difference.unwrap_or(0),
        });
    }

    let statement_id = crux::DocumentId::new().to_string();
    transaction.execute(
        "INSERT INTO bank_statement
             (id, bank_account_id, format, source_name, period_start, period_end,
              opening_balance_micros, closing_balance_micros, payload_hash, imported_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            statement_id,
            bank_account_id,
            statement.format,
            statement.source_name,
            statement.period_start.map(LocalDate::to_iso_string),
            statement.period_end.map(LocalDate::to_iso_string),
            statement.opening_balance_micros,
            statement.closing_balance_micros,
            payload_hash,
            now,
        ],
    )?;

    let mut imported = 0_usize;
    let mut duplicates = 0_usize;
    for line in &statement.transactions {
        let fingerprint = fingerprint(line);
        let written = transaction.execute(
            "INSERT INTO bank_transaction
                 (id, statement_id, booking_date_local, value_date_local, amount_micros, currency,
                  counterparty_name, counterparty_iban, remittance_info, external_ref,
                  status, ignored_reason, fingerprint)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'new', NULL, ?11)
             ON CONFLICT (fingerprint) DO NOTHING",
            params![
                crux::DocumentId::new().to_string(),
                statement_id,
                line.booking_date.to_iso_string(),
                line.value_date.map(LocalDate::to_iso_string),
                line.amount_micros,
                line.currency.as_str(),
                line.counterparty_name,
                line.counterparty_iban,
                line.remittance_info,
                line.external_ref,
                fingerprint,
            ],
        )?;
        if written == 1 {
            imported += 1;
        } else {
            duplicates += 1;
        }
    }

    audit::append(
        &transaction,
        now,
        "cli",
        "bank_statement_imported",
        "bank_statement",
        &statement_id,
        serde_json::json!({
            "format": statement.format,
            "source": statement.source_name,
            "transactions": statement.transactions.len(),
            "imported": imported,
            "duplicates": duplicates,
            "payload_hash": payload_hash,
        }),
    )?;

    transaction.commit()?;

    Ok(ImportReport {
        statement_id,
        created: true,
        imported,
        duplicates,
        balance_difference_micros: difference.unwrap_or(0),
    })
}

/// The fingerprint that makes one payment one row.
///
/// Date, amount, counterparty, and remittance: what a bank repeats verbatim when
/// the same payment appears in overlapping files, and what changes when it is a
/// different payment.
pub fn fingerprint(line: &ParsedTransaction) -> String {
    let canonical = format!(
        "{}|{}|{}|{}|{}|{}",
        line.booking_date.to_iso_string(),
        line.amount_micros,
        line.currency,
        line.counterparty_iban.as_deref().unwrap_or(""),
        line.counterparty_name.as_deref().unwrap_or(""),
        line.remittance_info.as_deref().unwrap_or(""),
    );
    audit::sha256_hex(canonical.as_bytes())
}

/// Closing minus opening minus the sum of the lines, when the bank stated both.
fn balance_difference(statement: &ParsedStatement) -> Option<i64> {
    let opening = statement.opening_balance_micros?;
    let closing = statement.closing_balance_micros?;
    let movements: i64 = statement.transactions.iter().map(|line| line.amount_micros).sum();
    Some(closing - opening - movements)
}

/// A stored transaction, as a list shows it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BankTransactionRow {
    /// Identifier.
    pub id: String,
    /// Where the money went or came from.
    pub account_name: String,
    /// The date the bank booked it.
    pub booking_date: LocalDate,
    /// Signed amount.
    pub amount: Money,
    /// Who paid or was paid.
    pub counterparty_name: Option<String>,
    /// Their account, when the bank gave it.
    pub counterparty_iban: Option<String>,
    /// The remittance information, which is where invoice numbers live.
    pub remittance_info: Option<String>,
    /// The bank's or the sender's reference.
    pub external_ref: Option<String>,
    /// What has happened to it so far.
    pub status: String,
}

/// Lists transactions, newest first, optionally only one status.
pub fn list_transactions(
    conn: &Connection,
    seller_id: SellerId,
    status: Option<&str>,
) -> Result<Vec<BankTransactionRow>, StoreError> {
    // An empty status means "any": the column never holds an empty string, so it
    // stands in for the absent filter without a second prepared statement.
    let wanted = status.unwrap_or_default();
    let mut statement = conn.prepare(
        "SELECT t.id, a.name, t.booking_date_local, t.amount_micros, t.currency,
                t.counterparty_name, t.counterparty_iban, t.remittance_info, t.external_ref, t.status
         FROM bank_transaction t
         JOIN bank_statement s ON s.id = t.statement_id
         JOIN bank_account a ON a.id = s.bank_account_id
         WHERE a.seller_id = ?1 AND (?2 = '' OR t.status = ?2)
         ORDER BY t.booking_date_local DESC, t.id",
    )?;
    let mut rows = statement.query(params![seller_id.to_string(), wanted])?;
    let mut transactions = Vec::new();
    while let Some(row) = rows.next()? {
        let booking_date: String = row.get(2)?;
        let currency: String = row.get(4)?;
        let currency = Currency::parse(&currency).map_err(|error| StoreError::Data(error.to_string()))?;
        transactions.push(BankTransactionRow {
            id: row.get(0)?,
            account_name: row.get(1)?,
            booking_date: LocalDate::parse_iso(&booking_date).map_err(|error| StoreError::Data(error.to_string()))?,
            amount: Money::from_micros(row.get(3)?, currency),
            counterparty_name: row.get(5)?,
            counterparty_iban: row.get(6)?,
            remittance_info: row.get(7)?,
            external_ref: row.get(8)?,
            status: row.get(9)?,
        });
    }
    Ok(transactions)
}
