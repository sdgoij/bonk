//! Readiness checks before a VAT return is prepared.
//!
//! A return is only worth filing if three things hold: every posting lands in a
//! rubriek, the VAT in the ledger is the VAT on the documents, and the books
//! balance. Each check answers with what it found rather than a bare yes, because
//! "not ready" without a reason is how people learn to ignore the warning.
//!
//! One check is deliberately reported as *not checked* rather than passed: the
//! control accounts cannot be compared against a subledger until open items
//! exist. Saying so is the difference between an honest gate and a green tick that
//! means nothing.

use crux::{Currency, LocalDate, Money, SellerId};
use rusqlite::Connection;
use serde_json::Value;

use crate::error::StoreError;
use crate::posting::{VAT_PAYABLE_HIGH, VAT_PAYABLE_LOW, VAT_PAYABLE_OTHER, VAT_RECEIVABLE};
use crate::vat::{self, VatReturn};

/// The outcome of one check.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Check {
    /// Everything the check looks at agrees.
    Passed,
    /// The check ran and found problems; each entry is a sentence for a person.
    Failed(Vec<String>),
    /// The check cannot run yet, and why.
    NotChecked(&'static str),
}

impl Check {
    /// True when the check found nothing to report.
    pub fn is_clean(&self) -> bool {
        !matches!(self, Self::Failed(_))
    }

    /// The reason, when this check could not run.
    pub const fn not_checked(&self) -> Option<&'static str> {
        match self {
            Self::NotChecked(reason) => Some(*reason),
            Self::Passed | Self::Failed(_) => None,
        }
    }

    /// The problems found, if any.
    pub fn failures(&self) -> &[String] {
        match self {
            Self::Failed(problems) => problems,
            Self::Passed | Self::NotChecked(_) => &[],
        }
    }
}

/// Everything that has to hold before a period may be marked prepared.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Readiness {
    /// Year of the period.
    pub year: i32,
    /// Month of the period.
    pub period_no: u8,
    /// Reporting currency of the books.
    pub currency: Currency,
    /// Every VAT-bearing line found a rubriek.
    pub ledger_to_return: Check,
    /// The VAT posted for each document is the VAT on that document.
    pub invoice_to_ledger: Check,
    /// The books balance up to the end of the period.
    pub balance_sheet: Check,
    /// Control accounts against the subledger; not implemented in v1.1.
    pub control_accounts: Check,
    /// The computed return, so a caller does not compute it twice.
    pub report: VatReturn,
}

impl Readiness {
    /// Every reason the period is not ready, in the order the checks ran.
    ///
    /// A check that cannot run is not a reason to block: refusing to prepare a
    /// return because a future feature is missing would leave the books unfileable
    /// forever.
    pub fn blockers(&self) -> Vec<String> {
        let mut blockers = Vec::new();
        blockers.extend(self.ledger_to_return.failures().iter().cloned());
        blockers.extend(self.invoice_to_ledger.failures().iter().cloned());
        blockers.extend(self.balance_sheet.failures().iter().cloned());
        blockers
    }

    /// True when nothing blocks preparation.
    pub fn is_ready(&self) -> bool {
        self.blockers().is_empty()
    }

    /// Every check that could not run, with its reason.
    pub fn unchecked(&self) -> Vec<&'static str> {
        [
            &self.ledger_to_return,
            &self.invoice_to_ledger,
            &self.balance_sheet,
            &self.control_accounts,
        ]
        .into_iter()
        .filter_map(Check::not_checked)
        .collect()
    }
}

/// The control accounts agree with the sum of their open items.
///
/// The subledger is cut at the same date as the control account, because an
/// invoice dated next month belongs to next month's control balance as much as to
/// next month's return.
fn control_accounts(
    conn: &Connection,
    seller_id: SellerId,
    year: i32,
    period_no: u8,
    currency: Currency,
) -> Result<Check, StoreError> {
    let (_, ends) = vat::period_bounds(year, period_no).map_err(|error| StoreError::Data(error.to_string()))?;
    let beginning = LocalDate::from_ymd(1, 1, 1).map_err(|error| StoreError::Data(error.to_string()))?;
    let balance = crate::report::trial_balance(conn, seller_id, beginning, ends)?;

    let mut problems = Vec::new();
    for kind in [
        crate::open_items::OpenItemKind::Receivable,
        crate::open_items::OpenItemKind::Payable,
    ] {
        let control = kind.control_account();
        let posted = balance
            .rows
            .iter()
            .find(|row| row.code.as_str() == control)
            .map_or(0, crate::report::AccountBalance::balance_micros);
        let subledger: Option<i64> = conn.query_row(
            "SELECT sum(amount_micros) FROM open_item
             WHERE seller_id = ?1 AND kind = ?2 AND original_date_local <= ?3",
            rusqlite::params![seller_id.to_string(), kind.as_str(), ends.to_iso_string()],
            |row| row.get(0),
        )?;
        let subledger = subledger.unwrap_or(0);
        if posted != subledger {
            problems.push(format!(
                "{control} ({}) is {} in the ledger and {} in the subledger",
                kind.as_str(),
                money(posted, currency),
                money(subledger, currency)
            ));
            // The usual cause is a document whose counterparty was never matched:
            // it posted to the control account and cannot appear in the subledger.
            let unmatched: i64 = conn.query_row(
                "SELECT count(*) FROM document d
                 JOIN journal_entry e ON e.source_id = d.id
                 WHERE d.owner_id = ?1 AND d.counterparty_id IS NULL
                   AND d.issue_date_local <= ?2",
                rusqlite::params![seller_id.to_string(), ends.to_iso_string()],
                |row| row.get(0),
            )?;
            if unmatched > 0 {
                problems.push(format!(
                    "{unmatched} posted document(s) up to {ends} have no counterparty and cannot appear in the subledger"
                ));
            }
        }
    }

    Ok(if problems.is_empty() {
        Check::Passed
    } else {
        Check::Failed(problems)
    })
}

/// Runs the readiness checks for one month.
pub fn readiness(conn: &Connection, seller_id: SellerId, year: i32, period_no: u8) -> Result<Readiness, StoreError> {
    let report = vat::compute(conn, seller_id, year, period_no)?;
    let ledger_to_return = if report.unmapped.is_empty() {
        Check::Passed
    } else {
        Check::Failed(
            report
                .unmapped
                .iter()
                .map(|line| format!("this line has no rubriek: {line}"))
                .collect(),
        )
    };

    Ok(Readiness {
        year,
        period_no,
        currency: report.currency,
        ledger_to_return,
        invoice_to_ledger: invoice_to_ledger(conn, seller_id, year, period_no, report.currency)?,
        balance_sheet: balance_sheet(conn, seller_id, year, period_no)?,
        control_accounts: control_accounts(conn, seller_id, year, period_no, report.currency)?,
        report,
    })
}

/// Compares the VAT on each document in the period with the VAT posted for it.
///
/// Document by document rather than in total, because "the totals agree" can hide
/// a document that posted nothing and another that posted twice. A document with
/// no entry is a blocker: a return over half a month is worse than no return.
fn invoice_to_ledger(
    conn: &Connection,
    seller_id: SellerId,
    year: i32,
    period_no: u8,
    currency: Currency,
) -> Result<Check, StoreError> {
    let (starts, ends) = vat::period_bounds(year, period_no).map_err(|error| StoreError::Data(error.to_string()))?;

    let mut statement = conn.prepare(
        "SELECT d.direction, d.number, d.doc_type, d.tax_breakdown_json, e.id
         FROM document d
         LEFT JOIN journal_entry e
                ON e.source_id = d.id
               AND e.source_kind = CASE d.direction
                                       WHEN 'outbound' THEN 'outbound_document'
                                       ELSE 'inbound_document'
                                   END
         WHERE d.owner_id = ?1 AND d.issue_date_local >= ?2 AND d.issue_date_local <= ?3
         ORDER BY d.issue_date_local, d.number",
    )?;
    let mut rows = statement.query(rusqlite::params![
        seller_id.to_string(),
        starts.to_iso_string(),
        ends.to_iso_string()
    ])?;

    let mut problems = Vec::new();
    while let Some(row) = rows.next()? {
        let direction: String = row.get(0)?;
        let number: String = row.get(1)?;
        let doc_type: String = row.get(2)?;
        let breakdown: String = row.get(3)?;
        let entry: Option<String> = row.get(4)?;

        let Some(entry) = entry else {
            problems.push(format!("{number} is not posted"));
            continue;
        };

        let document_tax = signed_document_tax(&breakdown, &doc_type, currency)
            .map_err(|error| StoreError::Data(format!("document {number}: {error}")))?;

        if direction == "outbound" {
            let posted = vat_account_net(conn, &entry, &[VAT_PAYABLE_HIGH, VAT_PAYABLE_LOW, VAT_PAYABLE_OTHER])?;
            if posted != document_tax {
                problems.push(format!(
                    "{number}: the document says {}, the ledger says {}",
                    money(document_tax, currency),
                    money(posted, currency)
                ));
            }
        } else {
            // Input VAT is what the supplier charged plus whatever we assessed on
            // the supply ourselves; both post to the deductible account.
            let self_assessed = vat_account_net(conn, &entry, &[VAT_PAYABLE_OTHER])?;
            let posted = -vat_account_net(conn, &entry, &[VAT_RECEIVABLE])?;
            let expected = document_tax + self_assessed;
            if posted != expected {
                problems.push(format!(
                    "{number}: the document and the assessment come to {}, the ledger says {}",
                    money(expected, currency),
                    money(posted, currency)
                ));
            }
        }
    }

    Ok(if problems.is_empty() {
        Check::Passed
    } else {
        Check::Failed(problems)
    })
}

/// The VAT on the documents in the period, signed by their type.
///
/// A credit note carries a positive amount on paper and subtracts from the
/// period, which is how the ledger already treats it.
fn signed_document_tax(breakdown: &str, doc_type: &str, currency: Currency) -> Result<i64, String> {
    let rows: Value = serde_json::from_str(breakdown).map_err(|error| format!("tax breakdown: {error}"))?;
    let rows = rows
        .as_array()
        .ok_or_else(|| "tax breakdown is not a list".to_owned())?;
    let mut total = 0_i64;
    for row in rows {
        let tax = row
            .get("tax")
            .and_then(Value::as_str)
            .ok_or_else(|| "tax breakdown row has no tax".to_owned())?;
        total += Money::parse_fixed(tax, currency)
            .map_err(|error| error.to_string())?
            .micros();
    }
    Ok(if doc_type == "381" { -total } else { total })
}

/// Net movement of one entry across a set of accounts, credit positive.
fn vat_account_net(conn: &Connection, entry: &str, accounts: &[&str]) -> Result<i64, StoreError> {
    let placeholders = vec!["?"; accounts.len()].join(", ");
    let sql = format!(
        "SELECT sum(CASE side WHEN 'credit' THEN amount_micros ELSE -amount_micros END)
         FROM journal_line WHERE entry_id = ? AND account_code IN ({placeholders})"
    );
    let mut statement = conn.prepare(&sql)?;
    let mut values: Vec<String> = vec![entry.to_owned()];
    values.extend(accounts.iter().map(|account| (*account).to_owned()));
    let net: Option<i64> = statement.query_row(rusqlite::params_from_iter(values), |row| row.get(0))?;
    Ok(net.unwrap_or(0))
}

/// The books balance up to the end of the period.
fn balance_sheet(conn: &Connection, seller_id: SellerId, year: i32, period_no: u8) -> Result<Check, StoreError> {
    let (_, ends) = vat::period_bounds(year, period_no).map_err(|error| StoreError::Data(error.to_string()))?;
    let beginning = LocalDate::from_ymd(1, 1, 1).map_err(|error| StoreError::Data(error.to_string()))?;
    let balance = crate::report::trial_balance(conn, seller_id, beginning, ends)?;

    Ok(if balance.balances() {
        Check::Passed
    } else {
        Check::Failed(vec![format!(
            "the trial balance is off by {} up to {ends}",
            money(balance.difference_micros(), balance.currency)
        )])
    })
}

/// Micro-units as an amount, for a sentence.
fn money(micros: i64, currency: Currency) -> String {
    Money::from_micros(micros, currency)
        .to_fixed_string(2)
        .map(|text| format!("{text} {currency}"))
        .unwrap_or_else(|_| format!("{micros} u"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_check_that_cannot_run_is_not_a_failure() {
        assert!(Check::NotChecked("later").is_clean());
        assert!(Check::Passed.is_clean());
        assert!(!Check::Failed(vec!["x".to_owned()]).is_clean());
        assert_eq!(Check::Failed(vec!["x".to_owned()]).failures().len(), 1);
    }
}
