//! Reports over the ledger.
//!
//! Every number here is a query over `journal_line`; no balance is stored, so a
//! report can never disagree with the entries that produced it. Amounts are the
//! lines' `base_amount_micros`, which is the reporting currency of the books —
//! keeping the report in one currency is what makes a total meaningful.
//!
//! The two assertions that make the books trustworthy are properties of the
//! reports themselves: a trial balance whose debit and credit columns differ, or a
//! balance sheet whose assets do not equal liabilities plus equity plus the
//! current result, is reported as it is rather than rounded into agreement.

use crux::{AccountCode, AccountKind, Currency, LocalDate, SellerId};
use rusqlite::{Connection, params};

use crate::error::StoreError;

/// Earliest date a report can start from, for cumulative reports.
const FROM_THE_BEGINNING: &str = "0000-01-01";

/// One account's movement in a window.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AccountBalance {
    /// The account.
    pub code: AccountCode,
    /// Its name, as stored, so a report renders without a second lookup per row.
    pub name: String,
    /// What kind of account it is, which decides how a reader reads the balance.
    pub kind: AccountKind,
    /// Sum of the debit lines.
    pub debit_micros: i64,
    /// Sum of the credit lines.
    pub credit_micros: i64,
}

impl AccountBalance {
    /// Debits minus credits: positive is a debit balance.
    pub fn net_micros(&self) -> i64 {
        self.debit_micros - self.credit_micros
    }

    /// The balance in the account's normal direction.
    ///
    /// A liability with more debits than credits is a negative liability, not a
    /// positive asset, so the sign is kept rather than flipped.
    pub fn balance_micros(&self) -> i64 {
        match self.kind.normal_side() {
            crux::Side::Debit => self.net_micros(),
            crux::Side::Credit => -self.net_micros(),
        }
    }

    /// True when anything moved on the account in the window.
    pub fn has_movement(&self) -> bool {
        self.debit_micros != 0 || self.credit_micros != 0
    }
}

/// Every account that moved in a window, with both columns.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrialBalance {
    /// Reporting currency of the books.
    pub currency: Currency,
    /// First day, inclusive.
    pub from: LocalDate,
    /// Last day, inclusive.
    pub to: LocalDate,
    /// Accounts with movement, in code order.
    pub rows: Vec<AccountBalance>,
    /// Sum of the debit column.
    pub debit_total_micros: i64,
    /// Sum of the credit column.
    pub credit_total_micros: i64,
}

impl TrialBalance {
    /// True when the two columns agree, which they must for a double-entry ledger.
    pub fn balances(&self) -> bool {
        self.debit_total_micros == self.credit_total_micros
    }

    /// The difference between the columns; zero for correct books.
    pub fn difference_micros(&self) -> i64 {
        self.debit_total_micros - self.credit_total_micros
    }
}

/// Revenue and expenses over a window, and the result they produce.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProfitAndLoss {
    /// Reporting currency of the books.
    pub currency: Currency,
    /// First day, inclusive.
    pub from: LocalDate,
    /// Last day, inclusive.
    pub to: LocalDate,
    /// Revenue accounts, in code order.
    pub revenue: Vec<AccountBalance>,
    /// Expense accounts, in code order.
    pub expenses: Vec<AccountBalance>,
    /// Revenue in its normal direction.
    pub revenue_micros: i64,
    /// Expenses in their normal direction.
    pub expense_micros: i64,
}

impl ProfitAndLoss {
    /// Revenue minus expenses; negative is a loss.
    pub fn result_micros(&self) -> i64 {
        self.revenue_micros - self.expense_micros
    }
}

/// The balance sheet at a date, with the current result folded in as equity.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BalanceSheet {
    /// Reporting currency of the books.
    pub currency: Currency,
    /// The date the sheet is drawn at, inclusive.
    pub as_of: LocalDate,
    /// Asset accounts.
    pub assets: Vec<AccountBalance>,
    /// Liability accounts.
    pub liabilities: Vec<AccountBalance>,
    /// Equity accounts.
    pub equity: Vec<AccountBalance>,
    /// Assets in their normal direction.
    pub assets_micros: i64,
    /// Liabilities in their normal direction.
    pub liabilities_micros: i64,
    /// Equity in its normal direction, without the current result.
    pub equity_micros: i64,
    /// Revenue minus expenses up to the date: the result not yet appropriated.
    pub result_micros: i64,
}

impl BalanceSheet {
    /// Right-hand side: what the business owes plus what the owners own.
    pub fn liabilities_and_equity_micros(&self) -> i64 {
        self.liabilities_micros + self.equity_micros + self.result_micros
    }

    /// True when assets equal liabilities plus equity plus the current result.
    pub fn balances(&self) -> bool {
        self.assets_micros == self.liabilities_and_equity_micros()
    }

    /// The difference between the two sides; zero for correct books.
    pub fn difference_micros(&self) -> i64 {
        self.assets_micros - self.liabilities_and_equity_micros()
    }
}

/// Sums every account's movement between two dates.
pub fn trial_balance(
    conn: &Connection,
    seller_id: SellerId,
    from: LocalDate,
    to: LocalDate,
) -> Result<TrialBalance, StoreError> {
    if from > to {
        return Err(StoreError::Rejected(format!(
            "the window ends before it starts: {from} to {to}"
        )));
    }
    let currency = reporting_currency(conn, seller_id)?;
    let rows = balances_between(conn, seller_id, &from.to_iso_string(), &to.to_iso_string())?;
    let debit_total_micros = rows.iter().map(|row| row.debit_micros).sum();
    let credit_total_micros = rows.iter().map(|row| row.credit_micros).sum();
    Ok(TrialBalance {
        currency,
        from,
        to,
        rows,
        debit_total_micros,
        credit_total_micros,
    })
}

/// Revenue and expenses between two dates.
pub fn profit_and_loss(
    conn: &Connection,
    seller_id: SellerId,
    from: LocalDate,
    to: LocalDate,
) -> Result<ProfitAndLoss, StoreError> {
    if from > to {
        return Err(StoreError::Rejected(format!(
            "the window ends before it starts: {from} to {to}"
        )));
    }
    let currency = reporting_currency(conn, seller_id)?;
    let rows = balances_between(conn, seller_id, &from.to_iso_string(), &to.to_iso_string())?;

    let mut revenue = Vec::new();
    let mut expenses = Vec::new();
    for row in rows {
        match row.kind {
            AccountKind::Revenue => revenue.push(row),
            AccountKind::Expense => expenses.push(row),
            _ => {}
        }
    }
    let revenue_micros = revenue.iter().map(AccountBalance::balance_micros).sum();
    let expense_micros = expenses.iter().map(AccountBalance::balance_micros).sum();

    Ok(ProfitAndLoss {
        currency,
        from,
        to,
        revenue,
        expenses,
        revenue_micros,
        expense_micros,
    })
}

/// The balance sheet at a date.
///
/// The current result is not posted to equity yet: it is derived from the same
/// revenue and expense accounts the P&L shows, so the two reports cannot disagree
/// about it.
pub fn balance_sheet(conn: &Connection, seller_id: SellerId, as_of: LocalDate) -> Result<BalanceSheet, StoreError> {
    let currency = reporting_currency(conn, seller_id)?;
    let rows = balances_between(conn, seller_id, FROM_THE_BEGINNING, &as_of.to_iso_string())?;

    let mut assets = Vec::new();
    let mut liabilities = Vec::new();
    let mut equity = Vec::new();
    let mut result_micros = 0_i64;
    for row in rows {
        match row.kind {
            AccountKind::Asset => assets.push(row),
            AccountKind::Liability => liabilities.push(row),
            AccountKind::Equity => equity.push(row),
            AccountKind::Revenue => result_micros += row.balance_micros(),
            AccountKind::Expense => result_micros -= row.balance_micros(),
        }
    }

    let assets_micros = assets.iter().map(AccountBalance::balance_micros).sum();
    let liabilities_micros = liabilities.iter().map(AccountBalance::balance_micros).sum();
    let equity_micros = equity.iter().map(AccountBalance::balance_micros).sum();

    Ok(BalanceSheet {
        currency,
        as_of,
        assets,
        liabilities,
        equity,
        assets_micros,
        liabilities_micros,
        equity_micros,
        result_micros,
    })
}

/// The reporting currency of the books.
pub fn reporting_currency(conn: &Connection, seller_id: SellerId) -> Result<Currency, StoreError> {
    let raw: String = conn.query_row(
        "SELECT default_currency FROM seller WHERE id = ?1",
        params![seller_id.to_string()],
        |row| row.get(0),
    )?;
    Currency::parse(&raw).map_err(|error| StoreError::Data(error.to_string()))
}

/// Every account with movement in a date window, in code order.
fn balances_between(
    conn: &Connection,
    seller_id: SellerId,
    from: &str,
    to: &str,
) -> Result<Vec<AccountBalance>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT a.code, a.name, a.kind,
                sum(CASE l.side WHEN 'debit' THEN l.base_amount_micros ELSE 0 END),
                sum(CASE l.side WHEN 'credit' THEN l.base_amount_micros ELSE 0 END)
         FROM journal_line l
         JOIN journal_entry e ON e.id = l.entry_id
         JOIN fiscal_period p ON p.id = e.period_id
         JOIN account a ON a.code = l.account_code
         WHERE p.seller_id = ?1 AND e.entry_date_local >= ?2 AND e.entry_date_local <= ?3
         GROUP BY a.code, a.name, a.kind
         ORDER BY a.code",
    )?;
    let mut rows = statement.query(params![seller_id.to_string(), from, to])?;
    let mut balances = Vec::new();
    while let Some(row) = rows.next()? {
        let code: String = row.get(0)?;
        let kind: String = row.get(2)?;
        balances.push(AccountBalance {
            code: AccountCode::parse(&code).map_err(|error| StoreError::Data(error.to_string()))?,
            name: row.get(1)?,
            kind: AccountKind::parse(&kind).map_err(|error| StoreError::Data(error.to_string()))?,
            debit_micros: row.get(3)?,
            credit_micros: row.get(4)?,
        });
    }
    Ok(balances)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cents(micros: i64) -> i64 {
        micros / 10_000
    }

    #[test]
    fn a_balance_reads_in_its_normal_direction() {
        let asset = AccountBalance {
            code: AccountCode::parse("1100").unwrap(),
            name: "Bank".to_owned(),
            kind: AccountKind::Asset,
            debit_micros: 500_000_000,
            credit_micros: 0,
        };
        assert_eq!(asset.balance_micros(), 500_000_000);
        assert_eq!(cents(asset.net_micros()), 50_000);

        // A liability that has been overpaid still reads as a negative liability.
        let liability = AccountBalance {
            code: AccountCode::parse("1400").unwrap(),
            name: "Handelscrediteuren".to_owned(),
            kind: AccountKind::Liability,
            debit_micros: 10_000,
            credit_micros: 4_000,
        };
        assert_eq!(liability.balance_micros(), -6_000);
        assert_eq!(liability.net_micros(), 6_000);
    }
}
