//! Posting rules: turning an archived document into a journal entry.
//!
//! The rule set is small, named, and versioned (`nl-sme`, version 1) because a
//! posting must be reproducible: re-running yesterday's rules over an old
//! invoice has to produce yesterday's entry, and a rule change has to be
//! visible in the entry rather than silently rewriting it.
//!
//! Scope of version 1: outbound documents. A sale is a close read of facts the
//! document already carries — gross to the debtors control account, each VAT
//! breakdown row's taxable base to a revenue account and its tax to a VAT
//! account. Purchase invoices need the reverse-charge rate policy (the supplier
//! charges nothing, we self-assess), which belongs with the tax-code mapping of
//! A3 rather than here; posting one today would have to guess a rate.

use crux::{
    AccountCode, Currency, DocumentId, ExchangeRate, Journal, LedgerEntry, LedgerLine, LocalDate, Money, SellerId,
    VatCategory, VatRate,
};
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;

use crate::error::StoreError;
use crate::ledger::{self, PostedEntry, PostingRequest, PostingSource};

/// Identifier of the shipped rule set.
pub const PROFILE_ID: &str = "nl-sme";

/// Version of the shipped rule set. Bump when the mapping below changes.
pub const RULE_VERSION: &str = "1";

/// AR control account.
pub const DEBTORS: &str = "1300";
/// AP control account.
pub const CREDITORS: &str = "1400";
/// Deductible VAT.
pub const VAT_RECEIVABLE: &str = "1500";
/// VAT payable at the high rate.
pub const VAT_PAYABLE_HIGH: &str = "1600";
/// VAT payable at the low rate.
pub const VAT_PAYABLE_LOW: &str = "1610";
/// VAT payable at any other rate.
pub const VAT_PAYABLE_OTHER: &str = "1620";

/// The expense account a purchase is posted to until someone classifies it.
///
/// Classification is line-level work that belongs with the item catalogue and the
/// supplier defaults; until one exists, a purchase lands here and a period close
/// refuses to happen while this account has a balance.
pub const UNCLASSIFIED_EXPENSE: &str = crate::ledger::UNCLASSIFIED_ACCOUNT;

/// The rate a reverse-charged purchase is self-assessed at.
///
/// The supplier charges nothing, so the VAT we owe and the VAT we may deduct are
/// both computed here, at the domestic rate for the same supply. It is a rule-set
/// value, not a guess: a business with a different rate policy needs its own
/// profile, and the posting records which profile produced it.
pub const SELF_ASSESSED_RATE_BP: u16 = 2100;

/// The revenue account a VAT treatment is posted to.
///
/// One account per treatment, because the VAT return and the P&L both have to
/// explain themselves: a single "revenue" account would make a zero-rated export
/// and a domestic sale indistinguishable after the fact.
pub fn revenue_account(category: VatCategory, rate: VatRate) -> Result<AccountCode, StoreError> {
    let code = match category {
        VatCategory::Standard if rate.basis_points() == 900 => "8010",
        VatCategory::Standard => "8000",
        VatCategory::ReverseCharge => "8020",
        VatCategory::IntraCommunity => "8030",
        VatCategory::Export => "8040",
        VatCategory::ZeroRated
        | VatCategory::Exempt
        | VatCategory::OutsideScope
        | VatCategory::CanaryIslands
        | VatCategory::CeutaMelilla => "8050",
    };
    account(code)
}

/// The VAT payable account for a treatment, or `None` when the document carries
/// no VAT for it.
pub fn output_vat_account(category: VatCategory, rate: VatRate) -> Result<Option<AccountCode>, StoreError> {
    if category != VatCategory::Standard {
        return Ok(None);
    }
    let code = match rate.basis_points() {
        2100 => VAT_PAYABLE_HIGH,
        900 => VAT_PAYABLE_LOW,
        _ => VAT_PAYABLE_OTHER,
    };
    account(code).map(Some)
}

/// Posts an issued invoice or credit note.
///
/// The document must exist and be outbound. Posting is idempotent: a second call
/// for the same document returns the entry the first one wrote, so a user who
/// presses the button twice does not double the books.
pub fn post_outbound_document(
    conn: &Connection,
    document_id: DocumentId,
    actor: &str,
    now: &str,
) -> Result<PostedEntry, StoreError> {
    let document = load_document(conn, document_id)?;
    if document.direction != "outbound" {
        return Err(StoreError::Rejected(format!(
            "document {} is {} and cannot be posted as a sale",
            document.number, document.direction
        )));
    }

    let entry = outbound_entry_for(&document)?;
    ledger::post_entry(
        conn,
        PostingRequest {
            entry: &entry,
            seller_id: document.seller_id,
            entry_date: document.issue_date,
            source: PostingSource::OutboundDocument(document_id),
            actor,
            now,
            posting_profile_id: PROFILE_ID,
            rule_version: RULE_VERSION,
            reverses: None,
            open_item: open_item_for(&document, &entry),
        },
    )
}

/// An outbound document that has no journal entry yet.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PendingPosting {
    /// Document identity.
    pub id: DocumentId,
    /// Its number, as printed to the customer or written by the supplier.
    pub number: String,
    /// The date the posting will carry.
    pub issue_date: LocalDate,
    /// `outbound` for one we issued, `inbound` for one we received.
    pub direction: String,
}

/// Every document of these books that has not been posted.
///
/// The left join is on the source key the posting uses, so a document cannot
/// look unposted while an entry exists for it, whatever route wrote that entry.
pub fn pending(conn: &Connection, seller_id: SellerId) -> Result<Vec<PendingPosting>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT d.id, d.number, d.issue_date_local, d.direction
         FROM document d
         LEFT JOIN journal_entry e
                ON e.source_id = d.id
               AND e.source_kind = CASE d.direction
                                       WHEN 'outbound' THEN 'outbound_document'
                                       ELSE 'inbound_document'
                                   END
         WHERE d.owner_id = ?1 AND e.id IS NULL
         ORDER BY d.issue_date_local, d.number",
    )?;
    let mut rows = statement.query([seller_id.to_string()])?;
    let mut pending = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let issue_date: String = row.get(2)?;
        pending.push(PendingPosting {
            id: DocumentId::parse_str(&id).map_err(|error| StoreError::Data(error.to_string()))?,
            number: row.get(1)?,
            issue_date: LocalDate::parse_iso(&issue_date).map_err(|error| StoreError::Data(error.to_string()))?,
            direction: row.get(3)?,
        });
    }
    Ok(pending)
}

/// Posts a document, choosing the rule set that matches its direction.
pub fn post_document(
    conn: &Connection,
    document_id: DocumentId,
    actor: &str,
    now: &str,
) -> Result<PostedEntry, StoreError> {
    match load_document(conn, document_id)?.direction.as_str() {
        "outbound" => post_outbound_document(conn, document_id, actor, now),
        "inbound" => post_inbound_document(conn, document_id, actor, now),
        other => Err(StoreError::Data(format!(
            "document {document_id} has direction {other:?}"
        ))),
    }
}

/// Posts an invoice we received.
///
/// A purchase is the mirror of a sale, with one difference that matters: on a
/// reverse-charged supply the supplier charges no VAT, so the VAT we owe and the
/// VAT we may deduct are both computed here from the taxable base at the
/// self-assessed rate. Both sides of that assessment are posted, because a
/// deduction without the matching liability would understate what is due.
pub fn post_inbound_document(
    conn: &Connection,
    document_id: DocumentId,
    actor: &str,
    now: &str,
) -> Result<PostedEntry, StoreError> {
    let document = load_document(conn, document_id)?;
    if document.direction != "inbound" {
        return Err(StoreError::Rejected(format!(
            "document {} is {} and cannot be posted as a purchase",
            document.number, document.direction
        )));
    }

    let entry = inbound_entry_for(&document)?;
    ledger::post_entry(
        conn,
        PostingRequest {
            entry: &entry,
            seller_id: document.seller_id,
            entry_date: document.issue_date,
            source: PostingSource::InboundDocument(document_id),
            actor,
            now,
            posting_profile_id: PROFILE_ID,
            rule_version: RULE_VERSION,
            reverses: None,
            open_item: open_item_for(&document, &entry),
        },
    )
}

/// The subledger item a stored document creates, if its counterparty is known.
///
/// Posting uses this through [`open_item_for`]; it is public so a repair can
/// write exactly what the posting would have written instead of a second
/// opinion that could disagree with it.
pub fn open_item_for_document(
    conn: &Connection,
    document_id: DocumentId,
) -> Result<Option<crate::open_items::NewOpenItem>, StoreError> {
    let document = load_document(conn, document_id)?;
    // A repair must write what the posting wrote, not what today's rate cache
    // would write. When the entry exists, its control line is the carrying value
    // and its rate is the one the books hold; a document still unposted is
    // rebuilt through the same rules the posting would run.
    let entry = match crate::ledger::entry_for_document(conn, document_id)? {
        Some(id) => match crate::ledger::load_entry(conn, id)? {
            Some(stored) => stored.entry,
            None => entry_for(&document)?,
        },
        None => entry_for(&document)?,
    };
    Ok(open_item_for(&document, &entry))
}

/// The entry a stored document posts, without writing it.
fn entry_for(document: &DocumentFacts) -> Result<LedgerEntry, StoreError> {
    match document.direction.as_str() {
        "outbound" => outbound_entry_for(document),
        "inbound" => inbound_entry_for(document),
        other => Err(StoreError::Data(format!(
            "document {} has direction {other:?}",
            document.number
        ))),
    }
}

/// The subledger entry a document creates, if its counterparty is known.
///
/// A credit note is negative: it reduces what is owed rather than creating a
/// second receivable, which is what keeps the control account equal to the sum of
/// the subledger across a correction.
fn open_item_for(document: &DocumentFacts, entry: &LedgerEntry) -> Option<crate::open_items::NewOpenItem> {
    let party_id = document.counterparty_id?;
    let gross = money(&document.totals, "gross", document.currency).ok()?;
    let signed = if document.is_credit_note() {
        gross.checked_neg().ok()?
    } else {
        gross
    };
    let kind = if document.direction == "outbound" {
        crate::open_items::OpenItemKind::Receivable
    } else {
        crate::open_items::OpenItemKind::Payable
    };
    // The control line is the subledger balance. Its reporting-currency amount is
    // the carrying value, and the one the trial balance shows, so the item and the
    // account cannot disagree by a micro-unit of conversion rounding.
    let control = entry
        .lines()
        .iter()
        .find(|line| line.account().as_str() == kind.control_account())?;
    let base = control.base_amount().unwrap_or_else(|| control.amount());
    let base_signed = if document.is_credit_note() {
        base.checked_neg().ok()?
    } else {
        base
    };
    Some(crate::open_items::NewOpenItem {
        kind,
        party_id,
        amount: signed,
        base_amount: base_signed,
        fx_rate_micros: entry.fx().map(|fx| fx.rate.per_unit_micros()),
        due_date: document.due_date,
        original_date: document.issue_date,
    })
}

/// Converts every line into the reporting currency.
///
/// Conversion rounds each line once. The control line takes the residual so the
/// two sides still balance in the reporting currency and the subledger's carrying
/// value is exactly what the trial balance shows; without that, a document could
/// round a micro-unit into a trial balance that no longer nets to zero.
fn converted_lines(
    document: &DocumentFacts,
    lines: Vec<LedgerLine>,
    control: AccountCode,
) -> Result<Vec<LedgerLine>, StoreError> {
    let Some(rate) = document.fx else {
        rate_required(document)?;
        return Ok(lines);
    };
    let base = document.base_currency;
    let mut bases = Vec::with_capacity(lines.len());
    for line in &lines {
        bases.push(rate.convert(line.amount(), base).map_err(domain)?);
    }
    let mut other = Money::zero(base);
    for (line, amount) in lines.iter().zip(&bases) {
        if *line.account() != control {
            other = other.checked_add(*amount).map_err(domain)?;
        }
    }
    let mut converted = Vec::with_capacity(lines.len());
    for (line, amount) in lines.iter().zip(&bases) {
        let base_amount = if *line.account() == control { other } else { *amount };
        converted.push(line_with_base(line, base_amount)?);
    }
    Ok(converted)
}

/// The same line with a reporting-currency amount, tags included.
fn line_with_base(line: &LedgerLine, base: Money) -> Result<LedgerLine, StoreError> {
    let mut rebuilt = LedgerLine::new(line.account().clone(), line.side(), line.amount()).map_err(domain)?;
    if let Some(vat) = line.vat() {
        let rate = VatRate::from_basis_points(vat.rate_bp).map_err(domain)?;
        rebuilt = rebuilt.tagged(vat.category, rate);
    }
    if let Some(code) = line.tax_code() {
        rebuilt = rebuilt.coded(code.clone());
    }
    rebuilt.converted(base).map_err(domain)
}

/// Builds the entry, converted when the document is not in the reporting currency.
fn entry_with_conversion(
    document: &DocumentFacts,
    journal: Journal,
    description: String,
    lines: Vec<LedgerLine>,
) -> Result<LedgerEntry, StoreError> {
    match document.fx {
        Some(rate) => LedgerEntry::converted(journal, description, lines, document.base_currency, rate).map_err(domain),
        None => {
            rate_required(document)?;
            LedgerEntry::new(journal, description, lines).map_err(domain)
        }
    }
}

/// The rate a foreign document needs, with the commands that provide one.
fn rate_required(document: &DocumentFacts) -> Result<Option<ExchangeRate>, StoreError> {
    if document.currency == document.base_currency {
        return Ok(None);
    }
    if let Some(rate) = document.fx {
        return Ok(Some(rate));
    }
    Err(StoreError::Rejected(format!(
        "document {} is in {} and the books report in {}, but there is no rate on or before {}; import a rate file (`bonk ledger fx import <file>`) or choose one for this document (`bonk ledger fx override {} --rate <{} per {}>`)",
        document.number,
        document.currency,
        document.base_currency,
        document.issue_date,
        document.number,
        document.base_currency,
        document.currency
    )))
}

/// The entry an outbound document would produce, without writing it.
///
/// Separate from [`post_outbound_document`] so a caller can show a preview of
/// what is about to be posted, which is the same arithmetic a reader of the
/// journal will see afterwards.
pub fn preview_outbound_entry(conn: &Connection, document_id: DocumentId) -> Result<LedgerEntry, StoreError> {
    outbound_entry_for(&load_document(conn, document_id)?)
}

fn outbound_entry_for(document: &DocumentFacts) -> Result<LedgerEntry, StoreError> {
    let currency = document.currency;
    let gross = money(&document.totals, "gross", currency)?;
    if !gross.is_positive() {
        return Err(StoreError::Rejected(format!(
            "document {} has no amount to post",
            document.number
        )));
    }
    let breakdown = document
        .totals
        .get("breakdown")
        .and_then(Value::as_array)
        .ok_or_else(|| StoreError::Data(format!("document {} has no tax breakdown", document.number)))?;

    let mut lines = vec![LedgerLine::debit(account(DEBTORS)?, gross).map_err(domain)?];
    let mut credited = false;
    for row in breakdown {
        let category = VatCategory::parse(&text(row, "category")?).map_err(domain)?;
        let rate = VatRate::from_basis_points(
            u16::try_from(integer(row, "rate_bp")?)
                .map_err(|_| StoreError::Data(format!("document {} has an out-of-range VAT rate", document.number)))?,
        )
        .map_err(domain)?;
        let base = money(row, "taxable_base", currency)?;
        let tax = money(row, "tax", currency)?;

        if !base.is_zero() {
            lines.push(
                LedgerLine::credit(revenue_account(category, rate)?, base)
                    .map_err(domain)?
                    .tagged(category, rate),
            );
            credited = true;
        }
        if tax.is_positive() {
            let vat = output_vat_account(category, rate)?.ok_or_else(|| {
                StoreError::Rejected(format!(
                    "category {} carries VAT but the rule set has no VAT account for it",
                    category.code()
                ))
            })?;
            lines.push(LedgerLine::credit(vat, tax).map_err(domain)?.tagged(category, rate));
        }
    }

    if !credited {
        return Err(StoreError::Rejected(format!(
            "document {} has no taxable amount to post",
            document.number
        )));
    }

    let description = if document.is_credit_note() {
        format!("Creditfactuur {}", document.number)
    } else {
        format!("Factuur {}", document.number)
    };
    let lines = converted_lines(document, lines, account(DEBTORS)?)?;
    let entry = entry_with_conversion(document, Journal::Sales, description.clone(), lines)?;
    if document.is_credit_note() {
        // A credit note is the invoice's mirror, so the correction reads the same
        // way the invoice did, with every side reversed.
        entry.reversed(description).map_err(domain)
    } else {
        Ok(entry)
    }
}

fn inbound_entry_for(document: &DocumentFacts) -> Result<LedgerEntry, StoreError> {
    let currency = document.currency;
    let gross = money(&document.totals, "gross", currency)?;
    if !gross.is_positive() {
        return Err(StoreError::Rejected(format!(
            "document {} has no amount to post",
            document.number
        )));
    }
    let breakdown = document
        .totals
        .get("breakdown")
        .and_then(Value::as_array)
        .ok_or_else(|| StoreError::Data(format!("document {} has no tax breakdown", document.number)))?;

    let mut lines = Vec::new();
    let mut classified = false;
    // Where the cost goes: the account a person chose, or the one nobody has
    // classified yet. A period cannot be closed while the latter holds anything,
    // which is what makes the choice part of the work rather than an extra.
    let cost = match &document.expense_account {
        Some(chosen) => chosen.clone(),
        None => account(UNCLASSIFIED_EXPENSE)?,
    };
    for row in breakdown {
        let category = VatCategory::parse(&text(row, "category")?).map_err(domain)?;
        let base = money(row, "taxable_base", currency)?;
        let tax = money(row, "tax", currency)?;
        if base.is_zero() {
            continue;
        }
        classified = true;
        let rate = VatRate::from_basis_points(
            u16::try_from(integer(row, "rate_bp")?)
                .map_err(|_| StoreError::Data(format!("document {} has an out-of-range VAT rate", document.number)))?,
        )
        .map_err(domain)?;
        // A chosen code only applies to the rows of its own category: a purchase
        // can carry both a domestic line and a reverse-charged one.
        let code = document.tax_code.as_ref().filter(|code| code.category == category);
        lines.push(coded(
            LedgerLine::debit(cost.clone(), base)
                .map_err(domain)?
                .tagged(category, rate),
            code,
        ));

        // What may be deducted: what the supplier charged, or — when they charged
        // nothing because the VAT shifted to us — what we owe ourselves.
        let assessed_rate = VatRate::from_basis_points(SELF_ASSESSED_RATE_BP).map_err(domain)?;
        let (deductible, assessed) = if tax.is_positive() {
            (tax, None)
        } else if matches!(category, VatCategory::ReverseCharge | VatCategory::IntraCommunity) {
            let amount = crux::tax_amount(base, assessed_rate).map_err(domain)?;
            (amount, Some(amount))
        } else {
            (crux::Money::zero(currency), None)
        };

        if deductible.is_positive() {
            lines.push(coded(
                LedgerLine::debit(account(VAT_RECEIVABLE)?, deductible)
                    .map_err(domain)?
                    .tagged(category, rate),
                code,
            ));
        }
        if let Some(amount) = assessed.filter(|amount| amount.is_positive()) {
            // The liability we assessed on the supplier's behalf is tagged with the
            // treatment and the rate we used, which is what puts it in the same box
            // of the return as the deduction it belongs with.
            lines.push(coded(
                LedgerLine::credit(account(VAT_PAYABLE_OTHER)?, amount)
                    .map_err(domain)?
                    .tagged(category, assessed_rate),
                code,
            ));
        }
    }

    if !classified {
        return Err(StoreError::Rejected(format!(
            "document {} has no taxable amount to post",
            document.number
        )));
    }

    // What we owe the supplier.
    lines.push(LedgerLine::credit(account(CREDITORS)?, gross).map_err(domain)?);

    let description = if document.is_credit_note() {
        format!("Inkoopcreditfactuur {}", document.number)
    } else {
        format!("Inkoopfactuur {}", document.number)
    };
    let lines = converted_lines(document, lines, account(CREDITORS)?)?;
    let entry = entry_with_conversion(document, Journal::Purchase, description.clone(), lines)?;
    if document.is_credit_note() {
        entry.reversed(description).map_err(domain)
    } else {
        Ok(entry)
    }
}

/// Refuses a document that is not a received, unposted one.
///
/// `what` names the decision being made, so the sentence reads "its account is
/// fixed" or "its treatment is fixed". Shared by the two decisions a purchase
/// carries, because the rule is the same for both: the posting snapshots them,
/// entries are append-only, so the moment to decide is before the posting. Returns
/// the stored tax breakdown, which the decision may need to look at.
pub(crate) fn unposted_inbound(conn: &Connection, document_id: DocumentId, what: &str) -> Result<String, StoreError> {
    let (direction, breakdown): (String, String) = conn
        .query_row(
            "SELECT direction, tax_breakdown_json FROM document WHERE id = ?1",
            [document_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("document {document_id}")))?;
    if direction != "inbound" {
        return Err(StoreError::Rejected(format!(
            "a document we issued has no {what} to choose; its category and rate decide everything"
        )));
    }
    let posted: Option<String> = conn
        .query_row(
            "SELECT id FROM journal_entry WHERE source_kind = 'inbound_document' AND source_id = ?1",
            [document_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    if posted.is_some() {
        return Err(StoreError::Rejected(format!(
            "document {document_id} is already posted; its {what} is fixed in the entry that was written"
        )));
    }
    Ok(breakdown)
}

/// The stored facts of a document that posting needs.
struct DocumentFacts {
    direction: String,
    doc_type: String,
    number: String,
    seller_id: SellerId,
    counterparty_id: Option<crux::CustomerId>,
    currency: Currency,
    /// The books' reporting currency.
    base_currency: Currency,
    /// The rate to that currency, null when the document is already in it.
    fx: Option<ExchangeRate>,
    issue_date: LocalDate,
    due_date: Option<LocalDate>,
    totals: Value,
    /// The treatment a person chose for the purchase, when one was chosen.
    tax_code: Option<crate::tax_code::TaxCodeRecord>,
    /// The account a person chose for its cost, when one was chosen.
    expense_account: Option<AccountCode>,
}

impl DocumentFacts {
    fn is_credit_note(&self) -> bool {
        self.doc_type == "381"
    }
}

fn load_document(conn: &Connection, id: DocumentId) -> Result<DocumentFacts, StoreError> {
    let mut statement = conn.prepare(
        "SELECT direction, doc_type, number, owner_id, counterparty_id, currency,
                issue_date_local, due_date_local, totals_json
         FROM document WHERE id = ?1",
    )?;
    let mut rows = statement.query([id.to_string()])?;
    let Some(row) = rows.next()? else {
        return Err(StoreError::NotFound(format!("document {id}")));
    };
    let counterparty: Option<String> = row.get(4)?;
    let due_date: Option<String> = row.get(7)?;
    let seller_id = SellerId::parse_str(&row.get::<_, String>(3)?).map_err(domain)?;
    let currency = Currency::parse(&row.get::<_, String>(5)?).map_err(domain)?;
    let issue_date = LocalDate::parse_iso(&row.get::<_, String>(6)?).map_err(domain)?;
    let number: String = row.get(2)?;
    let base_currency = crate::report::reporting_currency(conn, seller_id)?;
    let fx = crate::fx::document_rate(conn, seller_id, id, currency, issue_date)?
        .map(|(rate, _source)| rate)
        .filter(|_| currency != base_currency);
    Ok(DocumentFacts {
        direction: row.get(0)?,
        doc_type: row.get(1)?,
        number,
        seller_id,
        counterparty_id: counterparty
            .map(|value| crux::CustomerId::parse_str(&value))
            .transpose()
            .map_err(domain)?,
        currency,
        base_currency,
        fx,
        issue_date,
        due_date: due_date
            .map(|value| LocalDate::parse_iso(&value))
            .transpose()
            .map_err(domain)?,
        totals: serde_json::from_str(&row.get::<_, String>(8)?)?,
        tax_code: crate::tax_code::assigned(conn, id)?,
        expense_account: crate::expense::chosen(conn, id)?,
    })
}

fn account(code: &str) -> Result<AccountCode, StoreError> {
    AccountCode::parse(code).map_err(domain)
}

/// Stamps the chosen treatment on a line, when one applies to it.
fn coded(line: LedgerLine, code: Option<&crate::tax_code::TaxCodeRecord>) -> LedgerLine {
    match code {
        Some(code) => line.coded(code.code.clone()),
        None => line,
    }
}

fn money(value: &Value, key: &str, currency: Currency) -> Result<Money, StoreError> {
    Money::parse_fixed(&text(value, key)?, currency).map_err(domain)
}

fn text(value: &Value, key: &str) -> Result<String, StoreError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| StoreError::Data(format!("stored value has no string {key:?}")))
}

fn integer(value: &Value, key: &str) -> Result<i64, StoreError> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| StoreError::Data(format!("stored value has no integer {key:?}")))
}

fn domain(error: crux::DomainError) -> StoreError {
    StoreError::Data(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_mapping_covers_every_category() {
        // Every EN 16931 category has a revenue account, and only standard-rated
        // sales carry a VAT line.
        for category in [
            VatCategory::Standard,
            VatCategory::ZeroRated,
            VatCategory::Exempt,
            VatCategory::ReverseCharge,
            VatCategory::IntraCommunity,
            VatCategory::Export,
            VatCategory::OutsideScope,
            VatCategory::CanaryIslands,
            VatCategory::CeutaMelilla,
        ] {
            let rate = if category == VatCategory::Standard {
                VatRate::STANDARD_NL
            } else {
                VatRate::ZERO
            };
            assert!(
                revenue_account(category, rate).is_ok(),
                "{category:?} has no revenue account"
            );
            let vat = output_vat_account(category, rate).unwrap();
            assert_eq!(vat.is_some(), category == VatCategory::Standard, "{category:?}");
        }
    }

    #[test]
    fn rates_select_their_own_accounts() {
        let high = revenue_account(VatCategory::Standard, VatRate::STANDARD_NL).unwrap();
        let low = revenue_account(VatCategory::Standard, VatRate::REDUCED_NL).unwrap();
        assert_eq!(high.as_str(), "8000");
        assert_eq!(low.as_str(), "8010");
        assert_ne!(high, low);

        assert_eq!(
            output_vat_account(VatCategory::Standard, VatRate::STANDARD_NL)
                .unwrap()
                .unwrap()
                .as_str(),
            VAT_PAYABLE_HIGH
        );
        assert_eq!(
            output_vat_account(VatCategory::Standard, VatRate::REDUCED_NL)
                .unwrap()
                .unwrap()
                .as_str(),
            VAT_PAYABLE_LOW
        );
        // A rate outside the two Dutch ones is VAT payable, but not at the high
        // or low account: the return has to be able to tell.
        let other = VatRate::parse_percent("6").unwrap();
        assert_eq!(
            output_vat_account(VatCategory::Standard, other)
                .unwrap()
                .unwrap()
                .as_str(),
            "1620"
        );
    }
}
