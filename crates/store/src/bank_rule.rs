//! Rules that classify the bank lines no invoice will ever match.
//!
//! A bank statement carries two kinds of line. Most are payments against an
//! invoice, which the matcher in [`crate::bank`] proposes. The rest never belonged
//! to an invoice: bank fees, interest, VAT payments, payroll, a transfer between
//! the business's own accounts. They used to sit in the queue until a person booked
//! them by hand, every month, forever. A rule is the standing instruction that
//! replaces that: it names the shape of the line and the one account the money
//! belongs on, and the posting is the same two-line entry the mover would have
//! written.
//!
//! Two guards keep a rule from doing damage, and both are load-bearing:
//!
//! * a line the reference ladder already proposes an open item for is never
//!   classified, so a broad "money out" rule cannot swallow a customer payment;
//! * two rules that both fit are ambiguous. The line stays in the queue with both
//!   names reported, because a coin toss about which account money belongs on is
//!   worse than a question.
//!
//! Undoing a classification is a reversal, like every other correction in the
//! ledger: the entry is reversed and the line returns to the queue, and the history
//! says a rule fired and a person took it back.

use crux::{AccountCode, Currency, DocumentId, JournalEntryId, LocalDate, Money, SellerId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::audit;
use crate::error::StoreError;

/// A standing instruction about a shape of bank line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BankRule {
    /// Identifier.
    pub id: String,
    /// What the operator calls it.
    pub name: String,
    /// Where it sits in the list a person reads.
    pub position: i64,
    /// Whether it is tried at all.
    pub enabled: bool,
    /// `1` for money in, `-1` for money out, `None` for either.
    pub direction: Option<i64>,
    /// The counterparty's account, matched exactly.
    pub counterparty_iban: Option<String>,
    /// A case-insensitive substring of the remittance text.
    pub remittance_contains: Option<String>,
    /// A case-insensitive substring of the counterparty's name.
    pub counterparty_contains: Option<String>,
    /// The account the bank line is booked against.
    pub account_code: AccountCode,
}

/// What a new rule says, before it has an identifier.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuleDefinition {
    /// What the operator calls it.
    pub name: String,
    /// `1` for money in, `-1` for money out, `None` for either.
    pub direction: Option<i64>,
    /// The counterparty's account, matched exactly.
    pub counterparty_iban: Option<String>,
    /// A case-insensitive substring of the remittance text.
    pub remittance_contains: Option<String>,
    /// A case-insensitive substring of the counterparty's name.
    pub counterparty_contains: Option<String>,
    /// The account the bank line is booked against.
    pub account_code: String,
}

/// What classifying one line did, or why it did nothing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Classification {
    /// A rule fitted and the entry is in the books (or would be, on a dry run).
    Posted(Classified),
    /// More than one rule fitted, so nothing was posted.
    Ambiguous {
        /// The line.
        transaction_id: String,
        /// The rule names that fitted, in list order.
        rules: Vec<String>,
    },
    /// The line is proposed for an invoice, which a rule may not take.
    Reserved {
        /// The line.
        transaction_id: String,
        /// The document it is proposed for.
        number: String,
    },
    /// No rule fitted, and the line stays in the queue.
    NoRule {
        /// The line.
        transaction_id: String,
    },
}

/// A line a rule recognised, and the entry it produced.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Classified {
    /// The line.
    pub transaction_id: String,
    /// What the operator calls the rule.
    pub rule: String,
    /// The account the money was booked on.
    pub account_code: AccountCode,
    /// What the line moved.
    pub amount: Money,
    /// The entry that was written, when this was not a dry run.
    pub entry_id: Option<JournalEntryId>,
}

/// Stores a new rule at the end of the list.
pub fn add(
    conn: &Connection,
    seller_id: SellerId,
    definition: &RuleDefinition,
    actor: &str,
    now: &str,
) -> Result<BankRule, StoreError> {
    let name = check_name(&definition.name)?;
    if definition.counterparty_iban.is_none()
        && definition.remittance_contains.is_none()
        && definition.counterparty_contains.is_none()
    {
        return Err(StoreError::Rejected(
            "a rule needs at least one matcher: an IBAN, a remittance text, or a counterparty name".to_owned(),
        ));
    }
    if let Some(direction) = definition.direction
        && direction != 1
        && direction != -1
    {
        return Err(StoreError::Rejected(format!(
            "direction {direction} is not 1 (in) or -1 (out)"
        )));
    }
    let code = AccountCode::parse(&definition.account_code).map_err(|error| StoreError::Rejected(error.to_string()))?;
    account_exists(conn, &code)?;
    let taken: Option<String> = conn
        .query_row(
            "SELECT id FROM bank_rule WHERE seller_id = ?1 AND name = ?2",
            params![seller_id.to_string(), name],
            |row| row.get(0),
        )
        .optional()?;
    if taken.is_some() {
        return Err(StoreError::Rejected(format!("a rule called {name} already exists")));
    }

    let transaction = conn.unchecked_transaction()?;
    let position: i64 = transaction.query_row(
        "SELECT COALESCE(max(position), 0) + 1 FROM bank_rule WHERE seller_id = ?1",
        [seller_id.to_string()],
        |row| row.get(0),
    )?;
    let id = DocumentId::new().to_string();
    transaction.execute(
        "INSERT INTO bank_rule
                 (id, seller_id, name, position, enabled, direction, counterparty_iban,
                  remittance_contains, counterparty_contains, account_code, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
        params![
            id,
            seller_id.to_string(),
            name,
            position,
            definition.direction,
            definition.counterparty_iban.as_deref().map(normalise_iban),
            definition.remittance_contains,
            definition.counterparty_contains,
            code.as_str(),
            now,
        ],
    )?;
    audit::append(
        &transaction,
        now,
        actor,
        "bank_rule_added",
        "bank_rule",
        &id,
        serde_json::json!({
            "name": name,
            "account": code.as_str(),
            "direction": definition.direction,
        }),
    )?;
    transaction.commit()?;

    Ok(BankRule {
        id,
        name: name.to_owned(),
        position,
        enabled: true,
        direction: definition.direction,
        counterparty_iban: definition.counterparty_iban.as_deref().map(normalise_iban),
        remittance_contains: definition.remittance_contains.clone(),
        counterparty_contains: definition.counterparty_contains.clone(),
        account_code: code,
    })
}

/// Every rule of these books, in the order a person reads them.
pub fn list(conn: &Connection, seller_id: SellerId) -> Result<Vec<BankRule>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, name, position, enabled, direction, counterparty_iban,
                remittance_contains, counterparty_contains, account_code
         FROM bank_rule WHERE seller_id = ?1 ORDER BY position, name",
    )?;
    let mut rows = statement.query([seller_id.to_string()])?;
    let mut rules = Vec::new();
    while let Some(row) = rows.next()? {
        let code: String = row.get(8)?;
        rules.push(BankRule {
            id: row.get(0)?,
            name: row.get(1)?,
            position: row.get(2)?,
            enabled: row.get::<_, i64>(3)? != 0,
            direction: row.get(4)?,
            counterparty_iban: row.get(5)?,
            remittance_contains: row.get(6)?,
            counterparty_contains: row.get(7)?,
            account_code: AccountCode::parse(&code).map_err(|error| StoreError::Data(error.to_string()))?,
        });
    }
    Ok(rules)
}

/// Removes a rule by name.
///
/// Nothing is refused: a line the rule already classified keeps its entry, and
/// correcting that is a reversal whether or not the rule still exists.
pub fn remove(conn: &Connection, seller_id: SellerId, name: &str, actor: &str, now: &str) -> Result<(), StoreError> {
    let transaction = conn.unchecked_transaction()?;
    let id: Option<String> = transaction
        .query_row(
            "SELECT id FROM bank_rule WHERE seller_id = ?1 AND name = ?2",
            params![seller_id.to_string(), name],
            |row| row.get(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Err(StoreError::NotFound(format!("bank rule {name}")));
    };
    transaction.execute("DELETE FROM bank_rule WHERE id = ?1", [&id])?;
    audit::append(
        &transaction,
        now,
        actor,
        "bank_rule_removed",
        "bank_rule",
        &id,
        serde_json::json!({ "name": name }),
    )?;
    transaction.commit()?;
    Ok(())
}

/// Classifies every line still waiting in the queue.
///
/// Lines the reference ladder proposes for an invoice are left alone and reported,
/// so the caller can print the queue rather than a silent subtraction.
pub fn classify(
    conn: &Connection,
    seller_id: SellerId,
    dry_run: bool,
    actor: &str,
    now: &str,
) -> Result<Vec<Classification>, StoreError> {
    let waiting = crate::bank::list_transactions(conn, seller_id, Some("new"))?;
    let mut outcomes = Vec::with_capacity(waiting.len());
    for row in waiting {
        outcomes.push(classify_one(conn, seller_id, &row.id, dry_run, actor, now)?);
    }
    Ok(outcomes)
}

/// Classifies one line.
///
/// The guards live here rather than in [`classify`] so a line named by hand gets
/// the same treatment as one swept up by the queue: a person naming a transaction
/// still cannot make a rule take a payment that is proposed for an invoice.
pub fn classify_one(
    conn: &Connection,
    seller_id: SellerId,
    transaction_id: &str,
    dry_run: bool,
    actor: &str,
    now: &str,
) -> Result<Classification, StoreError> {
    let line = load(conn, seller_id, transaction_id)?;
    if line.status != "new" {
        return Err(StoreError::Rejected(format!(
            "transaction {transaction_id} is {}; only a line waiting in the queue is classified",
            line.status.replace('_', " ")
        )));
    }

    let proposed = crate::bank::proposals(conn, seller_id, transaction_id)?;
    if let Some(proposal) = proposed.first() {
        return Ok(Classification::Reserved {
            transaction_id: transaction_id.to_owned(),
            number: proposal.number.clone(),
        });
    }

    let fitting: Vec<BankRule> = list(conn, seller_id)?
        .into_iter()
        .filter(|rule| rule.enabled && rule.matches(&line))
        .collect();
    let rule = match fitting.as_slice() {
        [] => {
            return Ok(Classification::NoRule {
                transaction_id: transaction_id.to_owned(),
            });
        }
        [only] => only.clone(),
        many => {
            return Ok(Classification::Ambiguous {
                transaction_id: transaction_id.to_owned(),
                rules: many.iter().map(|rule| rule.name.clone()).collect(),
            });
        }
    };

    let reporting = crate::report::reporting_currency(conn, seller_id)?;
    let amount_document = Money::from_micros(line.amount_micros.abs(), line.currency);
    // The GL is in the reporting currency; a foreign bank account's line is
    // converted at the rate for the day the bank booked it. A missing rate is a
    // refusal rather than a number in the wrong currency.
    let amount = if line.currency == reporting {
        amount_document
    } else {
        let rate = crate::fx::rate_on(conn, seller_id, line.currency, line.booking_date)?.ok_or_else(|| {
            StoreError::Rejected(format!(
                "transaction {transaction_id} is in {}, and there is no {reporting} rate on or before {}; import a rate file (`bonk ledger fx import <file>`) or set one (`bonk ledger fx set --currency {} --date {} --rate <{reporting} per {}>`)",
                line.currency,
                line.booking_date,
                line.currency,
                line.booking_date,
                line.currency
            ))
        })?;
        rate.convert(amount_document, reporting).map_err(domain)?
    };
    if dry_run {
        return Ok(Classification::Posted(Classified {
            transaction_id: transaction_id.to_owned(),
            rule: rule.name,
            account_code: rule.account_code,
            amount,
            entry_id: None,
        }));
    }

    let entry_id = post(conn, &line, &rule, amount, actor, now)?;
    Ok(Classification::Posted(Classified {
        transaction_id: transaction_id.to_owned(),
        rule: rule.name,
        account_code: rule.account_code,
        amount,
        entry_id: Some(entry_id),
    }))
}

/// Reverses the entry a rule wrote and puts the line back in the queue.
pub fn unclassify(
    conn: &Connection,
    seller_id: SellerId,
    transaction_id: &str,
    reason: &str,
    actor: &str,
    now: &str,
) -> Result<PostedReversal, StoreError> {
    if reason.trim().is_empty() {
        return Err(StoreError::Rejected("a correction needs a reason".to_owned()));
    }
    let line = load(conn, seller_id, transaction_id)?;
    match line.status.as_str() {
        "posted" => {}
        "matched" | "partially_matched" => {
            return Err(StoreError::Rejected(format!(
                "transaction {transaction_id} was matched to an invoice; use `bank unmatch`"
            )));
        }
        other => {
            return Err(StoreError::Rejected(format!(
                "transaction {transaction_id} is {other}; only a classified line has an entry to reverse"
            )));
        }
    }

    let entry_id = entry_of(conn, transaction_id)?;
    let transaction = conn.unchecked_transaction()?;
    let reversal = crate::ledger::reverse_entry_in(&transaction, entry_id, reason, line.booking_date, actor, now)?;
    transaction.execute(
        "UPDATE bank_transaction SET status = 'new' WHERE id = ?1",
        [transaction_id],
    )?;
    audit::append(
        &transaction,
        now,
        actor,
        "bank_transaction_unclassified",
        "bank_transaction",
        transaction_id,
        serde_json::json!({
            "entry": entry_id.to_string(),
            "reversal": reversal.id.to_string(),
            "reason": reason,
        }),
    )?;
    transaction.commit()?;

    Ok(PostedReversal {
        transaction_id: transaction_id.to_owned(),
        entry_id,
        reversal_id: reversal.id,
    })
}

/// What unclassifying wrote.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PostedReversal {
    /// The line that went back into the queue.
    pub transaction_id: String,
    /// The entry that was undone.
    pub entry_id: JournalEntryId,
    /// The reversal that undid it.
    pub reversal_id: JournalEntryId,
}

impl BankRule {
    /// True when this rule describes the line.
    fn matches(&self, line: &LoadedLine) -> bool {
        if let Some(direction) = self.direction {
            let line_direction = if line.amount_micros > 0 { 1 } else { -1 };
            if direction != line_direction {
                return false;
            }
        }
        if let Some(iban) = &self.counterparty_iban {
            let Some(theirs) = &line.counterparty_iban else {
                return false;
            };
            if *iban != normalise_iban(theirs) {
                return false;
            }
        }
        if let Some(text) = &self.remittance_contains
            && !contains_ignoring_case(line.remittance_info.as_deref(), text)
        {
            return false;
        }
        if let Some(text) = &self.counterparty_contains
            && !contains_ignoring_case(line.counterparty_name.as_deref(), text)
        {
            return false;
        }
        true
    }
}

/// Posts the line: the bank against the rule's account, in one entry.
///
/// The source is the transaction, so the uniqueness that stops a payment being
/// posted twice also stops a classification landing beside a settlement.
fn post(
    conn: &Connection,
    line: &LoadedLine,
    rule: &BankRule,
    amount: Money,
    actor: &str,
    now: &str,
) -> Result<JournalEntryId, StoreError> {
    let bank = AccountCode::parse(&line.ledger_account_code).map_err(|error| StoreError::Data(error.to_string()))?;
    let money_in = line.amount_micros > 0;
    let lines = if money_in {
        vec![
            crux::LedgerLine::debit(bank, amount).map_err(domain)?,
            crux::LedgerLine::credit(rule.account_code.clone(), amount).map_err(domain)?,
        ]
    } else {
        vec![
            crux::LedgerLine::debit(rule.account_code.clone(), amount).map_err(domain)?,
            crux::LedgerLine::credit(bank, amount).map_err(domain)?,
        ]
    };

    let description = describe(rule, line);
    let entry = crux::LedgerEntry::new(crux::Journal::Bank, description, lines).map_err(domain)?;

    let transaction = conn.unchecked_transaction()?;
    let posted = crate::ledger::post_entry_in(
        &transaction,
        crate::ledger::PostingRequest {
            entry: &entry,
            seller_id: line.seller_id,
            entry_date: line.booking_date,
            source: crate::ledger::PostingSource::BankTransaction(line.id),
            actor,
            now,
            posting_profile_id: crate::posting::PROFILE_ID,
            rule_version: crate::posting::RULE_VERSION,
            reverses: None,
            open_item: None,
        },
    )?;
    transaction.execute(
        "UPDATE bank_transaction SET status = 'posted' WHERE id = ?1",
        [line.id.to_string()],
    )?;
    audit::append(
        &transaction,
        now,
        actor,
        "bank_transaction_classified",
        "bank_transaction",
        &line.id.to_string(),
        serde_json::json!({
            "rule": rule.name,
            "account": rule.account_code.as_str(),
            "amount_micros": -line.amount_micros,
            "entry": posted.id.to_string(),
        }),
    )?;
    transaction.commit()?;
    Ok(posted.id)
}

/// The entry's description: which rule fired, and about what.
fn describe(rule: &BankRule, line: &LoadedLine) -> String {
    let subject = line
        .counterparty_name
        .as_deref()
        .or(line.remittance_info.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let Some(subject) = subject else {
        return rule.name.clone();
    };
    let mut clipped: String = subject.chars().take(60).collect();
    if subject.chars().count() > 60 {
        clipped.push('…');
    }
    format!("{}: {clipped}", rule.name)
}

/// The line as a rule needs it.
struct LoadedLine {
    id: DocumentId,
    seller_id: SellerId,
    booking_date: LocalDate,
    amount_micros: i64,
    currency: Currency,
    counterparty_name: Option<String>,
    counterparty_iban: Option<String>,
    remittance_info: Option<String>,
    status: String,
    ledger_account_code: String,
}

fn load(conn: &Connection, seller_id: SellerId, id: &str) -> Result<LoadedLine, StoreError> {
    let raw = conn
        .query_row(
            "SELECT t.id, a.seller_id, t.booking_date_local, t.amount_micros, t.currency,
                    t.counterparty_name, t.counterparty_iban, t.remittance_info, t.status,
                    a.ledger_account_code
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
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .map_err(|_| StoreError::NotFound(format!("bank transaction {id}")))?;

    Ok(LoadedLine {
        id: DocumentId::parse_str(&raw.0).map_err(|error| StoreError::Data(error.to_string()))?,
        seller_id: SellerId::parse_str(&raw.1).map_err(|error| StoreError::Data(error.to_string()))?,
        booking_date: LocalDate::parse_iso(&raw.2).map_err(|error| StoreError::Data(error.to_string()))?,
        amount_micros: raw.3,
        currency: Currency::parse(&raw.4).map_err(|error| StoreError::Data(error.to_string()))?,
        counterparty_name: raw.5,
        counterparty_iban: raw.6,
        remittance_info: raw.7,
        status: raw.8,
        ledger_account_code: raw.9,
    })
}

/// The entry a rule wrote for this line.
fn entry_of(conn: &Connection, transaction_id: &str) -> Result<JournalEntryId, StoreError> {
    let id: String = conn
        .query_row(
            "SELECT id FROM journal_entry
             WHERE journal_code = ?2 AND source_kind = 'bank' AND source_id = ?1",
            params![transaction_id, crux::Journal::Bank.code()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| {
            StoreError::Rejected(format!(
                "transaction {transaction_id} says it was classified, but no entry names it"
            ))
        })?;
    JournalEntryId::parse_str(&id).map_err(|error| StoreError::Data(error.to_string()))
}

/// An IBAN compares without its written grouping or its case.
fn normalise_iban(iban: &str) -> String {
    iban.chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

/// Case-insensitive substring search.
fn contains_ignoring_case(haystack: Option<&str>, needle: &str) -> bool {
    let Some(haystack) = haystack else {
        return false;
    };
    let needle = needle.trim();
    if needle.is_empty() {
        return false;
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn account_exists(conn: &Connection, code: &AccountCode) -> Result<(), StoreError> {
    let exists: Option<i64> = conn
        .query_row("SELECT 1 FROM account WHERE code = ?1", [code.as_str()], |row| {
            row.get(0)
        })
        .optional()?;
    if exists.is_none() {
        return Err(StoreError::NotFound(format!("account {code}")));
    }
    Ok(())
}

/// A name that may be used as a key and read in a list.
fn check_name(name: &str) -> Result<&str, StoreError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(StoreError::Rejected("a rule needs a name".to_owned()));
    }
    if trimmed != name || name.chars().any(char::is_control) {
        return Err(StoreError::Rejected(format!(
            "{name:?} is not usable as a rule name: no leading or trailing space, and no control characters"
        )));
    }
    Ok(name)
}

fn domain(error: crux::DomainError) -> StoreError {
    StoreError::Data(error.to_string())
}
