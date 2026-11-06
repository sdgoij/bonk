//! CSV bank statements, read through a stated profile.
//!
//! A CSV export has no schema and no magic bytes: the delimiter, the column
//! order, the decimal comma, the date layout, and the encoding are whatever that
//! bank decided. Guessing any of them is how `1.234` becomes one euro and change,
//! so nothing here guesses. A [`CsvProfile`] says what the columns mean, and the
//! reader refuses a file it cannot read through one — with the line number,
//! because "the file is wrong" is not something a person can act on.
//!
//! The profile is written as TOML, because it is edited by hand:
//!
//! ```toml
//! # A semicolon-separated export with a decimal comma.
//! delimiter = ";"
//! decimal_separator = ","
//! date_format = "%d-%m-%Y"
//!
//! [columns]
//! date = "Datum"
//! amount = "Bedrag"
//! mark = "Af Bij"
//! counterparty = "Naam tegenpartij"
//! remittance = "Omschrijving"
//! ```
//!
//! Two limitations are stated rather than papered over. A CSV states no opening
//! or closing balance in any standard arrangement, so [`read_csv`] leaves both
//! empty and the import's "does the statement add up" check cannot run for this
//! format; the payload hash and the per-payment fingerprint still make a re-import
//! a no-op. And a profile maps columns, not meanings: a bank that packs structured
//! subfields into one cell produces a remittance that is stored as it arrived,
//! exactly as the MT940 reader does with `:86:`.

use crux::{Currency, LocalDate, Money};
use serde::Deserialize;
use store::bank::{ParsedStatement, ParsedTransaction};

use crate::bank_file::{BankError, MAX_DOCUMENT_BYTES};

/// Why a profile could not be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    /// The text is not TOML, or not this shape of TOML.
    #[error("the profile is not readable: {0}")]
    Toml(String),
    /// The text is TOML but says something the reader cannot use.
    #[error("{0}")]
    Invalid(String),
}

/// How to read one bank's CSV export.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CsvProfile {
    /// Field separator: one character. Default `,`.
    #[serde(default = "default_delimiter")]
    pub delimiter: String,
    /// `utf-8` (default), `windows-1252`, or `latin1`.
    #[serde(default = "default_encoding")]
    pub encoding: String,
    /// Layout of the date column, e.g. `%d-%m-%Y`.
    #[serde(default = "default_date_format")]
    pub date_format: String,
    /// The character that separates the fraction: `.` (default) or `,`.
    #[serde(default = "default_decimal")]
    pub decimal_separator: String,
    /// The character grouping thousands, if the export has one.
    #[serde(default)]
    pub thousands_separator: Option<String>,
    /// Lines before the header, for exports that open with a preamble.
    #[serde(default)]
    pub skip_lines: u32,
    /// Whether the first line after the preamble names the columns.
    #[serde(default = "default_yes")]
    pub has_header: bool,
    /// The currency when the export has no currency column.
    #[serde(default = "default_currency")]
    pub currency: String,
    /// What each column holds.
    pub columns: Columns,
}

/// The column layout of an export.
///
/// A column is either a header name or a 1-based position. The position is what an
/// export without a header needs; the name is what survives a bank inserting a
/// column in the middle.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Columns {
    /// The booking date. Required.
    pub date: Column,
    /// The value date, when the export carries one.
    #[serde(default)]
    pub value_date: Option<Column>,
    /// A signed amount, or an unsigned one together with `mark`.
    #[serde(default)]
    pub amount: Option<Column>,
    /// Money out, when the export has two amount columns.
    #[serde(default)]
    pub debit: Option<Column>,
    /// Money in, when the export has two amount columns.
    #[serde(default)]
    pub credit: Option<Column>,
    /// A column that says which way the money went, e.g. `Af` / `Bij`.
    #[serde(default)]
    pub mark: Option<Column>,
    /// The currency, when the export names one per line.
    #[serde(default)]
    pub currency: Option<Column>,
    /// Who paid or was paid.
    #[serde(default)]
    pub counterparty: Option<Column>,
    /// Their account.
    #[serde(default)]
    pub counterparty_iban: Option<Column>,
    /// Free text, which is where an invoice number lives.
    #[serde(default)]
    pub remittance: Option<Column>,
    /// The bank's own reference.
    #[serde(default)]
    pub external_ref: Option<Column>,
}

/// Where a column is.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Column {
    /// A header name, matched without regard to case.
    Name(String),
    /// A 1-based position.
    Index(u32),
}

/// Values in the mark column that mean money left the account.
const DEBIT_MARKS: [&str; 4] = ["af", "debit", "d", "dr"];
/// Values in the mark column that mean money arrived.
const CREDIT_MARKS: [&str; 4] = ["bij", "credit", "c", "cr"];

fn default_delimiter() -> String {
    ",".to_owned()
}

fn default_encoding() -> String {
    "utf-8".to_owned()
}

fn default_date_format() -> String {
    "%Y-%m-%d".to_owned()
}

fn default_decimal() -> String {
    ".".to_owned()
}

fn default_currency() -> String {
    "EUR".to_owned()
}

const fn default_yes() -> bool {
    true
}

impl CsvProfile {
    /// Parses and checks a profile.
    ///
    /// Everything the reader would otherwise have to be defensive about is settled
    /// here: the separators are single characters and different from each other,
    /// the date layout is one we can read, and the amount columns make sense
    /// together. A profile that gets past this cannot surprise the reader.
    pub fn parse(text: &str) -> Result<Self, ProfileError> {
        let profile: Self = toml::from_str(text).map_err(|error| ProfileError::Toml(error.message().to_owned()))?;
        profile.check()?;
        Ok(profile)
    }

    fn check(&self) -> Result<(), ProfileError> {
        let delimiter = single_character(&self.delimiter, "delimiter")?;
        if delimiter == '"' || delimiter == '\n' || delimiter == '\r' {
            return Err(ProfileError::Invalid(format!(
                "delimiter {:?} cannot separate fields",
                self.delimiter
            )));
        }

        match self.encoding.to_ascii_lowercase().as_str() {
            "utf-8" | "utf8" | "windows-1252" | "cp1252" | "latin1" | "iso-8859-1" => {}
            other => {
                return Err(ProfileError::Invalid(format!(
                    "encoding {other:?} is not one of utf-8, windows-1252, latin1"
                )));
            }
        }

        DateLayout::parse(&self.date_format).map_err(ProfileError::Invalid)?;

        let decimal = single_character(&self.decimal_separator, "decimal_separator")?;
        let thousands = match &self.thousands_separator {
            Some(text) => Some(single_character(text, "thousands_separator")?),
            None => None,
        };
        if thousands == Some(decimal) {
            return Err(ProfileError::Invalid(
                "the decimal and thousands separators are the same character".to_owned(),
            ));
        }

        Currency::parse(&self.currency)
            .map_err(|_| ProfileError::Invalid(format!("currency {:?} is not a code", self.currency)))?;

        match (&self.columns.amount, &self.columns.debit, &self.columns.credit) {
            (Some(_), None, None) | (None, Some(_), Some(_)) => {}
            (None, None, None) => {
                return Err(ProfileError::Invalid(
                    "columns.amount, or columns.debit with columns.credit, is required".to_owned(),
                ));
            }
            _ => {
                return Err(ProfileError::Invalid(
                    "use columns.amount, or columns.debit with columns.credit, not both".to_owned(),
                ));
            }
        }
        if self.columns.mark.is_some() && self.columns.amount.is_none() {
            return Err(ProfileError::Invalid(
                "columns.mark says which way an unsigned columns.amount went; without one it says nothing".to_owned(),
            ));
        }

        for (key, column) in self.named_columns() {
            match column {
                Column::Index(0) => {
                    return Err(ProfileError::Invalid(format!("columns.{key} is 1-based, not 0-based")));
                }
                Column::Name(name) if name.trim().is_empty() => {
                    return Err(ProfileError::Invalid(format!("columns.{key} is an empty name")));
                }
                Column::Name(_) if !self.has_header => {
                    return Err(ProfileError::Invalid(format!(
                        "columns.{key} is a header name, but has_header is false; use a position"
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Every configured column, with the key it was written under.
    fn named_columns(&self) -> Vec<(&'static str, &Column)> {
        let mut columns: Vec<(&'static str, &Column)> = vec![("date", &self.columns.date)];
        let optional = [
            ("value_date", &self.columns.value_date),
            ("amount", &self.columns.amount),
            ("debit", &self.columns.debit),
            ("credit", &self.columns.credit),
            ("mark", &self.columns.mark),
            ("currency", &self.columns.currency),
            ("counterparty", &self.columns.counterparty),
            ("counterparty_iban", &self.columns.counterparty_iban),
            ("remittance", &self.columns.remittance),
            ("external_ref", &self.columns.external_ref),
        ];
        for (key, column) in optional {
            if let Some(column) = column {
                columns.push((key, column));
            }
        }
        columns
    }
}

/// One character, or a message naming the key.
fn single_character(text: &str, key: &str) -> Result<char, ProfileError> {
    let mut characters = text.chars();
    match (characters.next(), characters.next()) {
        (Some(character), None) => Ok(character),
        _ => Err(ProfileError::Invalid(format!("{key} {text:?} is not one character"))),
    }
}

/// A date layout a profile may name.
///
/// A closed list rather than a strftime implementation: a profile that names a
/// layout this reader does not know is refused when it is stored, instead of
/// silently reading every date as unparseable at import time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct DateLayout {
    order: DateOrder,
    separator: Option<char>,
    short_year: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DateOrder {
    YearMonthDay,
    DayMonthYear,
    MonthDayYear,
}

impl DateLayout {
    fn parse(format: &str) -> Result<Self, String> {
        let (order, separator, short_year) = match format {
            "%Y-%m-%d" => (DateOrder::YearMonthDay, Some('-'), false),
            "%Y/%m/%d" => (DateOrder::YearMonthDay, Some('/'), false),
            "%d-%m-%Y" => (DateOrder::DayMonthYear, Some('-'), false),
            "%d/%m/%Y" => (DateOrder::DayMonthYear, Some('/'), false),
            "%d.%m.%Y" => (DateOrder::DayMonthYear, Some('.'), false),
            "%m/%d/%Y" => (DateOrder::MonthDayYear, Some('/'), false),
            "%d-%m-%y" => (DateOrder::DayMonthYear, Some('-'), true),
            "%d/%m/%y" => (DateOrder::DayMonthYear, Some('/'), true),
            "%Y%m%d" => (DateOrder::YearMonthDay, None, false),
            "%d%m%Y" => (DateOrder::DayMonthYear, None, false),
            other => {
                return Err(format!(
                    "date_format {other:?} is not one of %Y-%m-%d, %Y/%m/%d, %d-%m-%Y, %d/%m/%Y, %d.%m.%Y, %m/%d/%Y, %d-%m-%y, %d/%m/%y, %Y%m%d, %d%m%Y"
                ));
            }
        };
        Ok(Self {
            order,
            separator,
            short_year,
        })
    }

    /// Reads one date, or nothing when the cell is not in this layout.
    fn read(self, text: &str) -> Option<LocalDate> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let (year, month, day): (i32, i32, i32) = match self.separator {
            Some(separator) => {
                let parts: Vec<&str> = text.split(separator).collect();
                if parts.len() != 3 {
                    return None;
                }
                let number = |index: usize| parts[index].parse::<i32>().ok();
                match self.order {
                    DateOrder::YearMonthDay => (number(0)?, number(1)?, number(2)?),
                    DateOrder::DayMonthYear => (number(2)?, number(1)?, number(0)?),
                    DateOrder::MonthDayYear => (number(2)?, number(0)?, number(1)?),
                }
            }
            None => {
                if text.len() != 8 {
                    return None;
                }
                let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i32>().ok();
                match self.order {
                    DateOrder::YearMonthDay => (number(0..4)?, number(4..6)?, number(6..8)?),
                    DateOrder::DayMonthYear => (number(4..8)?, number(2..4)?, number(0..2)?),
                    DateOrder::MonthDayYear => (number(4..8)?, number(0..2)?, number(2..4)?),
                }
            }
        };
        let year = if self.short_year {
            if (0..=99).contains(&year) {
                2000 + year
            } else {
                return None;
            }
        } else {
            year
        };
        let month = u8::try_from(month).ok()?;
        let day = u8::try_from(day).ok()?;
        LocalDate::from_ymd(year, month, day).ok()
    }
}

/// Reads a CSV statement through a profile.
///
/// One file is one statement, as with MT940: a CSV is one export of one account
/// over one range. The result is a list because every statement reader returns
/// one, and the import path walks it.
pub fn read_csv(bytes: &[u8], profile: &CsvProfile) -> Result<Vec<ParsedStatement>, BankError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(BankError::TooLarge);
    }
    profile
        .check()
        .map_err(|error| BankError::Missing(format!("the profile cannot be used: {error}")))?;

    let delimiter =
        single_character(&profile.delimiter, "delimiter").map_err(|error| BankError::Missing(error.to_string()))?;
    let decimal = single_character(&profile.decimal_separator, "decimal_separator")
        .map_err(|error| BankError::Missing(error.to_string()))?;
    let thousands = match &profile.thousands_separator {
        Some(text) => {
            Some(single_character(text, "thousands_separator").map_err(|error| BankError::Missing(error.to_string()))?)
        }
        None => None,
    };
    let layout = DateLayout::parse(&profile.date_format)
        .map_err(|error| BankError::Missing(format!("the profile cannot be used: {error}")))?;
    let currency = Currency::parse(&profile.currency)
        .map_err(|_| BankError::Missing(format!("the profile names no currency: {:?}", profile.currency)))?;

    let text = decode(bytes, &profile.encoding)?;
    let mut lines = text.lines().skip(profile.skip_lines as usize);
    let header: Vec<String> = if profile.has_header {
        match lines.next() {
            Some(line) => split(line, delimiter),
            None => {
                return Err(BankError::Missing("the file is empty, so it has no header".to_owned()));
            }
        }
    } else {
        Vec::new()
    };
    let first_row = profile.skip_lines as usize + usize::from(profile.has_header) + 1;
    let columns = Resolved::new(profile, &header)?;

    let mut transactions = Vec::new();
    for (offset, line) in lines.enumerate() {
        // A blank line is padding, not a row.
        if line.trim().is_empty() {
            continue;
        }
        let number = first_row + offset;
        let fields = split(line, delimiter);

        let date_text = cell(&fields, columns.date);
        let booking_date = layout.read(date_text).ok_or_else(|| {
            row_error(
                number,
                &format!("{date_text:?} is not a date in {}", profile.date_format),
            )
        })?;

        let amount_micros = match (columns.mark, columns.amount) {
            (Some(mark), Some(_)) => {
                // The mark decides the direction, so a stray minus in the amount
                // cell is not allowed to flip it.
                let marker = cell(&fields, mark);
                let debit = sign_from_mark(marker)
                    .ok_or_else(|| row_error(number, &format!("{marker:?} is not a debit or credit mark (af/bij)")))?;
                let amount = amount_of(&columns, &fields, decimal, thousands, number)?;
                if debit { -amount.abs() } else { amount.abs() }
            }
            _ => amount_of(&columns, &fields, decimal, thousands, number)?,
        };
        if amount_micros == 0 {
            return Err(row_error(number, "it moves no money"));
        }

        let line_currency = match columns.currency {
            Some(index) => {
                let text = cell(&fields, index);
                if text.is_empty() {
                    currency
                } else {
                    Currency::parse(text).map_err(|_| row_error(number, &format!("{text:?} is not a currency code")))?
                }
            }
            None => currency,
        };

        let value_date = match columns.value_date {
            Some(index) => {
                let text = cell(&fields, index);
                if text.is_empty() {
                    None
                } else {
                    Some(layout.read(text).ok_or_else(|| {
                        row_error(number, &format!("{text:?} is not a date in {}", profile.date_format))
                    })?)
                }
            }
            None => None,
        };

        transactions.push(ParsedTransaction {
            booking_date,
            value_date,
            amount_micros,
            currency: line_currency,
            counterparty_name: text_of(&fields, columns.counterparty),
            counterparty_iban: text_of(&fields, columns.counterparty_iban),
            remittance_info: text_of(&fields, columns.remittance),
            external_ref: text_of(&fields, columns.external_ref),
        });
    }

    let period_start = transactions.iter().map(|line| line.booking_date).min();
    let period_end = transactions.iter().map(|line| line.booking_date).max();

    Ok(vec![ParsedStatement {
        format: "csv",
        source_name: None,
        currency,
        period_start,
        period_end,
        // A CSV states no balances, so there is nothing to reconcile against.
        // Claiming a zero would make the import's arithmetic check pass on a file
        // it never examined.
        opening_balance_micros: None,
        closing_balance_micros: None,
        transactions,
    }])
}

/// The amount of a row, signed only when the file says so in its own cells.
fn amount_of(
    columns: &Resolved,
    fields: &[String],
    decimal: char,
    thousands: Option<char>,
    number: usize,
) -> Result<i64, BankError> {
    let read = |index: usize, key: &str| -> Result<Option<i64>, BankError> {
        let text = cell(fields, index);
        if text.is_empty() {
            return Ok(None);
        }
        amount_micros(text, decimal, thousands)
            .map(Some)
            .ok_or_else(|| row_error(number, &format!("{text:?} is not an amount in columns.{key}")))
    };

    match (columns.amount, columns.debit, columns.credit) {
        (Some(amount), _, _) => read(amount, "amount")?.ok_or_else(|| row_error(number, "it has no amount")),
        (_, Some(debit), Some(credit)) => match (read(debit, "debit")?, read(credit, "credit")?) {
            (Some(value), None) => Ok(-value.abs()),
            (None, Some(value)) => Ok(value.abs()),
            (Some(_), Some(_)) => Err(row_error(number, "it is in both the debit and the credit column")),
            (None, None) => Err(row_error(number, "it has neither a debit nor a credit amount")),
        },
        _ => Err(BankError::Missing("the profile names no amount column".to_owned())),
    }
}

/// Reads a decimal amount to micro-units, without floating point.
///
/// Accepts the configured thousands separator, either decimal character, a
/// trailing minus as some banks write it, and a leading plus. Anything else
/// returns nothing, so the caller can name the line it was on.
fn amount_micros(text: &str, decimal: char, thousands: Option<char>) -> Option<i64> {
    let mut text = text.trim().to_owned();
    if let Some(thousands) = thousands {
        text = text.replace(thousands, "");
    }
    text.retain(|character| character != ' ');
    let trailing_minus = text.ends_with('-');
    if trailing_minus {
        text.pop();
    }
    let negative = trailing_minus || text.starts_with('-');
    let text = text.trim_start_matches('-');
    let text = text.strip_prefix('+').unwrap_or(text);
    if text.is_empty() {
        return None;
    }
    let dotted = text.replace(decimal, ".");
    // The currency plays no part in the arithmetic; only the micro-units matter
    // here, and the line's own currency is parsed separately.
    let magnitude = Money::parse_fixed(&dotted, Currency::EUR).ok().map(Money::micros)?;
    Some(if negative { -magnitude } else { magnitude })
}

/// True when the mark column says the money left the account.
fn sign_from_mark(mark: &str) -> Option<bool> {
    let mark = mark.trim().to_ascii_lowercase();
    if DEBIT_MARKS.contains(&mark.as_str()) {
        return Some(true);
    }
    if CREDIT_MARKS.contains(&mark.as_str()) {
        return Some(false);
    }
    None
}

/// Column positions, resolved once per file so a row never looks one up.
struct Resolved {
    date: usize,
    value_date: Option<usize>,
    amount: Option<usize>,
    debit: Option<usize>,
    credit: Option<usize>,
    mark: Option<usize>,
    currency: Option<usize>,
    counterparty: Option<usize>,
    counterparty_iban: Option<usize>,
    remittance: Option<usize>,
    external_ref: Option<usize>,
}

impl Resolved {
    fn new(profile: &CsvProfile, header: &[String]) -> Result<Self, BankError> {
        let columns = &profile.columns;
        Ok(Self {
            date: resolve("date", &columns.date, header)?,
            value_date: resolve_optional("value_date", &columns.value_date, header)?,
            amount: resolve_optional("amount", &columns.amount, header)?,
            debit: resolve_optional("debit", &columns.debit, header)?,
            credit: resolve_optional("credit", &columns.credit, header)?,
            mark: resolve_optional("mark", &columns.mark, header)?,
            currency: resolve_optional("currency", &columns.currency, header)?,
            counterparty: resolve_optional("counterparty", &columns.counterparty, header)?,
            counterparty_iban: resolve_optional("counterparty_iban", &columns.counterparty_iban, header)?,
            remittance: resolve_optional("remittance", &columns.remittance, header)?,
            external_ref: resolve_optional("external_ref", &columns.external_ref, header)?,
        })
    }
}

fn resolve_optional(key: &str, column: &Option<Column>, header: &[String]) -> Result<Option<usize>, BankError> {
    match column {
        Some(column) => resolve(key, column, header).map(Some),
        None => Ok(None),
    }
}

fn resolve(key: &str, column: &Column, header: &[String]) -> Result<usize, BankError> {
    match column {
        Column::Index(position) => usize::try_from(*position)
            .ok()
            .map(|position| position - 1)
            .ok_or_else(|| BankError::Missing(format!("columns.{key} names no column"))),
        Column::Name(wanted) => header
            .iter()
            .position(|cell| cell.eq_ignore_ascii_case(wanted.trim()))
            .ok_or_else(|| {
                BankError::Missing(format!(
                    "the header has no column {wanted:?} (columns.{key}); it has {}",
                    header.join(", ")
                ))
            }),
    }
}

/// One cell, trimmed; a missing cell reads as empty.
fn cell(fields: &[String], index: usize) -> &str {
    fields.get(index).map(|value| value.trim()).unwrap_or("")
}

/// One cell as text, when it holds something.
fn text_of(fields: &[String], index: Option<usize>) -> Option<String> {
    let index = index?;
    let value = cell(fields, index);
    if value.is_empty() { None } else { Some(value.to_owned()) }
}

/// Decodes the bytes according to the profile.
fn decode(bytes: &[u8], encoding: &str) -> Result<String, BankError> {
    // A UTF-8 BOM is not part of the first header name, and dropping it here saves
    // every profile from having to know about it.
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match encoding.to_ascii_lowercase().as_str() {
        "windows-1252" | "cp1252" | "latin1" | "iso-8859-1" => {
            let (text, _, had_errors) = encoding_rs::WINDOWS_1252.decode(bytes);
            if had_errors {
                return Err(BankError::Malformed(
                    "the file is not readable as windows-1252".to_owned(),
                ));
            }
            Ok(text.into_owned())
        }
        _ => std::str::from_utf8(bytes).map(str::to_owned).map_err(|error| {
            BankError::Malformed(format!(
                "the file is not UTF-8 (byte {}); a profile can name windows-1252 or latin1",
                error.valid_up_to()
            ))
        }),
    }
}

/// Splits one line, honouring double quotes and doubled quotes inside them.
fn split(line: &str, delimiter: char) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut characters = line.chars().peekable();

    while let Some(character) = characters.next() {
        if quoted {
            if character == '"' {
                if characters.peek() == Some(&'"') {
                    characters.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(character);
            }
            continue;
        }
        if character == '"' && field.trim().is_empty() {
            quoted = true;
            field.clear();
            continue;
        }
        if character == delimiter {
            fields.push(std::mem::take(&mut field));
            continue;
        }
        field.push(character);
    }
    fields.push(field);
    fields
}

fn row_error(number: usize, what: &str) -> BankError {
    BankError::Malformed(format!("line {number}: {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(text: &str) -> CsvProfile {
        CsvProfile::parse(text).expect("a usable profile")
    }

    const SIMPLE: &str = r#"
delimiter = ";"
decimal_separator = ","
date_format = "%d-%m-%Y"

[columns]
date = "Datum"
amount = "Bedrag"
currency = "Valuta"
counterparty = "Naam"
remittance = "Omschrijving"
"#;

    const SEMICOLON_EXPORT: &str = "Datum;Bedrag;Valuta;Naam;Omschrijving\n15-02-2026;1210,00;EUR;Klant BV;Betaling factuur 2026-0001\n20-02-2026;-60,50;EUR;Leverancier BV;Huur februari\n";

    #[test]
    fn reads_a_semicolon_export_with_a_decimal_comma() {
        let statements = read_csv(SEMICOLON_EXPORT.as_bytes(), &profile(SIMPLE)).unwrap();
        assert_eq!(statements.len(), 1, "one file is one statement");
        let statement = &statements[0];

        assert_eq!(statement.format, "csv");
        assert_eq!(statement.currency, Currency::EUR);
        assert_eq!(statement.transactions.len(), 2);
        assert_eq!(statement.period_start.unwrap().to_iso_string(), "2026-02-15");
        assert_eq!(statement.period_end.unwrap().to_iso_string(), "2026-02-20");
        assert_eq!(
            statement.opening_balance_micros, None,
            "a CSV states no balance, and claiming one would pass an arithmetic check that never ran"
        );

        let first = &statement.transactions[0];
        assert_eq!(first.booking_date.to_iso_string(), "2026-02-15");
        assert_eq!(first.amount_micros, 1_210_000_000);
        assert_eq!(first.counterparty_name.as_deref(), Some("Klant BV"));
        assert_eq!(first.remittance_info.as_deref(), Some("Betaling factuur 2026-0001"));

        let second = &statement.transactions[1];
        assert_eq!(second.amount_micros, -60_500_000, "a minus is money out");
    }

    #[test]
    fn the_mark_column_decides_the_direction() {
        let text = "Date,Amount,Af Bij,Name\n2026-02-15,1210.00,Bij,Klant BV\n2026-02-20,60.50,Af,Bank\n";
        let profile = profile(
            r#"
[columns]
date = "Date"
amount = "Amount"
mark = "Af Bij"
counterparty = "Name"
"#,
        );
        let statement = &read_csv(text.as_bytes(), &profile).unwrap()[0];
        assert_eq!(statement.transactions[0].amount_micros, 1_210_000_000);
        assert_eq!(statement.transactions[1].amount_micros, -60_500_000);
    }

    #[test]
    fn a_stray_minus_cannot_flip_the_mark() {
        let text = "Date,Amount,Af Bij\n2026-02-20,-60.50,Af\n";
        let profile = profile(
            r#"
[columns]
date = "Date"
amount = "Amount"
mark = "Af Bij"
"#,
        );
        let statement = &read_csv(text.as_bytes(), &profile).unwrap()[0];
        assert_eq!(statement.transactions[0].amount_micros, -60_500_000);
    }

    #[test]
    fn an_unknown_mark_is_refused_with_its_line() {
        let text = "Date,Amount,Af Bij\n2026-02-20,60.50,Onbekend\n";
        let profile = profile(
            r#"
[columns]
date = "Date"
amount = "Amount"
mark = "Af Bij"
"#,
        );
        let error = read_csv(text.as_bytes(), &profile).unwrap_err();
        assert!(error.to_string().contains("line 2"), "{error}");
        assert!(error.to_string().contains("mark"), "{error}");
    }

    #[test]
    fn two_amount_columns_are_read_as_debit_and_credit() {
        let text = "Date;Debit;Credit\n15-02-2026;;1.210,00\n16-02-2026;60,50;\n";
        let profile = profile(
            r#"
delimiter = ";"
decimal_separator = ","
thousands_separator = "."
date_format = "%d-%m-%Y"

[columns]
date = "Date"
debit = "Debit"
credit = "Credit"
"#,
        );
        let statement = &read_csv(text.as_bytes(), &profile).unwrap()[0];
        assert_eq!(statement.transactions[0].amount_micros, 1_210_000_000);
        assert_eq!(statement.transactions[1].amount_micros, -60_500_000);
    }

    #[test]
    fn a_row_in_both_amount_columns_is_refused() {
        let text = "Date;Debit;Credit\n15-02-2026;1,00;2,00\n";
        let profile = profile(
            r#"
delimiter = ";"
decimal_separator = ","
date_format = "%d-%m-%Y"

[columns]
date = "Date"
debit = "Debit"
credit = "Credit"
"#,
        );
        let error = read_csv(text.as_bytes(), &profile).unwrap_err();
        assert!(error.to_string().contains("both"), "{error}");
    }

    #[test]
    fn a_footer_that_is_not_a_row_is_refused_with_its_line_number() {
        let text = "Datum;Bedrag\n15-02-2026;10,00\nTotaal;10,00\n";
        let profile = profile(
            r#"
delimiter = ";"
decimal_separator = ","
date_format = "%d-%m-%Y"

[columns]
date = "Datum"
amount = "Bedrag"
"#,
        );
        let error = read_csv(text.as_bytes(), &profile).unwrap_err();
        assert!(error.to_string().contains("line 3"), "{error}");
        assert!(error.to_string().contains("Totaal"), "{error}");
    }

    #[test]
    fn a_row_that_moves_no_money_is_refused() {
        let text = "Datum;Bedrag\n15-02-2026;0,00\n";
        let profile = profile(
            r#"
delimiter = ";"
decimal_separator = ","
date_format = "%d-%m-%Y"

[columns]
date = "Datum"
amount = "Bedrag"
"#,
        );
        let error = read_csv(text.as_bytes(), &profile).unwrap_err();
        assert!(error.to_string().contains("moves no money"), "{error}");
    }

    #[test]
    fn a_header_the_profile_names_is_missing() {
        let text = "Date,Amount\n2026-02-15,10.00\n";
        let profile = profile(
            r#"
[columns]
date = "Datum"
amount = "Amount"
"#,
        );
        let error = read_csv(text.as_bytes(), &profile).unwrap_err();
        assert!(error.to_string().contains("Datum"), "{error}");
        assert!(
            error.to_string().contains("Date, Amount"),
            "the header is shown: {error}"
        );
    }

    #[test]
    fn a_byte_order_mark_does_not_become_part_of_the_first_name() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"Date,Amount\n2026-02-15,10.00\n");
        let profile = profile(
            r#"
[columns]
date = "Date"
amount = "Amount"
"#,
        );
        let statement = &read_csv(&bytes, &profile).unwrap()[0];
        assert_eq!(statement.transactions[0].amount_micros, 10_000_000);
    }

    #[test]
    fn a_latin1_export_is_read_as_the_profile_says() {
        let mut bytes = b"Date,Amount,Name\n2026-02-15,10.00,Caf".to_vec();
        bytes.push(0xE9);
        bytes.extend_from_slice(b" BV\n");
        let profile = profile(
            r#"
encoding = "latin1"

[columns]
date = "Date"
amount = "Amount"
counterparty = "Name"
"#,
        );
        let statement = &read_csv(&bytes, &profile).unwrap()[0];
        assert_eq!(statement.transactions[0].counterparty_name.as_deref(), Some("Café BV"));
    }

    #[test]
    fn utf8_is_the_default_and_a_latin1_byte_is_named() {
        let mut bytes = b"Date,Amount,Name\n2026-02-15,10.00,Caf".to_vec();
        bytes.push(0xE9);
        bytes.extend_from_slice(b" BV\n");
        let profile = profile(
            r#"
[columns]
date = "Date"
amount = "Amount"
counterparty = "Name"
"#,
        );
        let error = read_csv(&bytes, &profile).unwrap_err();
        assert!(error.to_string().contains("not UTF-8"), "{error}");
        assert!(error.to_string().contains("windows-1252"), "{error}");
    }

    #[test]
    fn a_thousands_separator_and_a_trailing_minus_are_understood() {
        assert_eq!(amount_micros("1.210,00", ',', Some('.')), Some(1_210_000_000));
        assert_eq!(amount_micros("1.210,00-", ',', Some('.')), Some(-1_210_000_000));
        assert_eq!(amount_micros("+ 12", ',', None), Some(12_000_000));
        assert_eq!(amount_micros("twaalf", ',', None), None);
    }

    #[test]
    fn an_export_without_a_header_uses_positions() {
        let text = "2026-02-15,10.00\n2026-02-16,20.00\n";
        let profile = profile(
            r#"
has_header = false
skip_lines = 0

[columns]
date = 1
amount = 2
"#,
        );
        let statement = &read_csv(text.as_bytes(), &profile).unwrap()[0];
        assert_eq!(statement.transactions.len(), 2);
        assert_eq!(statement.transactions[1].amount_micros, 20_000_000);
    }

    #[test]
    fn a_preamble_is_skipped_before_the_header() {
        let text = "Rabo export;versie 2\n\nDatum;Bedrag\n15-02-2026;10,00\n";
        let profile = profile(
            r#"
delimiter = ";"
decimal_separator = ","
date_format = "%d-%m-%Y"
skip_lines = 2

[columns]
date = "Datum"
amount = "Bedrag"
"#,
        );
        let statement = &read_csv(text.as_bytes(), &profile).unwrap()[0];
        assert_eq!(statement.transactions.len(), 1);
        assert_eq!(statement.transactions[0].amount_micros, 10_000_000);
    }

    #[test]
    fn a_blank_line_is_padding_not_a_row() {
        let text = "Datum;Bedrag\n15-02-2026;10,00\n\n16-02-2026;20,00\n";
        let profile = profile(
            r#"
delimiter = ";"
decimal_separator = ","
date_format = "%d-%m-%Y"

[columns]
date = "Datum"
amount = "Bedrag"
"#,
        );
        let statement = &read_csv(text.as_bytes(), &profile).unwrap()[0];
        assert_eq!(statement.transactions.len(), 2);
    }

    #[test]
    fn quoted_delimiters_and_doubled_quotes_stay_in_their_cell() {
        let text = "Datum;Bedrag;Omschrijving\n15-02-2026;10,00;\"Factuur 2026-0001; deel 1\"\n16-02-2026;20,00;\"Hij zei \"\"hoi\"\"\"\n";
        let profile = profile(
            r#"
delimiter = ";"
decimal_separator = ","
date_format = "%d-%m-%Y"

[columns]
date = "Datum"
amount = "Bedrag"
remittance = "Omschrijving"
"#,
        );
        let statement = &read_csv(text.as_bytes(), &profile).unwrap()[0];
        assert_eq!(
            statement.transactions[0].remittance_info.as_deref(),
            Some("Factuur 2026-0001; deel 1")
        );
        assert_eq!(
            statement.transactions[1].remittance_info.as_deref(),
            Some("Hij zei \"hoi\"")
        );
    }

    #[test]
    fn a_profile_needs_an_amount_column() {
        let error = CsvProfile::parse(
            r#"
[columns]
date = "Datum"
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("columns.amount"), "{error}");
    }

    #[test]
    fn two_ways_of_naming_the_amount_are_refused() {
        let error = CsvProfile::parse(
            r#"
[columns]
date = "Datum"
amount = "Bedrag"
debit = "Debet"
credit = "Credit"
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("not both"), "{error}");
    }

    #[test]
    fn the_same_character_for_decimal_and_thousands_is_refused() {
        let error = CsvProfile::parse(
            r#"
decimal_separator = ","
thousands_separator = ","

[columns]
date = "Datum"
amount = "Bedrag"
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("same character"), "{error}");
    }

    #[test]
    fn an_unknown_date_layout_is_named_with_the_ones_we_read() {
        let error = CsvProfile::parse(
            r#"
date_format = "%d %B %Y"

[columns]
date = "Datum"
amount = "Bedrag"
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("%d-%m-%Y"), "{error}");
    }

    #[test]
    fn a_header_name_without_a_header_is_refused() {
        let error = CsvProfile::parse(
            r#"
has_header = false

[columns]
date = "Datum"
amount = 2
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("position"), "{error}");
    }

    #[test]
    fn a_misspelled_key_is_refused_rather_than_ignored() {
        let error = CsvProfile::parse(
            r#"
delimeter = ";"

[columns]
date = "Datum"
amount = "Bedrag"
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("delimeter"), "{error}");
    }

    #[test]
    fn the_date_layouts_we_advertise_all_read() {
        let cases = [
            ("%Y-%m-%d", "2026-02-15"),
            ("%Y/%m/%d", "2026/02/15"),
            ("%d-%m-%Y", "15-02-2026"),
            ("%d/%m/%Y", "15/02/2026"),
            ("%d.%m.%Y", "15.02.2026"),
            ("%m/%d/%Y", "02/15/2026"),
            ("%d-%m-%y", "15-02-26"),
            ("%d/%m/%y", "15/02/26"),
            ("%Y%m%d", "20260215"),
            ("%d%m%Y", "15022026"),
        ];
        for (format, text) in cases {
            let layout = DateLayout::parse(format).expect(format);
            assert_eq!(
                layout.read(text).map(|date| date.to_iso_string()),
                Some("2026-02-15".to_owned()),
                "{format} on {text}"
            );
        }
        assert_eq!(DateLayout::parse("%Y-%m-%d").unwrap().read("2026-02-31"), None);
    }
}
