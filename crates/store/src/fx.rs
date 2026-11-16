//! The exchange-rate cache and the per-document overrides on top of it.
//!
//! Rates are offline on purpose: a posting must not depend on a network call, and
//! two people re-opening the same month must see the same numbers. The cache is
//! filled from a file the user downloaded — the European Central Bank's
//! `eurofxref-hist.csv` is the format implemented here — and every row records
//! the hash of the file it came from, so a rate can be traced to a document.
//!
//! The ECB quotes every currency per euro. Import crosses those quotes into the
//! books' reporting currency at import time, which is why a lookup is one row and
//! not two. A manual rate and a document override sit beside the cache: the first
//! is a fact about a day, the second a decision about one document, with an
//! author and a time.
//!
//! Nothing here converts an amount. `crux::ExchangeRate` does that; this module
//! only decides which rate applies.

use crux::{Currency, DocumentId, ExchangeRate, LocalDate, Money, SellerId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::audit;
use crate::error::StoreError;

/// The largest rate file accepted, so a mistaken path cannot exhaust memory.
const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;

/// The most rows accepted from one file; the ECB history is well under this.
const MAX_ROWS: usize = 500_000;

/// The parity quote used for the euro in an ECB file, which has no EUR column.
const EUR_QUOTE: i64 = crux::MICROS_PER_UNIT;

/// Where a rate came from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RateSource {
    /// The document is already in the books' reporting currency.
    Reporting,
    /// A person chose the rate for this document.
    DocumentOverride,
    /// The cached reference rate at or before the document's date.
    Cached,
}

impl RateSource {
    /// The stable token for an audit payload.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reporting => "reporting",
            Self::DocumentOverride => "document-override",
            Self::Cached => "cache",
        }
    }
}

/// One cached rate.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CachedRate {
    /// The currency being converted from.
    pub currency: Currency,
    /// The books' reporting currency.
    pub base_currency: Currency,
    /// The date the rate applies from.
    pub as_of: LocalDate,
    /// Reporting currency per unit of `currency`.
    pub rate: ExchangeRate,
    /// `ecb`, `manual`, or `demo`.
    pub source: String,
    /// The provider that published it.
    pub provider: String,
    /// The version of the provider's file, normally its SHA-256.
    pub provider_version: String,
    /// When it was written to this archive.
    pub recorded_at: String,
    /// Who wrote it.
    pub recorded_by: String,
}

/// A person's rate for one document.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocumentRate {
    /// The document it applies to.
    pub document_id: DocumentId,
    /// The books' reporting currency when it was chosen.
    pub base_currency: Currency,
    /// Reporting currency per unit of the document's currency.
    pub rate: ExchangeRate,
    /// The date the chosen rate belongs to.
    pub as_of: LocalDate,
    /// Who chose it.
    pub decided_by: String,
    /// When they chose it.
    pub decided_at: String,
}

/// What an import wrote.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImportReport {
    /// Rows read from the file.
    pub rows: usize,
    /// Rows written (inserted or replaced).
    pub written: usize,
    /// Rows skipped because a value was missing or the date had no base quote.
    pub skipped: usize,
    /// Distinct currencies seen.
    pub currencies: usize,
    /// The hash of the file, which versions every row it wrote.
    pub provider_version: String,
}

/// The reporting currency of the books.
fn reporting_currency(conn: &Connection, seller_id: SellerId) -> Result<Currency, StoreError> {
    crate::report::reporting_currency(conn, seller_id)
}

/// The rate from `currency` into the books' reporting currency at a date.
///
/// The latest row at or before `date` is used, so a weekend or a holiday takes
/// the last published reference rate. A manual row for the same day wins over
/// the cached one, because it is the person saying the reference rate is wrong
/// for this books.
pub fn rate_on(
    conn: &Connection,
    seller_id: SellerId,
    currency: Currency,
    date: LocalDate,
) -> Result<Option<ExchangeRate>, StoreError> {
    let base = reporting_currency(conn, seller_id)?;
    if currency == base {
        return Ok(Some(ExchangeRate::ONE));
    }
    let micros: Option<i64> = conn
        .query_row(
            "SELECT rate_micros FROM fx_rate
             WHERE seller_id = ?1 AND currency = ?2 AND base_currency = ?3 AND as_of <= ?4
             ORDER BY as_of DESC, CASE source WHEN 'manual' THEN 0 ELSE 1 END
             LIMIT 1",
            params![
                seller_id.to_string(),
                currency.as_str(),
                base.as_str(),
                date.to_iso_string()
            ],
            |row| row.get(0),
        )
        .optional()?;
    micros
        .map(ExchangeRate::from_per_unit_micros)
        .transpose()
        .map_err(|error| StoreError::Data(error.to_string()))
}

/// The rate a document's posting uses, with where it came from.
///
/// The override is read first; a document is posted once, so a decision attached
/// to it is what the posting must record. A rate that is missing is not an error
/// here: the caller says what to do about it, because only the caller knows
/// whether it is posting, previewing, or rebuilding.
pub fn document_rate(
    conn: &Connection,
    seller_id: SellerId,
    document_id: DocumentId,
    currency: Currency,
    issue_date: LocalDate,
) -> Result<Option<(ExchangeRate, RateSource)>, StoreError> {
    let base = reporting_currency(conn, seller_id)?;
    if currency == base {
        return Ok(Some((ExchangeRate::ONE, RateSource::Reporting)));
    }
    if let Some(over) = override_for(conn, document_id)? {
        if over.base_currency != base {
            return Err(StoreError::Rejected(format!(
                "document {document_id} has a rate into {}, but the books report in {base}; clear the override and choose a new one",
                over.base_currency
            )));
        }
        return Ok(Some((over.rate, RateSource::DocumentOverride)));
    }
    Ok(rate_on(conn, seller_id, currency, issue_date)?.map(|rate| (rate, RateSource::Cached)))
}

/// The rate a document would post at, with the facts the answer depends on.
pub fn rate_for_document(
    conn: &Connection,
    seller_id: SellerId,
    document_id: DocumentId,
) -> Result<Option<(Currency, LocalDate, ExchangeRate, RateSource)>, StoreError> {
    let (currency, issue_date) = document_facts(conn, seller_id, document_id)?;
    Ok(document_rate(conn, seller_id, document_id, currency, issue_date)?
        .map(|(rate, source)| (currency, issue_date, rate, source)))
}

/// The override recorded for a document, if any.
pub fn override_for(conn: &Connection, document_id: DocumentId) -> Result<Option<DocumentRate>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT document_id, base_currency, rate_micros, as_of, decided_by, decided_at
         FROM document_fx_rate WHERE document_id = ?1",
    )?;
    let mut rows = statement.query([document_id.to_string()])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let raw_document: String = row.get(0)?;
    let raw_base: String = row.get(1)?;
    let raw_as_of: String = row.get(3)?;
    Ok(Some(DocumentRate {
        document_id: DocumentId::parse_str(&raw_document).map_err(|error| StoreError::Data(error.to_string()))?,
        base_currency: Currency::parse(&raw_base).map_err(|error| StoreError::Data(error.to_string()))?,
        rate: ExchangeRate::from_per_unit_micros(row.get(2)?).map_err(|error| StoreError::Data(error.to_string()))?,
        as_of: LocalDate::parse_iso(&raw_as_of).map_err(|error| StoreError::Data(error.to_string()))?,
        decided_by: row.get(4)?,
        decided_at: row.get(5)?,
    }))
}

/// Records a person's rate for one document.
///
/// Refused once the document is posted: the entry is append-only, so a new rate
/// would describe a posting that did not use it. Correcting the rate is then the
/// ordinary reversal pair.
pub fn set_override(
    conn: &Connection,
    seller_id: SellerId,
    document_id: DocumentId,
    rate: ExchangeRate,
    as_of: LocalDate,
    actor: &str,
    now: &str,
) -> Result<(), StoreError> {
    let currency = document_currency(conn, seller_id, document_id)?;
    let base = reporting_currency(conn, seller_id)?;
    if currency == base {
        return Err(StoreError::Rejected(format!(
            "document {document_id} is already in {base}; it needs no rate"
        )));
    }
    if crate::ledger::entry_for_document(conn, document_id)?.is_some() {
        return Err(StoreError::Rejected(format!(
            "document {document_id} is posted; its rate is part of the entry and cannot be changed"
        )));
    }

    let transaction = conn.unchecked_transaction()?;
    let previous: Option<i64> = transaction
        .query_row(
            "SELECT rate_micros FROM document_fx_rate WHERE document_id = ?1",
            [document_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;

    transaction.execute(
        "INSERT INTO document_fx_rate
             (document_id, base_currency, rate_micros, as_of, decided_by, decided_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(document_id) DO UPDATE SET
             base_currency = excluded.base_currency,
             rate_micros = excluded.rate_micros,
             as_of = excluded.as_of,
             decided_by = excluded.decided_by,
             decided_at = excluded.decided_at",
        params![
            document_id.to_string(),
            base.as_str(),
            rate.per_unit_micros(),
            as_of.to_iso_string(),
            actor,
            now,
        ],
    )?;
    audit::append(
        &transaction,
        now,
        actor,
        "fx_rate_overridden",
        "document",
        &document_id.to_string(),
        serde_json::json!({
            "currency": currency.as_str(),
            "base_currency": base.as_str(),
            "rate_micros": rate.per_unit_micros(),
            "as_of": as_of.to_iso_string(),
            "previous_micros": previous,
        }),
    )?;
    transaction.commit()?;
    Ok(())
}

/// Removes a document's rate override, returning it to the cached rate.
pub fn clear_override(
    conn: &Connection,
    seller_id: SellerId,
    document_id: DocumentId,
    actor: &str,
    now: &str,
) -> Result<(), StoreError> {
    let _ = document_currency(conn, seller_id, document_id)?;
    if crate::ledger::entry_for_document(conn, document_id)?.is_some() {
        return Err(StoreError::Rejected(format!(
            "document {document_id} is posted; its rate is part of the entry and cannot be cleared"
        )));
    }
    let transaction = conn.unchecked_transaction()?;
    let previous: Option<i64> = transaction
        .query_row(
            "SELECT rate_micros FROM document_fx_rate WHERE document_id = ?1",
            [document_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(previous) = previous else {
        return Err(StoreError::NotFound(format!(
            "document {document_id} has no rate override"
        )));
    };
    transaction.execute(
        "DELETE FROM document_fx_rate WHERE document_id = ?1",
        [document_id.to_string()],
    )?;
    audit::append(
        &transaction,
        now,
        actor,
        "fx_rate_cleared",
        "document",
        &document_id.to_string(),
        serde_json::json!({ "previous_micros": previous }),
    )?;
    transaction.commit()?;
    Ok(())
}

/// Stores a manual rate for a currency and date.
pub fn set_manual_rate(
    conn: &Connection,
    seller_id: SellerId,
    currency: Currency,
    as_of: LocalDate,
    rate: ExchangeRate,
    actor: &str,
    now: &str,
) -> Result<(), StoreError> {
    let base = reporting_currency(conn, seller_id)?;
    if currency == base {
        return Err(StoreError::Rejected(format!(
            "{currency} is the reporting currency; its rate is one"
        )));
    }
    let transaction = conn.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO fx_rate
             (id, seller_id, currency, base_currency, as_of, rate_micros, source, provider,
              provider_version, recorded_at, recorded_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'manual', 'manual', 'manual', ?7, ?8)
         ON CONFLICT (seller_id, currency, base_currency, as_of, source) DO UPDATE SET
             rate_micros = excluded.rate_micros,
             recorded_at = excluded.recorded_at,
             recorded_by = excluded.recorded_by",
        params![
            DocumentId::new().to_string(),
            seller_id.to_string(),
            currency.as_str(),
            base.as_str(),
            as_of.to_iso_string(),
            rate.per_unit_micros(),
            now,
            actor,
        ],
    )?;
    audit::append(
        &transaction,
        now,
        actor,
        "fx_rate_set",
        "fx_rate",
        &format!("{currency}:{as_of}"),
        serde_json::json!({
            "currency": currency.as_str(),
            "base_currency": base.as_str(),
            "rate_micros": rate.per_unit_micros(),
            "as_of": as_of.to_iso_string(),
        }),
    )?;
    transaction.commit()?;
    Ok(())
}

/// Every cached rate, newest first.
pub fn list_rates(
    conn: &Connection,
    seller_id: SellerId,
    currency: Option<Currency>,
) -> Result<Vec<CachedRate>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT currency, base_currency, as_of, rate_micros, source, provider, provider_version,
                recorded_at, recorded_by
         FROM fx_rate
         WHERE seller_id = ?1 AND (?2 IS NULL OR currency = ?2)
         ORDER BY as_of DESC, currency",
    )?;
    let currency_raw = currency.map(|value| value.as_str().to_owned());
    let mut rows = statement.query(params![seller_id.to_string(), currency_raw])?;
    let mut rates = Vec::new();
    while let Some(row) = rows.next()? {
        let raw_currency: String = row.get(0)?;
        let raw_base: String = row.get(1)?;
        let raw_as_of: String = row.get(2)?;
        rates.push(CachedRate {
            currency: Currency::parse(&raw_currency).map_err(|error| StoreError::Data(error.to_string()))?,
            base_currency: Currency::parse(&raw_base).map_err(|error| StoreError::Data(error.to_string()))?,
            as_of: LocalDate::parse_iso(&raw_as_of).map_err(|error| StoreError::Data(error.to_string()))?,
            rate: ExchangeRate::from_per_unit_micros(row.get(3)?)
                .map_err(|error| StoreError::Data(error.to_string()))?,
            source: row.get(4)?,
            provider: row.get(5)?,
            provider_version: row.get(6)?,
            recorded_at: row.get(7)?,
            recorded_by: row.get(8)?,
        });
    }
    Ok(rates)
}

/// Stores a demonstration rate, labelled as one.
///
/// The demo archive trades in dollars and pounds, so it needs rates to post; a
/// fixed, clearly-labelled pair keeps the demo deterministic without pretending
/// to be a reference published by anybody.
pub fn seed_demo_rate(
    conn: &Connection,
    seller_id: SellerId,
    currency: Currency,
    as_of: LocalDate,
    rate: ExchangeRate,
    actor: &str,
    now: &str,
) -> Result<(), StoreError> {
    let base = reporting_currency(conn, seller_id)?;
    conn.execute(
        "INSERT INTO fx_rate
             (id, seller_id, currency, base_currency, as_of, rate_micros, source, provider,
              provider_version, recorded_at, recorded_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'demo', 'demo', 'demo-seed', ?7, ?8)
         ON CONFLICT (seller_id, currency, base_currency, as_of, source) DO UPDATE SET
             rate_micros = excluded.rate_micros,
             recorded_at = excluded.recorded_at,
             recorded_by = excluded.recorded_by",
        params![
            DocumentId::new().to_string(),
            seller_id.to_string(),
            currency.as_str(),
            base.as_str(),
            as_of.to_iso_string(),
            rate.per_unit_micros(),
            now,
            actor,
        ],
    )?;
    Ok(())
}

/// Imports the European Central Bank's `eurofxref-hist.csv`.
///
/// The file is wide: a date column and one column per currency, each value the
/// units of that currency per euro. The import crosses every quote into the
/// books' reporting currency, which is why the base currency has to appear in the
/// file when it is not the euro. Rows are replaced rather than duplicated, so a
/// corrected file updates the cache instead of stacking a second opinion beside
/// it; the file's hash versions every row it writes.
pub fn import_ecb(
    conn: &Connection,
    seller_id: SellerId,
    bytes: &[u8],
    actor: &str,
    now: &str,
) -> Result<ImportReport, StoreError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(StoreError::Rejected(format!(
            "the rate file is {} bytes; the limit is {MAX_FILE_BYTES}",
            bytes.len()
        )));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| StoreError::Rejected("the rate file is not UTF-8 text".to_owned()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let base = reporting_currency(conn, seller_id)?;

    let mut lines = text.lines().enumerate();
    let Some((_, header)) = lines.next() else {
        return Err(StoreError::Rejected("the rate file is empty".to_owned()));
    };
    let header = header.trim_end_matches('\r');
    let mut columns = header.split(',');
    let first = columns.next().unwrap_or_default();
    if !first.eq_ignore_ascii_case("Date") {
        return Err(StoreError::Rejected(
            "the rate file has no Date column; this importer reads the ECB eurofxref-hist.csv layout".to_owned(),
        ));
    }
    let mut currencies: Vec<Option<Currency>> = Vec::new();
    let mut base_column: Option<usize> = None;
    for (index, raw) in columns.enumerate() {
        let code = raw.trim();
        if code.is_empty() {
            currencies.push(None);
            continue;
        }
        let currency = Currency::parse(code)
            .map_err(|_| StoreError::Rejected(format!("the rate file has an invalid currency column {code:?}")))?;
        if currency == base {
            base_column = Some(index);
        }
        currencies.push(Some(currency));
    }
    if base != Currency::EUR && base_column.is_none() {
        return Err(StoreError::Rejected(format!(
            "the rate file does not quote {base}, which is the reporting currency of these books"
        )));
    }
    if currencies.iter().all(Option::is_none) {
        return Err(StoreError::Rejected("the rate file has no currency columns".to_owned()));
    }

    let version = audit::sha256_hex(bytes);
    let transaction = conn.unchecked_transaction()?;
    let mut statement = transaction.prepare(
        "INSERT INTO fx_rate
             (id, seller_id, currency, base_currency, as_of, rate_micros, source, provider,
              provider_version, recorded_at, recorded_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'ecb', 'ecb', ?7, ?8, ?9)
         ON CONFLICT (seller_id, currency, base_currency, as_of, source) DO UPDATE SET
             rate_micros = excluded.rate_micros,
             provider = excluded.provider,
             provider_version = excluded.provider_version,
             recorded_at = excluded.recorded_at,
             recorded_by = excluded.recorded_by",
    )?;

    let mut report = ImportReport {
        rows: 0,
        written: 0,
        skipped: 0,
        currencies: 0,
        provider_version: version.clone(),
    };
    let mut seen: Vec<Currency> = Vec::new();
    for (index, line) in lines {
        if index > MAX_ROWS {
            return Err(StoreError::Rejected(format!(
                "the rate file has more than {MAX_ROWS} rows"
            )));
        }
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        report.rows += 1;
        let fields: Vec<&str> = line.split(',').collect();
        let Some(raw_date) = fields.first() else {
            continue;
        };
        let date = LocalDate::parse_iso(raw_date.trim())
            .map_err(|_| StoreError::Rejected(format!("rate file line {}: {:?} is not a date", index + 1, raw_date)))?;

        let quote = |column: usize| -> Result<Option<i64>, StoreError> {
            let Some(raw) = fields.get(column + 1) else {
                return Ok(None);
            };
            let raw = raw.trim();
            if raw.is_empty() || raw.eq_ignore_ascii_case("N/A") {
                return Ok(None);
            }
            Money::parse_fixed(raw, Currency::EUR)
                .map(|value| Some(value.micros()))
                .map_err(|_| StoreError::Rejected(format!("rate file line {}: {:?} is not a rate", index + 1, raw)))
        };
        let base_quote = match base_column {
            Some(column) => match quote(column)? {
                Some(value) if value > 0 => value,
                _ => {
                    report.skipped += 1;
                    continue;
                }
            },
            None => EUR_QUOTE,
        };

        // An ECB file has no euro column because the euro is its unit. A books
        // reporting outside the euro still needs the euro leg, and its quote is
        // the base currency's own quote per euro.
        if base != Currency::EUR {
            statement.execute(params![
                DocumentId::new().to_string(),
                seller_id.to_string(),
                Currency::EUR.as_str(),
                base.as_str(),
                date.to_iso_string(),
                base_quote,
                version,
                now,
                actor,
            ])?;
            report.written += 1;
            if !seen.contains(&Currency::EUR) {
                seen.push(Currency::EUR);
            }
        }

        for (column, currency) in currencies.iter().enumerate() {
            let Some(currency) = currency else {
                continue;
            };
            if *currency == base {
                continue;
            }
            let Some(quote_currency) = quote(column)? else {
                report.skipped += 1;
                continue;
            };
            if quote_currency <= 0 {
                report.skipped += 1;
                continue;
            }
            // Reporting per unit of `currency`: both quotes are per euro, so the
            // cross is base/euro divided by currency/euro. `mul_div_round` is the
            // ledger's one rounding rule, and it never touches a float.
            let rate_micros = Money::from_micros(base_quote, Currency::EUR)
                .mul_div_round(i128::from(crux::MICROS_PER_UNIT), i128::from(quote_currency))
                .map_err(|error| StoreError::Rejected(error.to_string()))?
                .micros();
            if rate_micros <= 0 {
                report.skipped += 1;
                continue;
            }
            statement.execute(params![
                DocumentId::new().to_string(),
                seller_id.to_string(),
                currency.as_str(),
                base.as_str(),
                date.to_iso_string(),
                rate_micros,
                version,
                now,
                actor,
            ])?;
            report.written += 1;
            if !seen.contains(currency) {
                seen.push(*currency);
            }
        }
    }
    drop(statement);
    report.currencies = seen.len();

    audit::append(
        &transaction,
        now,
        actor,
        "fx_rates_imported",
        "seller",
        &seller_id.to_string(),
        serde_json::json!({
            "provider": "ecb",
            "provider_version": version,
            "rows": report.rows,
            "written": report.written,
            "skipped": report.skipped,
            "currencies": report.currencies,
        }),
    )?;
    transaction.commit()?;
    Ok(report)
}

/// The currency and issue date of a document, after checking it is ours.
pub fn document_facts(
    conn: &Connection,
    seller_id: SellerId,
    document_id: DocumentId,
) -> Result<(Currency, LocalDate), StoreError> {
    let (raw, date): (String, String) = conn
        .query_row(
            "SELECT currency, issue_date_local FROM document WHERE id = ?1 AND owner_id = ?2",
            params![document_id.to_string(), seller_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| StoreError::NotFound(format!("document {document_id}")))?;
    Ok((
        Currency::parse(&raw).map_err(|error| StoreError::Data(error.to_string()))?,
        LocalDate::parse_iso(&date).map_err(|error| StoreError::Data(error.to_string()))?,
    ))
}

/// The currency a document is denominated in, after checking it is ours.
fn document_currency(conn: &Connection, seller_id: SellerId, document_id: DocumentId) -> Result<Currency, StoreError> {
    Ok(document_facts(conn, seller_id, document_id)?.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_source_has_a_stable_token() {
        assert_eq!(RateSource::Reporting.as_str(), "reporting");
        assert_eq!(RateSource::DocumentOverride.as_str(), "document-override");
        assert_eq!(RateSource::Cached.as_str(), "cache");
    }
}
