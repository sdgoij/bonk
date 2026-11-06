//! VAT return preparation (BTW-aangifte).
//!
//! A return is a projection of journal lines for one period, grouped into the
//! rubrieken of the Dutch form. Nothing is typed in by hand: the numbers come out
//! of the ledger, and preparing a return stores the totals together with the
//! entries they were computed from, so a filed figure can always be explained.
//!
//! The mapping below is a versioned artefact rather than a set of loose `if`s.
//! Two things make it defensible: every account that carries VAT meaning is named
//! here or reported as unmapped, and the whole table is asserted by a test, so a
//! change to a rubriek is a change to a test that says which rubriek and why.
//!
//! What the archive cannot yet tell apart, and this version therefore does not
//! pretend to: reverse-charged *goods* (4b) from reverse-charged *services* (4a),
//! because the EN 16931 category is `AE` for both and no tax code distinguishes
//! them yet. Both land in 4a, which is the services box, and the limitation is
//! recorded where the mapping is defined rather than hidden in a summary.

use std::collections::BTreeMap;

use crux::{AccountCode, AccountKind, Currency, Journal, JournalEntryId, LocalDate, SellerId, VatCategory, VatTag};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::audit;
use crate::error::StoreError;
use crate::report::reporting_currency;

/// Version of the rubriek mapping and the rules behind it.
///
/// The Dutch form changes with the tax year; a return records which version
/// produced it, so a later change never rewrites what was filed.
pub const MAPPING_VERSION: &str = "nl-2026.1";

/// A box on the Dutch VAT return.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Rubriek {
    /// 1a — supplies at the high rate.
    HighRate,
    /// 1b — supplies at the low rate.
    LowRate,
    /// 1c — supplies at other rates.
    OtherRate,
    /// 1d — private use. Never produced in v1.1: it needs a private-use policy.
    PrivateUse,
    /// 1e — supplies at 0 % or not taxed with a right to deduct.
    ZeroOrExempt,
    /// 2a — intra-EU acquisitions.
    EuAcquisitions,
    /// 3a — intra-EU supplies.
    EuSupplies,
    /// 3b — exports outside the EU.
    Exports,
    /// 4a — reverse-charged services.
    ReverseChargeServices,
    /// 4b — reverse-charged goods. Never produced in v1.1: `AE` does not say which.
    ReverseChargeGoods,
    /// 5a — VAT due.
    VatDue,
    /// 5b — deductible VAT.
    VatDeductible,
    /// 5c — payable or refundable, derived from 5a minus 5b.
    VatPayable,
}

impl Rubriek {
    /// Every rubriek, in the order the form lists them.
    pub const ALL: [Self; 13] = [
        Self::HighRate,
        Self::LowRate,
        Self::OtherRate,
        Self::PrivateUse,
        Self::ZeroOrExempt,
        Self::EuAcquisitions,
        Self::EuSupplies,
        Self::Exports,
        Self::ReverseChargeServices,
        Self::ReverseChargeGoods,
        Self::VatDue,
        Self::VatDeductible,
        Self::VatPayable,
    ];

    /// The code on the form, e.g. `1a`.
    pub const fn code(self) -> &'static str {
        match self {
            Self::HighRate => "1a",
            Self::LowRate => "1b",
            Self::OtherRate => "1c",
            Self::PrivateUse => "1d",
            Self::ZeroOrExempt => "1e",
            Self::EuAcquisitions => "2a",
            Self::EuSupplies => "3a",
            Self::Exports => "3b",
            Self::ReverseChargeServices => "4a",
            Self::ReverseChargeGoods => "4b",
            Self::VatDue => "5a",
            Self::VatDeductible => "5b",
            Self::VatPayable => "5c",
        }
    }

    /// The Dutch wording the form uses, for a printed return.
    pub const fn label(self) -> &'static str {
        match self {
            Self::HighRate => "Leveringen/diensten belast met hoog tarief",
            Self::LowRate => "Leveringen/diensten belast met laag tarief",
            Self::OtherRate => "Leveringen/diensten belast met overige tarieven",
            Self::PrivateUse => "Privegebruik",
            Self::ZeroOrExempt => "Leveringen/diensten belast met 0% of niet bij u belast",
            Self::EuAcquisitions => "Verwerving goederen uit de EU",
            Self::EuSupplies => "Leveringen naar landen binnen de EU",
            Self::Exports => "Leveringen naar landen buiten de EU",
            Self::ReverseChargeServices => "Verlegde btw: diensten",
            Self::ReverseChargeGoods => "Verlegde btw: goederen",
            Self::VatDue => "Totaal af te dragen btw",
            Self::VatDeductible => "Totaal voorbelasting",
            Self::VatPayable => "Te betalen of terug te ontvangen",
        }
    }

    /// Parses a code as stored.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|rubriek| rubriek.code() == value)
    }

    /// True for the boxes that carry a total rather than a supply: 5a and 5c.
    pub const fn is_total(self) -> bool {
        matches!(self, Self::VatDue | Self::VatPayable)
    }

    /// True when this box's VAT is part of 5a.
    ///
    /// The supplies with a zero VAT column (1e, 3a, 3b) are boxes without VAT, and
    /// 5b is deducted rather than added. Everything else with a VAT column raises
    /// what is due.
    pub const fn counts_toward_due(self) -> bool {
        matches!(
            self,
            Self::HighRate
                | Self::LowRate
                | Self::OtherRate
                | Self::PrivateUse
                | Self::EuAcquisitions
                | Self::ReverseChargeServices
                | Self::ReverseChargeGoods
        )
    }
}

/// The box a tax code puts a purchase line in, when it applies to that line.
///
/// A code does not move the deduction: input VAT belongs in 5b whatever the supply
/// was. It decides the box of the supply and of the VAT we assessed on it, which
/// is exactly where the category alone could not say.
fn chosen_rubriek(journal: Journal, account: &AccountCode, code: Option<Rubriek>) -> Option<Rubriek> {
    if journal != Journal::Purchase || account.as_str() == crate::posting::VAT_RECEIVABLE {
        return None;
    }
    code
}

/// The rubriek a journal line belongs in, or `None` when it has no VAT meaning.
///
/// The VAT accounts are recognised by code because the rule set routes every
/// treatment to a dedicated account; everything else is decided by the treatment
/// the line carries, which is why it travels on the line rather than being
/// re-derived from the account afterwards.
pub fn rubriek_for(journal: Journal, account: &AccountCode, kind: AccountKind, vat: Option<VatTag>) -> Option<Rubriek> {
    // The VAT accounts belong in the rubriek of the supply they are the tax on,
    // not in a single "VAT due" bucket: the form shows each box's own VAT column,
    // and 5a is the sum of them rather than a separate posting destination.
    match account.as_str() {
        crate::posting::VAT_RECEIVABLE => return Some(Rubriek::VatDeductible),
        crate::posting::VAT_PAYABLE_HIGH | crate::posting::VAT_PAYABLE_LOW => {
            return match vat?.rate_bp {
                900 => Some(Rubriek::LowRate),
                _ => Some(Rubriek::HighRate),
            };
        }
        crate::posting::VAT_PAYABLE_OTHER => {
            return match (journal, vat?.category) {
                // VAT we assessed on a supplier's behalf belongs in the box of the
                // acquisition that triggered it.
                (Journal::Purchase, VatCategory::ReverseCharge) => Some(Rubriek::ReverseChargeServices),
                (Journal::Purchase, VatCategory::IntraCommunity) => Some(Rubriek::EuAcquisitions),
                _ => Some(Rubriek::OtherRate),
            };
        }
        _ => {}
    }

    let tag = vat?;
    match (journal, kind) {
        (Journal::Sales, AccountKind::Revenue) => match tag.category {
            VatCategory::Standard if tag.rate_bp == 2100 => Some(Rubriek::HighRate),
            VatCategory::Standard if tag.rate_bp == 900 => Some(Rubriek::LowRate),
            VatCategory::Standard => Some(Rubriek::OtherRate),
            VatCategory::ReverseCharge | VatCategory::IntraCommunity => Some(Rubriek::EuSupplies),
            VatCategory::Export => Some(Rubriek::Exports),
            VatCategory::ZeroRated
            | VatCategory::Exempt
            | VatCategory::OutsideScope
            | VatCategory::CanaryIslands
            | VatCategory::CeutaMelilla => Some(Rubriek::ZeroOrExempt),
        },
        (Journal::Purchase, AccountKind::Expense) => match tag.category {
            // `AE` covers goods and services alike; see the module comment.
            VatCategory::ReverseCharge => Some(Rubriek::ReverseChargeServices),
            VatCategory::IntraCommunity => Some(Rubriek::EuAcquisitions),
            _ => None,
        },
        _ => None,
    }
}

/// One rubriek's totals.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReturnLine {
    /// The box.
    pub rubriek: Rubriek,
    /// Taxable base, in micro-units of the reporting currency.
    pub base_micros: i64,
    /// VAT, in micro-units of the reporting currency.
    pub vat_micros: i64,
}

/// A return as computed, before or after it is stored.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VatReturn {
    /// Reporting currency of the books.
    pub currency: Currency,
    /// Year of the period.
    pub year: i32,
    /// Month of the period, 1–12.
    pub period_no: u8,
    /// Rubrieken with a non-zero amount, in form order.
    pub lines: Vec<ReturnLine>,
    /// The entries the numbers came from, in identifier order.
    pub basis: Vec<JournalEntryId>,
    /// Lines that carry a treatment but no rubriek, named for a human.
    ///
    /// A return with any of these is not ready to file; saying so here is what
    /// keeps an unclassifiable line from silently dropping out of the totals.
    pub unmapped: Vec<String>,
}

impl VatReturn {
    /// VAT due (5a).
    pub fn vat_due_micros(&self) -> i64 {
        self.line(Rubriek::VatDue).map_or(0, |line| line.vat_micros)
    }

    /// Deductible VAT (5b).
    pub fn vat_deductible_micros(&self) -> i64 {
        self.line(Rubriek::VatDeductible).map_or(0, |line| line.vat_micros)
    }

    /// What is payable (positive) or refundable (negative): 5a minus 5b.
    pub fn payable_micros(&self) -> i64 {
        self.vat_due_micros() - self.vat_deductible_micros()
    }

    /// One rubriek's line, when it has an amount.
    pub fn line(&self, rubriek: Rubriek) -> Option<&ReturnLine> {
        self.lines.iter().find(|line| line.rubriek == rubriek)
    }

    /// True when nothing was posted in the period.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// A stored return, with the facts of its preparation.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PreparedReturn {
    /// Identifier of the stored return.
    pub id: String,
    /// The computed figures, as filed.
    pub computed: VatReturn,
    /// Hash over the entries the figures came from.
    pub basis_hash: String,
    /// When it was prepared.
    pub prepared_at: String,
    /// The return this one replaced, if any.
    pub supersedes: Option<String>,
}

/// The period's return, computed from the ledger without storing anything.
pub fn compute(conn: &Connection, seller_id: SellerId, year: i32, period_no: u8) -> Result<VatReturn, StoreError> {
    let currency = reporting_currency(conn, seller_id)?;
    let Some(period_id) = period_id(conn, seller_id, year, period_no)? else {
        return Ok(VatReturn {
            currency,
            year,
            period_no,
            lines: Vec::new(),
            basis: Vec::new(),
            unmapped: Vec::new(),
        });
    };

    let mut statement = conn.prepare(
        "SELECT e.id, e.journal_code, l.account_code, a.kind, l.vat_category, l.vat_rate_bp,
                l.side, l.base_amount_micros, t.rubriek
         FROM journal_line l
         JOIN journal_entry e ON e.id = l.entry_id
         JOIN account a ON a.code = l.account_code
         LEFT JOIN tax_code t ON t.id = l.tax_code_id
         WHERE e.period_id = ?1
         ORDER BY e.id, l.line_no",
    )?;
    let mut rows = statement.query([period_id])?;

    let mut totals: BTreeMap<Rubriek, (i64, i64)> = BTreeMap::new();
    let mut basis: Vec<JournalEntryId> = Vec::new();
    let mut unmapped: Vec<String> = Vec::new();

    while let Some(row) = rows.next()? {
        let entry_raw: String = row.get(0)?;
        let journal_raw: String = row.get(1)?;
        let account_raw: String = row.get(2)?;
        let kind_raw: String = row.get(3)?;
        let category_raw: Option<String> = row.get(4)?;
        let rate_bp: Option<i64> = row.get(5)?;
        let side_raw: String = row.get(6)?;
        let amount: i64 = row.get(7)?;
        let code_rubriek: Option<String> = row.get(8)?;
        let code_rubriek = code_rubriek
            .map(|code| {
                Rubriek::parse(&code).ok_or_else(|| StoreError::Data(format!("a tax code names rubriek {code:?}")))
            })
            .transpose()?;

        let journal = Journal::parse(&journal_raw).map_err(|error| StoreError::Data(error.to_string()))?;
        let account = AccountCode::parse(&account_raw).map_err(|error| StoreError::Data(error.to_string()))?;
        let kind = AccountKind::parse(&kind_raw).map_err(|error| StoreError::Data(error.to_string()))?;
        let side = crux::Side::parse(&side_raw).map_err(|error| StoreError::Data(error.to_string()))?;
        let vat = match (category_raw, rate_bp) {
            (Some(category), Some(rate_bp)) => {
                let category = VatCategory::parse(&category).map_err(|error| StoreError::Data(error.to_string()))?;
                Some(VatTag {
                    category,
                    rate_bp: u16::try_from(rate_bp)
                        .map_err(|_| StoreError::Data(format!("VAT rate {rate_bp} does not fit in basis points")))?,
                })
            }
            _ => None,
        };

        let is_vat_account = matches!(
            account.as_str(),
            crate::posting::VAT_RECEIVABLE
                | crate::posting::VAT_PAYABLE_HIGH
                | crate::posting::VAT_PAYABLE_LOW
                | crate::posting::VAT_PAYABLE_OTHER
        );
        let Some(rubriek) =
            chosen_rubriek(journal, &account, code_rubriek).or_else(|| rubriek_for(journal, &account, kind, vat))
        else {
            // A domestic purchase base belongs in no box, so silence is right for
            // it. A revenue line or a VAT account without a box means a number
            // dropped out of the return, which is reported rather than dropped.
            if is_vat_account || kind == AccountKind::Revenue {
                unmapped.push(format!(
                    "{journal_raw} {account_raw} {}",
                    vat.map_or(0, |tag| tag.rate_bp)
                ));
            }
            continue;
        };

        // A VAT account's amount is VAT; a revenue or costs account's amount is a
        // taxable base. The same rubriek can receive both, which is why the
        // column is decided by the line and not by the box.
        let is_vat_line = matches!(
            account.as_str(),
            crate::posting::VAT_RECEIVABLE
                | crate::posting::VAT_PAYABLE_HIGH
                | crate::posting::VAT_PAYABLE_LOW
                | crate::posting::VAT_PAYABLE_OTHER
        );
        let signed = if side == kind.normal_side() { amount } else { -amount };
        let slot = totals.entry(rubriek).or_insert((0, 0));
        if is_vat_line {
            slot.1 += signed;
        } else {
            slot.0 += signed;
        }

        let entry_id = JournalEntryId::parse_str(&entry_raw).map_err(|error| StoreError::Data(error.to_string()))?;
        if basis.last() != Some(&entry_id) {
            basis.push(entry_id);
        }
    }

    // 5a is what the boxes above add up to, and 5c is what is left after the
    // deduction: both are stored as lines, so a prepared return reads the same
    // way whatever produced it.
    let vat_due: i64 = totals
        .iter()
        .filter(|(rubriek, _)| rubriek.counts_toward_due())
        .map(|(_, (_, vat))| *vat)
        .sum();
    let vat_deductible = totals.get(&Rubriek::VatDeductible).map_or(0, |(_, vat)| *vat);
    totals.insert(Rubriek::VatDue, (0, vat_due));
    totals.insert(Rubriek::VatPayable, (0, vat_due - vat_deductible));

    let mut lines = Vec::new();
    for (rubriek, (base, vat)) in totals {
        if base == 0 && vat == 0 {
            continue;
        }
        lines.push(ReturnLine {
            rubriek,
            base_micros: base,
            vat_micros: vat,
        });
    }

    Ok(VatReturn {
        currency,
        year,
        period_no,
        lines,
        basis,
        unmapped,
    })
}

/// Computes the period's return and stores it as prepared.
///
/// Preparing again replaces the previous one: the old row keeps its status as
/// superseded and its basis, so the books still show what was filed first and
/// why it changed.
pub fn prepare(
    conn: &Connection,
    seller_id: SellerId,
    year: i32,
    period_no: u8,
    actor: &str,
    now: &str,
) -> Result<PreparedReturn, StoreError> {
    // Refuse to mark a period prepared while a check is red: a stored return is
    // evidence, and evidence that cannot be reconciled is worse than none.
    let readiness = crate::reconcile::readiness(conn, seller_id, year, period_no)?;
    if !readiness.is_ready() {
        return Err(StoreError::Rejected(format!(
            "the period is not ready to be filed: {}",
            readiness.blockers().join("; ")
        )));
    }

    let computed = readiness.report;
    let basis_hash = basis_hash(conn, &computed.basis)?;

    let transaction = conn.unchecked_transaction()?;
    let previous: Option<String> = transaction
        .query_row(
            "SELECT id FROM vat_return
             WHERE seller_id = ?1 AND year = ?2 AND period_no = ?3 AND status = 'prepared'",
            params![seller_id.to_string(), year, period_no],
            |row| row.get(0),
        )
        .optional()?;
    if previous.is_some() {
        transaction.execute(
            "UPDATE vat_return SET status = 'superseded'
             WHERE seller_id = ?1 AND year = ?2 AND period_no = ?3 AND status = 'prepared'",
            params![seller_id.to_string(), year, period_no],
        )?;
    }

    let id = JournalEntryId::new().to_string();
    let totals = json!({
        "mapping": MAPPING_VERSION,
        "vat_due": computed.vat_due_micros(),
        "vat_deductible": computed.vat_deductible_micros(),
        "payable": computed.payable_micros(),
        "unmapped": computed.unmapped,
    });
    transaction.execute(
        "INSERT INTO vat_return
             (id, seller_id, year, period_no, status, prepared_at, basis_hash, totals_json,
              external_reference, notes)
         VALUES (?1, ?2, ?3, ?4, 'prepared', ?5, ?6, ?7, NULL, NULL)",
        params![
            id,
            seller_id.to_string(),
            year,
            period_no,
            now,
            basis_hash,
            serde_json::to_string(&totals)?
        ],
    )?;
    for line in &computed.lines {
        transaction.execute(
            "INSERT INTO vat_return_line (return_id, rubriek, base_micros, vat_micros)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, line.rubriek.code(), line.base_micros, line.vat_micros],
        )?;
    }
    for entry in &computed.basis {
        transaction.execute(
            "INSERT INTO vat_basis_posting (return_id, entry_id) VALUES (?1, ?2)",
            params![id, entry.to_string()],
        )?;
    }

    audit::append(
        &transaction,
        now,
        actor,
        "vat_return_prepared",
        "vat_return",
        &id,
        json!({
            "year": year,
            "period": period_no,
            "mapping": MAPPING_VERSION,
            "basis_hash": basis_hash,
            "vat_due": computed.vat_due_micros(),
            "vat_deductible": computed.vat_deductible_micros(),
            "payable": computed.payable_micros(),
        }),
    )?;
    transaction.commit()?;

    Ok(PreparedReturn {
        id,
        computed,
        basis_hash,
        prepared_at: now.to_owned(),
        supersedes: previous,
    })
}

/// The period's current prepared return, if one has been prepared.
pub fn prepared(
    conn: &Connection,
    seller_id: SellerId,
    year: i32,
    period_no: u8,
) -> Result<Option<PreparedReturn>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT id, prepared_at, basis_hash FROM vat_return
         WHERE seller_id = ?1 AND year = ?2 AND period_no = ?3 AND status = 'prepared'",
    )?;
    let mut rows = statement.query(params![seller_id.to_string(), year, period_no])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let id: String = row.get(0)?;
    let prepared_at: String = row.get(1)?;
    let stored_hash: String = row.get(2)?;
    drop(rows);
    drop(statement);

    let computed = stored_return(conn, &id, year, period_no)?;
    Ok(Some(PreparedReturn {
        id,
        computed,
        basis_hash: stored_hash,
        prepared_at,
        supersedes: None,
    }))
}

/// True when the stored return still matches the entries it names.
///
/// Postings are append-only, so this can only fail if the archive was edited
/// outside the application — which is exactly what it is for.
pub fn basis_is_intact(conn: &Connection, return_id: &str) -> Result<bool, StoreError> {
    let stored: String = conn.query_row("SELECT basis_hash FROM vat_return WHERE id = ?1", [return_id], |row| {
        row.get(0)
    })?;
    let mut statement =
        conn.prepare("SELECT b.entry_id FROM vat_basis_posting b WHERE b.return_id = ?1 ORDER BY b.entry_id")?;
    let mut rows = statement.query([return_id])?;
    let mut basis = Vec::new();
    while let Some(row) = rows.next()? {
        let raw: String = row.get(0)?;
        basis.push(JournalEntryId::parse_str(&raw).map_err(|error| StoreError::Data(error.to_string()))?);
    }
    Ok(basis_hash(conn, &basis)? == stored)
}

fn stored_return(conn: &Connection, id: &str, year: i32, period_no: u8) -> Result<VatReturn, StoreError> {
    let currency: String = conn.query_row(
        "SELECT s.default_currency FROM vat_return r JOIN seller s ON s.id = r.seller_id WHERE r.id = ?1",
        [id],
        |row| row.get(0),
    )?;
    let mut statement =
        conn.prepare("SELECT rubriek, base_micros, vat_micros FROM vat_return_line WHERE return_id = ?1")?;
    let mut rows = statement.query([id])?;
    let mut lines = Vec::new();
    while let Some(row) = rows.next()? {
        let code: String = row.get(0)?;
        let rubriek = Rubriek::parse(&code)
            .ok_or_else(|| StoreError::Data(format!("stored VAT return names rubriek {code:?}")))?;
        lines.push(ReturnLine {
            rubriek,
            base_micros: row.get(1)?,
            vat_micros: row.get(2)?,
        });
    }
    lines.sort_by_key(|line| line.rubriek);

    let mut basis_statement =
        conn.prepare("SELECT entry_id FROM vat_basis_posting WHERE return_id = ?1 ORDER BY entry_id")?;
    let mut basis_rows = basis_statement.query([id])?;
    let mut basis = Vec::new();
    while let Some(row) = basis_rows.next()? {
        let raw: String = row.get(0)?;
        basis.push(JournalEntryId::parse_str(&raw).map_err(|error| StoreError::Data(error.to_string()))?);
    }

    Ok(VatReturn {
        currency: Currency::parse(&currency).map_err(|error| StoreError::Data(error.to_string()))?,
        year,
        period_no,
        lines,
        basis,
        unmapped: Vec::new(),
    })
}

/// Hash over the ordered `(entry, content hash)` list.
///
/// Naming the entries by their content rather than their identity alone means a
/// return cannot be re-explained by a posting that has since been altered.
fn basis_hash(conn: &Connection, entries: &[JournalEntryId]) -> Result<String, StoreError> {
    let mut canonical = String::new();
    let mut statement = conn.prepare("SELECT content_hash FROM journal_entry WHERE id = ?1")?;
    for entry in entries {
        let content: String = statement.query_row([entry.to_string()], |row| row.get(0))?;
        canonical.push_str(&entry.to_string());
        canonical.push(':');
        canonical.push_str(&content);
        canonical.push('\n');
    }
    Ok(audit::sha256_hex(canonical.as_bytes()))
}

fn period_id(conn: &Connection, seller_id: SellerId, year: i32, period_no: u8) -> Result<Option<String>, StoreError> {
    conn.query_row(
        "SELECT id FROM fiscal_period WHERE seller_id = ?1 AND year = ?2 AND period_no = ?3",
        params![seller_id.to_string(), year, period_no],
        |row| row.get(0),
    )
    .optional()
    .map_err(StoreError::from)
}

/// The first and last day of a return period, for display.
pub fn period_bounds(year: i32, period_no: u8) -> Result<(LocalDate, LocalDate), StoreError> {
    let start = LocalDate::from_ymd(year, period_no, 1).map_err(|error| StoreError::Data(error.to_string()))?;
    let (next_year, next_month) = if period_no == 12 {
        (
            year.checked_add(1)
                .ok_or_else(|| StoreError::Data("year overflow".to_owned()))?,
            1,
        )
    } else {
        (year, period_no + 1)
    };
    let first_of_next =
        LocalDate::from_ymd(next_year, next_month, 1).map_err(|error| StoreError::Data(error.to_string()))?;
    let end = first_of_next
        .plus_days(-1)
        .map_err(|error| StoreError::Data(error.to_string()))?;
    Ok((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(raw: &str) -> AccountCode {
        AccountCode::parse(raw).unwrap()
    }

    fn tag(category: VatCategory, rate_bp: u16) -> Option<VatTag> {
        Some(VatTag { category, rate_bp })
    }

    #[test]
    fn rubriek_codes_round_trip_and_stay_unique() {
        let mut codes: Vec<&str> = Rubriek::ALL.iter().map(|rubriek| rubriek.code()).collect();
        let total = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), total, "two rubrieken share a code");
        for rubriek in Rubriek::ALL {
            assert_eq!(Rubriek::parse(rubriek.code()), Some(rubriek));
        }
    }

    /// The mapping, asserted row by row: changing a box means changing this test.
    #[test]
    fn the_shipped_mapping_is_what_the_form_says() {
        // journal, account, kind, treatment, expected box
        type Case = (Journal, &'static str, AccountKind, Option<VatTag>, Option<Rubriek>);
        let cases: Vec<Case> = vec![
            (
                Journal::Sales,
                "8000",
                AccountKind::Revenue,
                tag(VatCategory::Standard, 2100),
                Some(Rubriek::HighRate),
            ),
            (
                Journal::Sales,
                "8010",
                AccountKind::Revenue,
                tag(VatCategory::Standard, 900),
                Some(Rubriek::LowRate),
            ),
            (
                Journal::Sales,
                "8000",
                AccountKind::Revenue,
                tag(VatCategory::Standard, 600),
                Some(Rubriek::OtherRate),
            ),
            (
                Journal::Sales,
                "8020",
                AccountKind::Revenue,
                tag(VatCategory::ReverseCharge, 0),
                Some(Rubriek::EuSupplies),
            ),
            (
                Journal::Sales,
                "8030",
                AccountKind::Revenue,
                tag(VatCategory::IntraCommunity, 0),
                Some(Rubriek::EuSupplies),
            ),
            (
                Journal::Sales,
                "8040",
                AccountKind::Revenue,
                tag(VatCategory::Export, 0),
                Some(Rubriek::Exports),
            ),
            (
                Journal::Sales,
                "8050",
                AccountKind::Revenue,
                tag(VatCategory::Exempt, 0),
                Some(Rubriek::ZeroOrExempt),
            ),
            (
                Journal::Sales,
                "8050",
                AccountKind::Revenue,
                tag(VatCategory::ZeroRated, 0),
                Some(Rubriek::ZeroOrExempt),
            ),
            (
                Journal::Purchase,
                "9999",
                AccountKind::Expense,
                tag(VatCategory::ReverseCharge, 0),
                Some(Rubriek::ReverseChargeServices),
            ),
            (
                Journal::Purchase,
                "9999",
                AccountKind::Expense,
                tag(VatCategory::IntraCommunity, 0),
                Some(Rubriek::EuAcquisitions),
            ),
            (
                Journal::Purchase,
                "9999",
                AccountKind::Expense,
                tag(VatCategory::Standard, 2100),
                None,
            ),
            (
                Journal::Sales,
                "1600",
                AccountKind::Liability,
                tag(VatCategory::Standard, 2100),
                Some(Rubriek::HighRate),
            ),
            (
                Journal::Sales,
                "1610",
                AccountKind::Liability,
                tag(VatCategory::Standard, 900),
                Some(Rubriek::LowRate),
            ),
            (
                Journal::Sales,
                "1620",
                AccountKind::Liability,
                tag(VatCategory::Standard, 600),
                Some(Rubriek::OtherRate),
            ),
            (
                Journal::Purchase,
                "1620",
                AccountKind::Liability,
                tag(VatCategory::ReverseCharge, 0),
                Some(Rubriek::ReverseChargeServices),
            ),
            (
                Journal::Purchase,
                "1620",
                AccountKind::Liability,
                tag(VatCategory::IntraCommunity, 0),
                Some(Rubriek::EuAcquisitions),
            ),
            (
                Journal::Purchase,
                "1500",
                AccountKind::Asset,
                None,
                Some(Rubriek::VatDeductible),
            ),
            (Journal::Bank, "1100", AccountKind::Asset, None, None),
        ];

        for (journal, code, kind, vat, expected) in cases {
            assert_eq!(
                rubriek_for(journal, &account(code), kind, vat),
                expected,
                "{journal:?} {code} {kind:?}"
            );
        }
    }

    #[test]
    fn period_bounds_handle_december() {
        let (start, end) = period_bounds(2026, 12).unwrap();
        assert_eq!(start.to_iso_string(), "2026-12-01");
        assert_eq!(end.to_iso_string(), "2026-12-31");
        let (start, end) = period_bounds(2024, 2).unwrap();
        assert_eq!(start.to_iso_string(), "2024-02-01");
        assert_eq!(end.to_iso_string(), "2024-02-29");
    }
}
