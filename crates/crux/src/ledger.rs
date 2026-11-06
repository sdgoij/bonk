//! Ledger primitives: accounts, journals, and entries that balance.
//!
//! The accounting layer is a projection of the invoice archive: a posting rule
//! turns a fact — an issued invoice, a payment, a bank line — into a journal
//! entry. This module owns the shape of those entries and the invariants that
//! hold before one is written anywhere, because a store that accepts an
//! unbalanced entry has already lost the argument.
//!
//! What is deliberately absent: balances (they are queries over lines), posting
//! rules (they derive entries from documents), periods and locks (storage facts
//! with their own audit trail), and VAT rubriek mappings (a versioned artefact).
//! Keeping the derived things out is what stops this module from growing a
//! second copy of the truth.

use core::fmt;
use std::str::FromStr;

use crate::error::DomainError;
use crate::money::{Currency, Money};

/// Longest account code the ledger accepts.
///
/// Long enough for an RGS-style code with a suffix (`WMVb1`, `7000-09`), short
/// enough that it still fits a column in a report.
pub const MAX_ACCOUNT_CODE_LEN: usize = 16;

/// A user-visible account code such as `1300` or `WMVb1`.
///
/// Accounts are rows, not an enum: every business argues about its chart, and
/// the Dutch *Referentie Grootboek Schema* maps onto it rather than replacing
/// it. The code is what the user types on a bank line and what an accountant
/// reads on a printout, so it is stable for the life of the account.
///
/// Ordering is by text, which is what a printed chart looks like. `"1300"` sorts
/// before `"900"`; the interface groups by [`AccountKind`] and shows the code as
/// a string, so this never decides what a user sees first.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountCode(String);

impl AccountCode {
    /// Parses a code, rejecting anything that is not a compact identifier.
    ///
    /// Whitespace is rejected rather than trimmed: two codes that render alike
    /// but differ by a space would be two accounts, and every report would be
    /// subtly wrong.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        if value.is_empty() || value.len() > MAX_ACCOUNT_CODE_LEN {
            return Err(DomainError::InvalidAccountCode(value.to_owned()));
        }
        let mut characters = value.chars();
        let starts_well = characters
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric());
        let rest_is_fine =
            characters.all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '.'));
        if !starts_well || !rest_is_fine {
            return Err(DomainError::InvalidAccountCode(value.to_owned()));
        }
        Ok(Self(value.to_owned()))
    }

    /// The code as the user knows it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AccountCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for AccountCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AccountCode({})", self.0)
    }
}

impl FromStr for AccountCode {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// The five kinds of account, which is all the balance sheet and the P&L need.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum AccountKind {
    /// Things owned: bank, debtors, equipment.
    Asset,
    /// Things owed: creditors, VAT payable.
    Liability,
    /// The owner's stake, including retained results.
    Equity,
    /// Income from sales.
    Revenue,
    /// Costs, including deductible VAT.
    Expense,
}

impl AccountKind {
    /// Every kind, in the order a chart of accounts is usually presented.
    pub const ALL: [Self; 5] = [Self::Asset, Self::Liability, Self::Equity, Self::Revenue, Self::Expense];

    /// The stable storage token, also used in exports an accountant reads.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Asset => "asset",
            Self::Liability => "liability",
            Self::Equity => "equity",
            Self::Revenue => "revenue",
            Self::Expense => "expense",
        }
    }

    /// Parses the storage token.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "asset" => Ok(Self::Asset),
            "liability" => Ok(Self::Liability),
            "equity" => Ok(Self::Equity),
            "revenue" => Ok(Self::Revenue),
            "expense" => Ok(Self::Expense),
            other => Err(DomainError::InvalidAccountKind(other.to_owned())),
        }
    }

    /// The side on which a balance of this kind grows.
    ///
    /// A debit balance on a liability is not an error; it is a negative
    /// liability, and reports present it that way rather than flipping the sign.
    pub const fn normal_side(self) -> Side {
        match self {
            Self::Asset | Self::Expense => Side::Debit,
            Self::Liability | Self::Equity | Self::Revenue => Side::Credit,
        }
    }

    /// True for the kinds a balance sheet shows.
    pub const fn is_balance_sheet(self) -> bool {
        matches!(self, Self::Asset | Self::Liability | Self::Equity)
    }
}

/// Which side of an entry a line sits on.
///
/// The side is a direction, never a sign: a line's amount is always positive and
/// the side says which way it moves the balance. Storing signed amounts would
/// let a "negative debit" exist, and then nothing means anything.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Side {
    /// Left: assets, costs.
    Debit,
    /// Right: liabilities, equity, income.
    Credit,
}

impl Side {
    /// The stable storage token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Debit => "debit",
            Self::Credit => "credit",
        }
    }

    /// Parses the storage token.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "debit" => Ok(Self::Debit),
            "credit" => Ok(Self::Credit),
            other => Err(DomainError::InvalidSide(other.to_owned())),
        }
    }

    /// The other side.
    pub const fn opposite(self) -> Self {
        match self {
            Self::Debit => Self::Credit,
            Self::Credit => Self::Debit,
        }
    }
}

/// A book of related entries.
///
/// The set is fixed because numbering, reporting, and the audit trail are all per
/// journal, and a user-invented journal would need its own numbering policy. A
/// business that needs a second cash book uses two cash accounts in one journal,
/// which is the conventional answer anyway.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Journal {
    /// Sales: invoices and credit notes we issued (`VK`).
    Sales,
    /// Purchases: invoices we received (`IK`).
    Purchase,
    /// Bank statements and payments (`BNK`).
    Bank,
    /// Cash (`KAS`).
    Cash,
    /// Manual entries, the accountant's escape hatch (`MEM`).
    Memorial,
    /// Opening balances at the start date (`OPEN`).
    Opening,
    /// VAT settlements and corrections (`BTW`).
    Vat,
}

impl Journal {
    /// Every journal, in the order the interface lists them.
    pub const ALL: [Self; 7] = [
        Self::Sales,
        Self::Purchase,
        Self::Bank,
        Self::Cash,
        Self::Memorial,
        Self::Opening,
        Self::Vat,
    ];

    /// The stable code, as printed in a journal listing.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Sales => "VK",
            Self::Purchase => "IK",
            Self::Bank => "BNK",
            Self::Cash => "KAS",
            Self::Memorial => "MEM",
            Self::Opening => "OPEN",
            Self::Vat => "BTW",
        }
    }

    /// Parses a journal code.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "VK" => Ok(Self::Sales),
            "IK" => Ok(Self::Purchase),
            "BNK" => Ok(Self::Bank),
            "KAS" => Ok(Self::Cash),
            "MEM" => Ok(Self::Memorial),
            "OPEN" => Ok(Self::Opening),
            "BTW" => Ok(Self::Vat),
            other => Err(DomainError::InvalidJournal(other.to_owned())),
        }
    }
}

/// The VAT treatment a line carries, when it has one.
///
/// It travels with the line rather than being re-derived from the account later:
/// two purchases can sit on the same costs account and still belong in different
/// boxes of a VAT return, and a report that guessed from the account would be
/// inventing the answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VatTag {
    /// EN 16931 category of the document line that produced this one.
    pub category: crate::vat::VatCategory,
    /// Rate in basis points.
    pub rate_bp: u16,
}

/// One line of a journal entry.
///
/// The amount is strictly positive and carries the entry's currency; the side
/// carries the direction. Construction is checked, so a line that exists is a
/// line that can be posted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LedgerLine {
    account: AccountCode,
    side: Side,
    amount: Money,
    vat: Option<VatTag>,
    tax_code: Option<crate::vat::TaxCode>,
}

impl LedgerLine {
    /// Builds a line, rejecting zero and negative amounts.
    pub fn new(account: AccountCode, side: Side, amount: Money) -> Result<Self, DomainError> {
        if !amount.is_positive() {
            return Err(DomainError::AmountNotPositive(amount.micros()));
        }
        Ok(Self {
            account,
            side,
            amount,
            vat: None,
            tax_code: None,
        })
    }

    /// Attaches the VAT treatment of the document line behind this one.
    pub fn tagged(mut self, category: crate::vat::VatCategory, rate: crate::vat::VatRate) -> Self {
        self.vat = Some(VatTag {
            category,
            rate_bp: rate.basis_points(),
        });
        self
    }

    /// Names the treatment the books file this line under, when one was chosen.
    ///
    /// The code resolves what the category alone cannot say — whether an `AE`
    /// purchase is goods or services — and travels with the line because a report
    /// re-deriving it from the document later would be reading a fact that can
    /// still change.
    pub fn coded(mut self, code: crate::vat::TaxCode) -> Self {
        self.tax_code = Some(code);
        self
    }

    /// The treatment, when the line has a VAT meaning.
    pub const fn vat(&self) -> Option<VatTag> {
        self.vat
    }

    /// The tax code the line was posted under, when one was chosen.
    pub fn tax_code(&self) -> Option<&crate::vat::TaxCode> {
        self.tax_code.as_ref()
    }

    /// A debit line.
    pub fn debit(account: AccountCode, amount: Money) -> Result<Self, DomainError> {
        Self::new(account, Side::Debit, amount)
    }

    /// A credit line.
    pub fn credit(account: AccountCode, amount: Money) -> Result<Self, DomainError> {
        Self::new(account, Side::Credit, amount)
    }

    /// The account this line moves.
    pub fn account(&self) -> &AccountCode {
        &self.account
    }

    /// The side the line sits on.
    pub const fn side(&self) -> Side {
        self.side
    }

    /// The positive amount.
    pub const fn amount(&self) -> Money {
        self.amount
    }

    /// The same amount on the other side.
    pub fn mirrored(&self) -> Self {
        Self {
            account: self.account.clone(),
            side: self.side.opposite(),
            amount: self.amount,
            vat: self.vat,
            tax_code: self.tax_code.clone(),
        }
    }
}

/// A balanced set of lines moving value between accounts.
///
/// An entry is a statement of one economic event: it names the journal, says in
/// words what happened, and balances. Everything the store adds on top — the
/// period, the number, the hash, the source document — is context around this
/// value, not part of it.
///
/// Entries are immutable. A wrong entry is corrected by mirroring it into a
/// reversing entry and posting the right one, so the mistake stays visible in the
/// books instead of disappearing from them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LedgerEntry {
    journal: Journal,
    description: String,
    currency: Currency,
    lines: Vec<LedgerLine>,
}

impl LedgerEntry {
    /// Builds an entry, rejecting anything the ledger cannot hold.
    ///
    /// The checks are ordered: an empty entry is reported as empty rather than as
    /// unbalanced, because "no lines" is the actionable message.
    pub fn new(journal: Journal, description: impl Into<String>, lines: Vec<LedgerLine>) -> Result<Self, DomainError> {
        let description = description.into();
        if description.trim().is_empty() {
            return Err(DomainError::EmptyText("journal entry description"));
        }
        if lines.is_empty() {
            return Err(DomainError::EmptyEntry);
        }

        let currency = lines[0].amount.currency();
        for line in &lines {
            if line.amount.currency() != currency {
                return Err(DomainError::CurrencyMismatch {
                    left: currency.to_string(),
                    right: line.amount.currency().to_string(),
                });
            }
        }

        let entry = Self {
            journal,
            description,
            currency,
            lines,
        };
        let debits = entry.side_total(Side::Debit)?;
        let credits = entry.side_total(Side::Credit)?;
        if debits.micros() != credits.micros() {
            return Err(DomainError::EntryUnbalanced {
                debit_micros: debits.micros(),
                credit_micros: credits.micros(),
            });
        }
        Ok(entry)
    }

    /// The journal the entry belongs to.
    pub const fn journal(&self) -> Journal {
        self.journal
    }

    /// What the entry says happened, in words.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The single currency every line uses.
    pub const fn currency(&self) -> Currency {
        self.currency
    }

    /// The lines, in the order they were built.
    pub fn lines(&self) -> &[LedgerLine] {
        &self.lines
    }

    /// The sum of the debit lines.
    pub fn debit_total(&self) -> Result<Money, DomainError> {
        self.side_total(Side::Debit)
    }

    /// The sum of the credit lines.
    pub fn credit_total(&self) -> Result<Money, DomainError> {
        self.side_total(Side::Credit)
    }

    /// The entry that undoes this one: every line on the other side.
    ///
    /// The reversal carries its own description, because "reversal of X" is a
    /// statement about why the correction exists and the store records that
    /// reason separately from the arithmetic.
    pub fn reversed(&self, description: impl Into<String>) -> Result<Self, DomainError> {
        let lines = self.lines.iter().map(LedgerLine::mirrored).collect();
        Self::new(self.journal, description, lines)
    }

    fn side_total(&self, side: Side) -> Result<Money, DomainError> {
        self.lines
            .iter()
            .filter(|line| line.side == side)
            .try_fold(Money::zero(self.currency), |total, line| total.checked_add(line.amount))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn account(code: &str) -> AccountCode {
        AccountCode::parse(code).unwrap()
    }

    fn euros(units: i64) -> Money {
        Money::from_major_units(units, Currency::EUR).unwrap()
    }

    /// Kept out of the `prop_assert!` invocation, which stringifies its argument
    /// into a format string and would choke on the braces of a struct pattern.
    fn is_unbalanced(result: Result<LedgerEntry, DomainError>) -> bool {
        matches!(result, Err(DomainError::EntryUnbalanced { .. }))
    }

    /// The domestic sale from the scenario matrix: 1 000 net at 21 % is 1 210.
    fn domestic_sale() -> LedgerEntry {
        LedgerEntry::new(
            Journal::Sales,
            "Invoice 2026-0001",
            vec![
                LedgerLine::debit(account("1300"), euros(1_210)).unwrap(),
                LedgerLine::credit(account("8000"), euros(1_000)).unwrap(),
                LedgerLine::credit(account("1600"), euros(210)).unwrap(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn a_balanced_entry_reports_its_totals() {
        let entry = domestic_sale();
        assert_eq!(entry.journal(), Journal::Sales);
        assert_eq!(entry.currency(), Currency::EUR);
        assert_eq!(entry.debit_total().unwrap(), euros(1_210));
        assert_eq!(entry.credit_total().unwrap(), euros(1_210));
        assert_eq!(entry.lines().len(), 3);
    }

    #[test]
    fn an_unbalanced_entry_is_rejected_with_both_totals() {
        let error = LedgerEntry::new(
            Journal::Sales,
            "Off by one",
            vec![
                LedgerLine::debit(account("1300"), euros(1_000)).unwrap(),
                LedgerLine::credit(account("8000"), euros(999)).unwrap(),
            ],
        )
        .unwrap_err();

        assert_eq!(
            error,
            DomainError::EntryUnbalanced {
                debit_micros: 1_000_000_000,
                credit_micros: 999_000_000,
            }
        );
        assert_eq!(error.code(), "entry-unbalanced");
    }

    #[test]
    fn an_entry_needs_lines_and_a_description() {
        assert_eq!(
            LedgerEntry::new(Journal::Bank, "Empty", Vec::new()).unwrap_err(),
            DomainError::EmptyEntry
        );
        assert_eq!(
            LedgerEntry::new(
                Journal::Bank,
                "   ",
                vec![LedgerLine::debit(account("1000"), euros(1)).unwrap()]
            )
            .unwrap_err(),
            DomainError::EmptyText("journal entry description")
        );
    }

    #[test]
    fn mixed_currencies_cannot_balance() {
        let dollars = Money::from_major_units(1, Currency::USD).unwrap();
        let error = LedgerEntry::new(
            Journal::Bank,
            "Two currencies",
            vec![
                LedgerLine::debit(account("1000"), euros(1)).unwrap(),
                LedgerLine::credit(account("1100"), dollars).unwrap(),
            ],
        )
        .unwrap_err();

        assert!(matches!(error, DomainError::CurrencyMismatch { .. }));
    }

    #[test]
    fn line_amounts_are_strictly_positive() {
        assert_eq!(
            LedgerLine::debit(account("1000"), Money::zero(Currency::EUR)).unwrap_err(),
            DomainError::AmountNotPositive(0)
        );
        assert_eq!(
            LedgerLine::credit(account("1000"), euros(-5)).unwrap_err(),
            DomainError::AmountNotPositive(-5_000_000)
        );
    }

    #[test]
    fn a_reversal_mirrors_every_line_and_nets_to_zero() {
        let original = domestic_sale();
        let reversal = original.reversed("Credit note 2026-C0001").unwrap();

        assert_eq!(reversal.debit_total().unwrap(), original.credit_total().unwrap());
        assert_eq!(reversal.credit_total().unwrap(), original.debit_total().unwrap());
        assert_eq!(reversal.description(), "Credit note 2026-C0001");

        // Posting both leaves every account where it started: that is what makes
        // a reversal a correction rather than an edit.
        let mut combined = original.lines().to_vec();
        combined.extend(reversal.lines().iter().cloned());
        LedgerEntry::new(Journal::Memorial, "Original and reversal", combined).unwrap();

        let net = |code: &str| -> i64 {
            original
                .lines()
                .iter()
                .chain(reversal.lines())
                .filter(|line| line.account().as_str() == code)
                .map(|line| match line.side() {
                    Side::Debit => line.amount().micros(),
                    Side::Credit => -line.amount().micros(),
                })
                .sum()
        };
        assert_eq!(net("1300"), 0);
        assert_eq!(net("8000"), 0);
        assert_eq!(net("1600"), 0);
    }

    #[test]
    fn account_codes_accept_what_an_accountant_types() {
        for good in ["1300", "8000", "WMVb1", "7000-09", "1.2.3"] {
            assert!(AccountCode::parse(good).is_ok(), "{good} should be a code");
        }
        // Whitespace is rejected, not trimmed: `" 1300"` and `"1300"` must not
        // become two accounts that print the same.
        for bad in ["", " 1300", "1300 ", "130 0", "#1300", "a/b"] {
            assert!(AccountCode::parse(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(
            AccountCode::parse("12345678901234567").is_err(),
            "over the length limit"
        );
        assert!(AccountCode::parse("1234567890123456").is_ok(), "exactly at the limit");
    }

    #[test]
    fn account_kinds_know_their_side_and_round_trip() {
        for kind in AccountKind::ALL {
            assert_eq!(AccountKind::parse(kind.as_str()).unwrap(), kind);
        }
        assert_eq!(AccountKind::Asset.normal_side(), Side::Debit);
        assert_eq!(AccountKind::Expense.normal_side(), Side::Debit);
        assert_eq!(AccountKind::Liability.normal_side(), Side::Credit);
        assert_eq!(AccountKind::Equity.normal_side(), Side::Credit);
        assert_eq!(AccountKind::Revenue.normal_side(), Side::Credit);
        assert!(AccountKind::Asset.is_balance_sheet());
        assert!(!AccountKind::Revenue.is_balance_sheet());
        assert!(AccountKind::parse("turnover").is_err());
    }

    #[test]
    fn journals_round_trip_through_their_codes() {
        for journal in Journal::ALL {
            assert_eq!(Journal::parse(journal.code()).unwrap(), journal);
        }
        assert_eq!(Journal::Sales.code(), "VK");
        assert_eq!(Journal::Opening.code(), "OPEN");
        assert!(Journal::parse("XYZ").is_err());
    }

    #[test]
    fn sides_round_trip_and_oppose() {
        for side in [Side::Debit, Side::Credit] {
            assert_eq!(Side::parse(side.as_str()).unwrap(), side);
            assert_eq!(side.opposite().opposite(), side);
        }
        assert!(Side::parse("Dr").is_err());
    }

    proptest! {
        /// Any set of offsetting pairs balances, whatever the amounts.
        #[test]
        fn offsetting_pairs_always_balance(amounts in proptest::collection::vec(1_i64..=1_000_000, 1..8)) {
            let mut lines = Vec::new();
            for units in &amounts {
                let amount = Money::from_major_units(*units, Currency::EUR).unwrap();
                lines.push(LedgerLine::debit(account("1300"), amount).unwrap());
                lines.push(LedgerLine::credit(account("8000"), amount).unwrap());
            }
            let entry = LedgerEntry::new(Journal::Sales, "Paired", lines).unwrap();
            prop_assert_eq!(entry.debit_total().unwrap(), entry.credit_total().unwrap());
        }

        /// Changing one amount by a single micro-unit breaks the balance.
        #[test]
        fn a_one_micro_difference_is_caught(units in 1_i64..=1_000_000, slip in 1_i64..=1_000_000) {
            let lines = vec![
                LedgerLine::debit(account("1300"), Money::from_major_units(units + slip, Currency::EUR).unwrap()).unwrap(),
                LedgerLine::credit(account("8000"), Money::from_major_units(units, Currency::EUR).unwrap()).unwrap(),
            ];
            prop_assert!(is_unbalanced(LedgerEntry::new(Journal::Sales, "Slipped", lines)));
        }
    }
}
