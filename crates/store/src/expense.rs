//! The account a purchase's cost lands on.
//!
//! An invoice we receive says what it cost, not where it belongs in our chart.
//! EN 16931 has no field whose meaning is "office costs" for every business, so
//! somebody has to decide — and until they do, every purchase posts its base to
//! `9999 Nog te rubriceren`, which is a real account with a real balance and a
//! period close that will not accept it.
//!
//! The decision is stored with an author and a time, like the supplier match and
//! the tax code: the document is immutable, the posting snapshots the account on
//! the lines it wrote, and a report never asks the document again.
//!
//! One account per document, not per line. A purchase that mixes kinds of cost
//! still needs a memorial correction to split it, because splitting belongs with an
//! item catalogue and supplier defaults, which do not exist yet.

use crux::{AccountCode, AccountKind, DocumentId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::audit;
use crate::error::StoreError;

/// The account a purchase was put on, when a person chose one.
pub fn chosen(conn: &Connection, document_id: DocumentId) -> Result<Option<AccountCode>, StoreError> {
    let code: Option<String> = conn
        .query_row(
            "SELECT account_code FROM document_account WHERE document_id = ?1",
            [document_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    match code {
        Some(code) => Ok(Some(
            AccountCode::parse(&code).map_err(|error| StoreError::Data(error.to_string()))?,
        )),
        None => Ok(None),
    }
}

/// Chooses the account a received document's cost lands on.
///
/// Refused once the document is posted: the account is part of the entry that was
/// written, and entries are append-only. The moment to decide is between recording
/// the invoice and posting it, which is also when a person has it in front of them.
pub fn assign(
    conn: &Connection,
    document_id: DocumentId,
    code: &str,
    actor: &str,
    now: &str,
) -> Result<AccountCode, StoreError> {
    // The fundamental rule first: there is a moment to decide, and it has passed
    // once the entry exists. Where the cost may go is the second question.
    crate::posting::unposted_inbound(conn, document_id, "account")?;
    let account = cost_account(conn, code)?;

    let transaction = conn.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO document_account (document_id, account_code, decided_by, decided_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(document_id) DO UPDATE SET
             account_code = excluded.account_code,
             decided_by = excluded.decided_by,
             decided_at = excluded.decided_at",
        params![document_id.to_string(), account.as_str(), actor, now],
    )?;
    audit::append(
        &transaction,
        now,
        actor,
        "document_account_assigned",
        "document",
        &document_id.to_string(),
        serde_json::json!({ "account": account.as_str() }),
    )?;
    transaction.commit()?;
    Ok(account)
}

/// The account a purchase's cost may be put on.
///
/// One rule for both moments a cost account is decided: choosing it before the
/// posting, and moving it afterwards. It has to be an expense account that takes
/// postings, and it cannot be `9999`, which is the absence of a decision rather
/// than one.
pub(crate) fn cost_account(conn: &Connection, code: &str) -> Result<AccountCode, StoreError> {
    let account = AccountCode::parse(code).map_err(|error| StoreError::Rejected(error.to_string()))?;
    let stored = crate::ledger::load_account(conn, &account)?
        .ok_or_else(|| StoreError::NotFound(format!("account {account}")))?;
    if stored.archived {
        return Err(StoreError::Rejected(format!(
            "account {account} is archived and takes no postings"
        )));
    }
    if stored.is_control {
        return Err(StoreError::Rejected(format!(
            "account {account} holds a subledger; a purchase's cost goes to an expense account"
        )));
    }
    if stored.kind != AccountKind::Expense {
        return Err(StoreError::Rejected(format!(
            "account {account} is {}, not an expense; a purchase's cost landed on {} until somebody decides where it belongs",
            kind_label(stored.kind),
            crate::ledger::UNCLASSIFIED_ACCOUNT
        )));
    }
    if account.as_str() == crate::ledger::UNCLASSIFIED_ACCOUNT {
        return Err(StoreError::Rejected(format!(
            "account {account} is where a purchase lands when nobody has decided where its cost belongs, not somewhere to put one on purpose"
        )));
    }
    Ok(account)
}

/// Returns a document to the account purchases land on by default.
pub fn clear(conn: &Connection, document_id: DocumentId, actor: &str, now: &str) -> Result<(), StoreError> {
    crate::posting::unposted_inbound(conn, document_id, "account")?;
    let transaction = conn.unchecked_transaction()?;
    let removed = transaction.execute(
        "DELETE FROM document_account WHERE document_id = ?1",
        [document_id.to_string()],
    )?;
    if removed == 0 {
        return Err(StoreError::NotFound(format!(
            "document {document_id} has no account assigned"
        )));
    }
    audit::append(
        &transaction,
        now,
        actor,
        "document_account_cleared",
        "document",
        &document_id.to_string(),
        serde_json::json!({}),
    )?;
    transaction.commit()?;
    Ok(())
}

/// What a kind of account is called in a sentence.
fn kind_label(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Asset => "an asset account",
        AccountKind::Liability => "a liability account",
        AccountKind::Equity => "an equity account",
        AccountKind::Revenue => "a revenue account",
        AccountKind::Expense => "an expense account",
    }
}
