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

/// How a transaction was matched to what it settles.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchMethod {
    /// The remittance information named the invoice and the amount agreed.
    ExactReference,
    /// The amount agreed and the invoice was not named.
    ExactAmount,
    /// One payment settled several invoices, each named in the remittance.
    Group,
    /// A human said so.
    Manual,
}

impl MatchMethod {
    /// The storage token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExactReference => "exact_ref",
            Self::ExactAmount => "exact_amount",
            Self::Group => "group",
            Self::Manual => "manual",
        }
    }
}

/// A candidate open item for a transaction.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Proposal {
    /// The open item.
    pub open_item_id: String,
    /// Its document number.
    pub number: String,
    /// What is still open on it.
    pub outstanding: Money,
    /// What this transaction would settle.
    pub applies: Money,
    /// Why it is a candidate.
    pub method: MatchMethod,
    /// A sentence for a person.
    pub reason: String,
}

/// What one match did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MatchReport {
    /// The transaction.
    pub transaction_id: String,
    /// The open item it settled.
    pub open_item_id: String,
    /// The document number, for a list.
    pub number: String,
    /// How much was settled.
    pub settled: Money,
    /// The journal entry posted for it.
    pub entry_id: crux::JournalEntryId,
    /// The allocation written.
    pub allocation_id: String,
}

/// Candidate open items for one transaction, best first.
///
/// The ladder is deliberately short in this version: a reference that names the
/// invoice is the strongest signal, an exact amount is the next, and everything
/// else is left for a person. Ranking by name similarity without a human in the
/// loop produces confident wrong answers, which is worse than a queue.
pub fn proposals(conn: &Connection, seller_id: SellerId, transaction_id: &str) -> Result<Vec<Proposal>, StoreError> {
    let transaction = load_transaction(conn, seller_id, transaction_id)?;
    if transaction.status != "new" && transaction.status != "partially_matched" {
        return Ok(Vec::new());
    }

    // Money in settles what we are owed; money out settles what we owe.
    let magnitude = transaction.amount_micros.abs();
    let mut proposals = Vec::new();
    for item in crate::open_items::list(conn, seller_id, transaction.kind())? {
        if !item.is_open() || item.outstanding.currency() != transaction.currency {
            continue;
        }
        let outstanding = item.outstanding.micros().abs();
        let named = names_the_item(transaction.remittance_info.as_deref(), &item.number);
        let exact = outstanding == magnitude;
        if !named && !exact {
            continue;
        }
        let applies = magnitude.min(outstanding);
        let method = if named && exact {
            MatchMethod::ExactReference
        } else {
            MatchMethod::ExactAmount
        };
        let reason = match (named, exact) {
            (true, true) => format!("{} is named in the payment and the amount agrees", item.number),
            (true, false) => format!("{} is named in the payment; this settles part of it", item.number),
            (false, true) => format!("the amount matches {} exactly", item.number),
            (false, false) => String::new(),
        };
        proposals.push(Proposal {
            open_item_id: item.id,
            number: item.number,
            outstanding: item.outstanding,
            applies: Money::from_micros(applies, transaction.currency),
            method,
            reason,
        });
    }

    // Strongest signal first; ties broken by document number so the list is stable.
    proposals.sort_by(|left, right| {
        rank(left.method)
            .cmp(&rank(right.method))
            .then_with(|| left.number.cmp(&right.number))
    });
    Ok(proposals)
}

/// A set of open items that one payment settles together.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GroupProposal {
    /// The items, in document-number order.
    pub open_item_ids: Vec<String>,
    /// Their document numbers, for a person.
    pub numbers: Vec<String>,
    /// What the payment would settle.
    pub applies: Money,
}

/// How many named items a group search looks at.
///
/// The combinations grow cubically with the number of candidates, and a list of
/// hundreds of proposals is worse for a person than the four that fit. The cap is
/// a guard on the search, not a rule about what a payment may settle: a person can
/// still match any set by hand.
const MAX_GROUP_CANDIDATES: usize = 12;

/// Groups of two or three open items that one payment settles together.
///
/// A customer paying three invoices with one transfer sends one bank line, so the
/// matching needs to see the three at once. Only items the remittance names are
/// considered, and only an exact sum: "these two are four cents short" is a
/// question a person answers better than a rule that rounds.
pub fn group_proposals(
    conn: &Connection,
    seller_id: SellerId,
    transaction_id: &str,
) -> Result<Vec<GroupProposal>, StoreError> {
    let transaction = load_transaction(conn, seller_id, transaction_id)?;
    if transaction.status != "new" && transaction.status != "partially_matched" {
        return Ok(Vec::new());
    }
    let magnitude = transaction.amount_micros.abs();
    if magnitude == 0 {
        return Ok(Vec::new());
    }

    let mut candidates: Vec<crate::open_items::OpenItem> =
        crate::open_items::list(conn, seller_id, transaction.kind())?
            .into_iter()
            .filter(|item| item.is_open() && item.outstanding.currency() == transaction.currency)
            .filter(|item| names_the_item(transaction.remittance_info.as_deref(), &item.number))
            .collect();
    candidates.sort_by(|left, right| left.number.cmp(&right.number));
    candidates.truncate(MAX_GROUP_CANDIDATES);

    let mut groups = Vec::new();
    for first in 0..candidates.len() {
        for second in (first + 1)..candidates.len() {
            let pair = outstanding(&candidates[first]) + outstanding(&candidates[second]);
            if pair == magnitude {
                groups.push(as_group(&candidates, &[first, second], transaction.currency));
                // A third item would take the sum past the payment.
                continue;
            }
            for third in (second + 1)..candidates.len() {
                if pair + outstanding(&candidates[third]) == magnitude {
                    groups.push(as_group(&candidates, &[first, second, third], transaction.currency));
                }
            }
        }
    }
    Ok(groups)
}

fn outstanding(item: &crate::open_items::OpenItem) -> i64 {
    item.outstanding.micros().abs()
}

fn as_group(items: &[crate::open_items::OpenItem], picks: &[usize], currency: Currency) -> GroupProposal {
    let total: i64 = picks.iter().map(|index| outstanding(&items[*index])).sum();
    GroupProposal {
        open_item_ids: picks.iter().map(|index| items[*index].id.clone()).collect(),
        numbers: picks.iter().map(|index| items[*index].number.clone()).collect(),
        applies: Money::from_micros(total, currency),
    }
}

/// Matches every transaction that has exactly one strong candidate.
///
/// Only a reference-and-amount match counts as strong: an amount that fits two
/// open items is a question for a person, not a coin toss.
pub fn auto_match(
    conn: &Connection,
    seller_id: SellerId,
    actor: &str,
    now: &str,
) -> Result<Vec<MatchReport>, StoreError> {
    let mut reports = Vec::new();
    for row in list_transactions(conn, seller_id, Some("new"))? {
        let candidates = proposals(conn, seller_id, &row.id)?;
        let strong: Vec<&Proposal> = candidates
            .iter()
            .filter(|proposal| proposal.method == MatchMethod::ExactReference)
            .collect();
        if strong.len() != 1 {
            continue;
        }
        let proposal = strong[0];
        reports.push(allocate(
            conn,
            seller_id,
            &row.id,
            &proposal.open_item_id,
            MatchMethod::ExactReference,
            actor,
            now,
        )?);
    }
    Ok(reports)
}

/// Settles a transaction against one or more open items, and posts the payment.
///
/// One transfer can pay several invoices, and then the books should say so once:
/// one bank line, one entry for the total, and an allocation per invoice naming
/// its share. An entry per invoice would pretend the bank moved the money several
/// times, and between those entries the bank account would disagree with the
/// statement it came from.
///
/// The entry, the allocations, the status change, and the audit event are one
/// transaction. A group that half-wrote would leave invoices settled that no
/// payment covers. The entry is keyed by the transaction, so matching twice
/// cannot pay twice.
pub fn allocate_group(
    conn: &Connection,
    seller_id: SellerId,
    transaction_id: &str,
    open_item_ids: &[String],
    method: MatchMethod,
    actor: &str,
    now: &str,
) -> Result<Vec<MatchReport>, StoreError> {
    if open_item_ids.is_empty() {
        return Err(StoreError::Rejected("a match needs an open item".to_owned()));
    }
    for (index, id) in open_item_ids.iter().enumerate() {
        if open_item_ids[..index].contains(id) {
            return Err(StoreError::Rejected(format!("{id} is named twice in the same match")));
        }
    }

    let transaction = load_transaction(conn, seller_id, transaction_id)?;
    let items = crate::open_items::list(conn, seller_id, transaction.kind())?;

    // Each item takes what is still open on it, capped by what is left of the
    // payment, so a group can never settle more than arrived.
    let mut remaining = transaction.amount_micros.abs();
    let mut shares: Vec<(&crate::open_items::OpenItem, i64)> = Vec::with_capacity(open_item_ids.len());
    for id in open_item_ids {
        let item = items
            .iter()
            .find(|item| &item.id == id)
            .ok_or_else(|| StoreError::NotFound(format!("open item {id}")))?;
        if !item.is_open() {
            return Err(StoreError::Rejected(format!("{} is already settled", item.number)));
        }
        if item.outstanding.currency() != transaction.currency {
            return Err(StoreError::Rejected(format!(
                "{} is in {} and the payment is in {}; a currency difference is a separate entry",
                item.number,
                item.outstanding.currency(),
                transaction.currency
            )));
        }
        let share = remaining.min(item.outstanding.micros().abs());
        if share == 0 {
            return Err(StoreError::Rejected(format!(
                "nothing left of the payment to settle {}",
                item.number
            )));
        }
        remaining -= share;
        shares.push((item, share));
    }

    let total: i64 = shares.iter().map(|(_, share)| *share).sum();
    let currency = transaction.currency;
    let names: Vec<&str> = shares.iter().map(|(item, _)| item.number.as_str()).collect();

    let database = conn.unchecked_transaction()?;
    let posted = post_settlement(
        &database,
        &transaction,
        &names,
        Money::from_micros(total, currency),
        actor,
        now,
    )?;

    let mut reports = Vec::with_capacity(shares.len());
    for (item, share) in &shares {
        let allocation_id = crux::DocumentId::new().to_string();
        database.execute(
            "INSERT INTO allocation
                 (id, transaction_id, open_item_id, entry_id, amount_micros, fx_difference_micros,
                  matched_at, matched_by, match_method, reversed)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, 0)",
            params![
                allocation_id,
                transaction_id,
                item.id,
                posted.id.to_string(),
                -share,
                now,
                actor,
                method.as_str(),
            ],
        )?;
        reports.push(MatchReport {
            transaction_id: transaction_id.to_owned(),
            open_item_id: item.id.clone(),
            number: item.number.clone(),
            settled: Money::from_micros(-share, currency),
            entry_id: posted.id,
            allocation_id,
        });
    }

    // Matched means the payment is used up: a transaction with money left over
    // still needs a person, because the remainder is either another invoice or an
    // account it belongs on.
    let still_open = crate::open_items::list(&database, seller_id, transaction.kind())?
        .into_iter()
        .filter(|candidate| open_item_ids.contains(&candidate.id))
        .any(|candidate| candidate.is_open());
    let allocated: i64 = database.query_row(
        "SELECT COALESCE(sum(amount_micros), 0) FROM allocation WHERE transaction_id = ?1",
        [transaction_id],
        |row| row.get(0),
    )?;
    let settled_in_full = allocated.abs() >= transaction.amount_micros.abs();
    database.execute(
        "UPDATE bank_transaction SET status = ?2 WHERE id = ?1",
        params![
            transaction_id,
            if settled_in_full && !still_open {
                "matched"
            } else {
                "partially_matched"
            }
        ],
    )?;

    audit::append(
        &database,
        now,
        actor,
        "bank_transaction_matched",
        "bank_transaction",
        transaction_id,
        serde_json::json!({
            "open_items": open_item_ids,
            "numbers": names,
            "settled_micros": -total,
            "method": method.as_str(),
            "entry": posted.id.to_string(),
        }),
    )?;
    database.commit()?;

    Ok(reports)
}

/// Settles a transaction against one open item.
///
/// The common case of [`allocate_group`], kept because most payments settle one
/// invoice and a caller should not have to build a list for it.
pub fn allocate(
    conn: &Connection,
    seller_id: SellerId,
    transaction_id: &str,
    open_item_id: &str,
    method: MatchMethod,
    actor: &str,
    now: &str,
) -> Result<MatchReport, StoreError> {
    let mut reports = allocate_group(
        conn,
        seller_id,
        transaction_id,
        &[open_item_id.to_owned()],
        method,
        actor,
        now,
    )?;
    reports
        .pop()
        .ok_or_else(|| StoreError::Rejected("the match wrote no allocation".to_owned()))
}

/// Undoes a match by appending its opposite.
///
/// Nothing is deleted: the history says a person matched it and then matched it
/// away, which is what an auditor needs and what a delete cannot say.
pub fn unmatch(
    conn: &Connection,
    seller_id: SellerId,
    allocation_id: &str,
    actor: &str,
    now: &str,
) -> Result<(), StoreError> {
    let (transaction_id, open_item_id, amount_micros): (String, String, i64) = conn
        .query_row(
            "SELECT transaction_id, open_item_id, amount_micros FROM allocation WHERE id = ?1",
            [allocation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| StoreError::NotFound(format!("allocation {allocation_id}")))?;

    let undone: i64 = conn.query_row(
        "SELECT count(*) FROM allocation
         WHERE transaction_id = ?1 AND open_item_id = ?2 AND amount_micros = ?3",
        params![transaction_id, open_item_id, -amount_micros],
        |row| row.get(0),
    )?;
    if undone > 0 {
        return Err(StoreError::Rejected(format!(
            "allocation {allocation_id} has already been undone"
        )));
    }

    conn.execute(
        "INSERT INTO allocation
             (id, transaction_id, open_item_id, entry_id, amount_micros, fx_difference_micros,
              matched_at, matched_by, match_method, reversed)
         VALUES (?1, ?2, ?3, NULL, ?4, 0, ?5, ?6, 'manual', 1)",
        params![
            crux::DocumentId::new().to_string(),
            transaction_id,
            open_item_id,
            -amount_micros,
            now,
            actor,
        ],
    )?;
    conn.execute(
        "UPDATE bank_transaction SET status = 'new' WHERE id = ?1",
        [&transaction_id],
    )?;
    audit::append(
        conn,
        now,
        actor,
        "bank_transaction_unmatched",
        "bank_transaction",
        &transaction_id,
        serde_json::json!({
            "allocation": allocation_id,
            "open_item": open_item_id,
            "amount_micros": -amount_micros,
        }),
    )?;
    let _ = seller_id;
    Ok(())
}

/// Posts the payment: money in moves bank up and debtors down, and the reverse
/// for money out.
///
/// It runs inside the caller's transaction, so an allocation that fails takes the
/// payment with it. The description names the invoices the payment settles,
/// because on the bank statement the entry is one line and a reader has to be
/// able to see why.
fn post_settlement(
    conn: &Connection,
    transaction: &LoadedTransaction,
    numbers: &[&str],
    amount: Money,
    actor: &str,
    now: &str,
) -> Result<crate::ledger::PostedEntry, StoreError> {
    let magnitude = Money::from_micros(amount.micros().abs(), amount.currency());
    let bank_account = account(&transaction.ledger_account_code)?;
    let lines = if transaction.kind() == crate::open_items::OpenItemKind::Receivable {
        vec![
            crux::LedgerLine::debit(bank_account, magnitude).map_err(domain)?,
            crux::LedgerLine::credit(account(crate::posting::DEBTORS)?, magnitude).map_err(domain)?,
        ]
    } else {
        vec![
            crux::LedgerLine::debit(account(crate::posting::CREDITORS)?, magnitude).map_err(domain)?,
            crux::LedgerLine::credit(bank_account, magnitude).map_err(domain)?,
        ]
    };

    let description = format!("Betaling {}", numbers.join(", "));
    let entry = crux::LedgerEntry::new(crux::Journal::Bank, description, lines).map_err(domain)?;

    crate::ledger::post_entry_in(
        conn,
        crate::ledger::PostingRequest {
            entry: &entry,
            seller_id: transaction.seller_id,
            entry_date: transaction.booking_date,
            source: crate::ledger::PostingSource::BankTransaction(transaction.id),
            actor,
            now,
            posting_profile_id: crate::posting::PROFILE_ID,
            rule_version: crate::posting::RULE_VERSION,
            reverses: None,
            open_item: None,
        },
    )
}

/// The transaction as the matcher needs it.
struct LoadedTransaction {
    id: crux::DocumentId,
    seller_id: SellerId,
    booking_date: LocalDate,
    amount_micros: i64,
    currency: Currency,
    remittance_info: Option<String>,
    status: String,
    ledger_account_code: String,
}

impl LoadedTransaction {
    fn kind(&self) -> crate::open_items::OpenItemKind {
        if self.amount_micros > 0 {
            crate::open_items::OpenItemKind::Receivable
        } else {
            crate::open_items::OpenItemKind::Payable
        }
    }
}

fn load_transaction(conn: &Connection, seller_id: SellerId, id: &str) -> Result<LoadedTransaction, StoreError> {
    let raw = conn
        .query_row(
            "SELECT t.id, a.seller_id, t.booking_date_local, t.amount_micros, t.currency,
                    t.remittance_info, t.status, a.ledger_account_code
             FROM bank_transaction t
             JOIN bank_statement s ON s.id = t.statement_id
             JOIN bank_account a ON a.id = s.bank_account_id
             WHERE t.id = ?1 AND a.seller_id = ?2",
            params![id, seller_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .map_err(|_| StoreError::NotFound(format!("bank transaction {id}")))?;

    Ok(LoadedTransaction {
        id: crux::DocumentId::parse_str(&raw.0).map_err(|error| StoreError::Data(error.to_string()))?,
        seller_id: SellerId::parse_str(&raw.1).map_err(|error| StoreError::Data(error.to_string()))?,
        booking_date: LocalDate::parse_iso(&raw.2).map_err(|error| StoreError::Data(error.to_string()))?,
        amount_micros: raw.3,
        currency: Currency::parse(&raw.4).map_err(|error| StoreError::Data(error.to_string()))?,
        remittance_info: raw.5,
        status: raw.6,
        ledger_account_code: raw.7,
    })
}

/// True when the remittance information names this document.
fn names_the_item(remittance: Option<&str>, number: &str) -> bool {
    let Some(remittance) = remittance else {
        return false;
    };
    remittance.to_ascii_lowercase().contains(&number.to_ascii_lowercase())
}

fn rank(method: MatchMethod) -> u8 {
    match method {
        MatchMethod::ExactReference => 0,
        MatchMethod::ExactAmount => 1,
        MatchMethod::Group => 2,
        MatchMethod::Manual => 3,
    }
}

fn account(code: &str) -> Result<AccountCode, StoreError> {
    AccountCode::parse(code).map_err(domain)
}

fn domain(error: crux::DomainError) -> StoreError {
    StoreError::Data(error.to_string())
}
