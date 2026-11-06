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
    AccountCode, Currency, DocumentId, Journal, LedgerEntry, LedgerLine, LocalDate, Money, SellerId, VatCategory,
    VatRate,
};
use rusqlite::Connection;
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
        },
    )
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
            lines.push(LedgerLine::credit(revenue_account(category, rate)?, base).map_err(domain)?);
            credited = true;
        }
        if tax.is_positive() {
            let vat = output_vat_account(category, rate)?.ok_or_else(|| {
                StoreError::Rejected(format!(
                    "category {} carries VAT but the rule set has no VAT account for it",
                    category.code()
                ))
            })?;
            lines.push(LedgerLine::credit(vat, tax).map_err(domain)?);
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
    let entry = LedgerEntry::new(Journal::Sales, description.clone(), lines).map_err(domain)?;
    if document.is_credit_note() {
        // A credit note is the invoice's mirror, so the correction reads the same
        // way the invoice did, with every side reversed.
        entry.reversed(description).map_err(domain)
    } else {
        Ok(entry)
    }
}

/// The stored facts of a document that posting needs.
struct DocumentFacts {
    direction: String,
    doc_type: String,
    number: String,
    seller_id: SellerId,
    currency: Currency,
    issue_date: LocalDate,
    totals: Value,
}

impl DocumentFacts {
    fn is_credit_note(&self) -> bool {
        self.doc_type == "381"
    }
}

fn load_document(conn: &Connection, id: DocumentId) -> Result<DocumentFacts, StoreError> {
    let mut statement = conn.prepare(
        "SELECT direction, doc_type, number, owner_id, currency, issue_date_local, totals_json
         FROM document WHERE id = ?1",
    )?;
    let mut rows = statement.query([id.to_string()])?;
    let Some(row) = rows.next()? else {
        return Err(StoreError::NotFound(format!("document {id}")));
    };
    Ok(DocumentFacts {
        direction: row.get(0)?,
        doc_type: row.get(1)?,
        number: row.get(2)?,
        seller_id: SellerId::parse_str(&row.get::<_, String>(3)?).map_err(domain)?,
        currency: Currency::parse(&row.get::<_, String>(4)?).map_err(domain)?,
        issue_date: LocalDate::parse_iso(&row.get::<_, String>(5)?).map_err(domain)?,
        totals: serde_json::from_str(&row.get::<_, String>(6)?)?,
    })
}

fn account(code: &str) -> Result<AccountCode, StoreError> {
    AccountCode::parse(code).map_err(domain)
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
