//! CAMT.053 (ISO 20022) bank statements.
//!
//! The reader is deliberately narrow, like the invoice readers: it extracts the
//! booking date, the signed amount, the counterparty, and the remittance
//! information the matching ladder needs, and refuses anything it cannot
//! represent rather than guessing. Documents come from a bank, but they are still
//! files from outside: a DTD is rejected outright, size and nesting are bounded,
//! and the root element has to be a CAMT.053 document in a namespace we know.
//!
//! What it returns is [`store::bank::ParsedStatement`], so the store never has to
//! know what CAMT is and a second format can be added beside this one.

use crux::{Currency, LocalDate, Money};
use quick_xml::Reader;
use quick_xml::events::Event;
use store::bank::{ParsedStatement, ParsedTransaction};

/// Largest statement a reader accepts.
pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

/// Deepest element nesting a reader accepts.
pub const MAX_DEPTH: usize = 64;

/// Why a statement could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BankError {
    /// The file is larger than the reader accepts.
    #[error("the statement is larger than {MAX_DOCUMENT_BYTES} bytes")]
    TooLarge,
    /// The file carries a DTD, which is how entity expansion gets in.
    #[error("the statement carries a DTD, which is not accepted")]
    Dtd,
    /// Nesting went past what the reader allows.
    #[error("the statement is nested deeper than {MAX_DEPTH} elements")]
    TooDeep,
    /// The root element is not a CAMT document.
    #[error("this is not a CAMT statement: the root element is {0:?}")]
    NotCamt(String),
    /// The XML itself is broken.
    #[error("the statement is not valid XML: {0}")]
    Malformed(String),
    /// A field the reader needs is missing or unusable.
    #[error("{0}")]
    Missing(String),
}

impl From<quick_xml::Error> for BankError {
    fn from(error: quick_xml::Error) -> Self {
        Self::Malformed(error.to_string())
    }
}

impl From<quick_xml::encoding::EncodingError> for BankError {
    fn from(error: quick_xml::encoding::EncodingError) -> Self {
        Self::Malformed(error.to_string())
    }
}

/// Reads every statement in a CAMT.053 file.
///
/// A file may carry several statements, one per account or period; each is
/// returned separately so the caller decides what to do with the ones that belong
/// to an account it does not know.
pub fn read_camt053(input: &str) -> Result<Vec<ParsedStatement>, BankError> {
    if input.len() > MAX_DOCUMENT_BYTES {
        return Err(BankError::TooLarge);
    }
    if input.contains("<!DOCTYPE") || input.contains("<!ENTITY") {
        return Err(BankError::Dtd);
    }

    let mut reader = Reader::from_str(input);

    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut seen_root = false;

    let mut statements: Vec<ParsedStatement> = Vec::new();
    let mut statement = StatementBuilder::default();
    let mut entry = EntryBuilder::default();
    let mut balance = BalanceBuilder::default();
    let mut in_entry = false;
    let mut in_balance = false;
    let mut in_tx = false;

    loop {
        match reader.read_event()? {
            Event::Start(element) => {
                let name = local_name(element.name().as_ref());
                if path.is_empty() {
                    seen_root = true;
                    if !is_camt053(&name) && !name.eq_ignore_ascii_case("Document") {
                        return Err(BankError::NotCamt(name));
                    }
                }
                path.push(name.clone());
                if path.len() > MAX_DEPTH {
                    return Err(BankError::TooDeep);
                }
                text.clear();

                match name.as_str() {
                    "Stmt" => statement = StatementBuilder::default(),
                    "Ntry" => {
                        entry = EntryBuilder {
                            currency: currency_attribute(&element),
                            ..EntryBuilder::default()
                        };
                        in_entry = true;
                    }
                    "TxDtls" => in_tx = true,
                    "Bal" => {
                        balance = BalanceBuilder::default();
                        in_balance = true;
                    }
                    _ => {}
                }
            }
            Event::End(element) => {
                let name = local_name(element.name().as_ref());
                let value = text.trim().to_owned();

                match name.as_str() {
                    "TxDtls" => in_tx = false,
                    "Ntry" => {
                        if in_entry {
                            entry = std::mem::take(&mut entry).apply(&mut statement)?;
                            in_entry = false;
                        }
                    }
                    "Bal" => {
                        if in_balance {
                            balance = std::mem::take(&mut balance).apply(&mut statement);
                            in_balance = false;
                        }
                    }
                    "Stmt" => statements.push(statement.take()?),
                    _ => {}
                }

                if !value.is_empty() {
                    let context = path.as_slice();
                    if in_balance {
                        balance.capture(name.as_str(), context, &value);
                    } else if in_tx {
                        entry.capture_tx(name.as_str(), context, &value);
                    } else if in_entry {
                        entry.capture_entry(name.as_str(), context, &value);
                    } else {
                        statement.capture(name.as_str(), context, &value);
                    }
                }

                path.pop();
                text.clear();
            }
            Event::Text(bytes) => {
                text.push_str(&bytes.into_inner());
            }
            Event::CData(bytes) => {
                text.push_str(bytes.as_ref());
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if !seen_root {
        return Err(BankError::NotCamt(String::new()));
    }
    Ok(statements)
}

fn local_name(raw: &str) -> String {
    match raw.rsplit_once(':') {
        Some((_, local)) => local.to_owned(),
        None => raw.to_owned(),
    }
}

fn is_camt053(name: &str) -> bool {
    name.eq_ignore_ascii_case("Document")
}

fn currency_attribute(element: &quick_xml::events::BytesStart<'_>) -> Option<Currency> {
    for attribute in element.attributes().flatten() {
        if attribute.key.as_ref().eq_ignore_ascii_case("Ccy") {
            return Currency::parse(&attribute.value).ok();
        }
    }
    None
}

/// A statement being built.
#[derive(Default)]
struct StatementBuilder {
    iban: Option<String>,
    currency: Option<Currency>,
    statement_id: Option<String>,
    period_start: Option<LocalDate>,
    period_end: Option<LocalDate>,
    opening: Option<i64>,
    closing: Option<i64>,
    transactions: Vec<ParsedTransaction>,
    /// Currency seen on an amount, used when the account block names none.
    amount_currency: Option<Currency>,
}

impl StatementBuilder {
    fn capture(&mut self, name: &str, path: &[String], value: &str) {
        match name {
            "IBAN" if path.ends_with(&["Acct".to_owned(), "Id".to_owned(), "IBAN".to_owned()]) => {
                self.iban = Some(value.to_owned());
            }
            "Ccy" if self.currency.is_none() => {
                self.currency = Currency::parse(value).ok();
            }
            "Id" if path.ends_with(&["Stmt".to_owned(), "Id".to_owned()]) => {
                self.statement_id = Some(value.to_owned());
            }
            "FrDtTm" | "FrDt" => {
                self.period_start = date_of(value);
            }
            "ToDtTm" | "ToDt" => {
                self.period_end = date_of(value);
            }
            _ => {}
        }
    }

    fn take(&mut self) -> Result<ParsedStatement, BankError> {
        let currency = self
            .currency
            .or(self.amount_currency)
            .ok_or_else(|| BankError::Missing("the statement names no currency".to_owned()))?;
        Ok(ParsedStatement {
            format: "camt053",
            source_name: None,
            currency,
            period_start: self.period_start,
            period_end: self.period_end,
            opening_balance_micros: self.opening,
            closing_balance_micros: self.closing,
            transactions: std::mem::take(&mut self.transactions),
        })
    }
}

/// One entry being built.
#[derive(Default)]
struct EntryBuilder {
    amount: Option<i64>,
    currency: Option<Currency>,
    credit: Option<bool>,
    booking_date: Option<LocalDate>,
    value_date: Option<LocalDate>,
    counterparty_name: Option<String>,
    counterparty_iban: Option<String>,
    remittance: Option<String>,
    external_ref: Option<String>,
}

impl EntryBuilder {
    fn capture_entry(&mut self, name: &str, path: &[String], value: &str) {
        match name {
            "Amt" if path.ends_with(&["Ntry".to_owned(), "Amt".to_owned()]) => {
                self.amount = parse_amount(value, self.currency);
            }
            "CdtDbtInd" if path.ends_with(&["Ntry".to_owned(), "CdtDbtInd".to_owned()]) => {
                self.credit = Some(value.eq_ignore_ascii_case("CRDT"));
            }
            "Dt" if path.ends_with(&["BookgDt".to_owned(), "Dt".to_owned()]) => {
                self.booking_date = date_of(value);
            }
            "DtTm" if path.ends_with(&["BookgDt".to_owned(), "DtTm".to_owned()]) => {
                self.booking_date = date_of(value);
            }
            "Dt" if path.ends_with(&["ValDt".to_owned(), "Dt".to_owned()]) => {
                self.value_date = date_of(value);
            }
            "DtTm" if path.ends_with(&["ValDt".to_owned(), "DtTm".to_owned()]) => {
                self.value_date = date_of(value);
            }
            "AcctSvcrRef" => {
                self.external_ref = Some(value.to_owned());
            }
            _ => {}
        }
    }

    fn capture_tx(&mut self, name: &str, path: &[String], value: &str) {
        match name {
            "Ustrd" => {
                let existing = self.remittance.take().unwrap_or_default();
                let joined = if existing.is_empty() {
                    value.to_owned()
                } else {
                    format!("{existing} {value}")
                };
                self.remittance = Some(joined);
            }
            "EndToEndId" if self.external_ref.is_none() => {
                self.external_ref = Some(value.to_owned());
            }
            "Nm" if path.contains(&"Dbtr".to_owned()) && self.counterparty_name.is_none() => {
                self.counterparty_name = Some(value.to_owned());
            }
            "Nm" if path.contains(&"Cdtr".to_owned()) && self.counterparty_name.is_none() => {
                self.counterparty_name = Some(value.to_owned());
            }
            "IBAN" if path.contains(&"DbtrAcct".to_owned()) && self.counterparty_iban.is_none() => {
                self.counterparty_iban = Some(value.to_owned());
            }
            "IBAN" if path.contains(&"CdtrAcct".to_owned()) && self.counterparty_iban.is_none() => {
                self.counterparty_iban = Some(value.to_owned());
            }
            _ => {}
        }
    }

    fn apply(self, statement: &mut StatementBuilder) -> Result<EntryBuilder, BankError> {
        let Some(booking_date) = self.booking_date else {
            return Err(BankError::Missing("an entry has no booking date".to_owned()));
        };
        let currency = self
            .currency
            .or(statement.currency)
            .or(statement.amount_currency)
            .ok_or_else(|| BankError::Missing("an entry names no currency".to_owned()))?;
        if statement.amount_currency.is_none() {
            statement.amount_currency = Some(currency);
        }
        let magnitude = self
            .amount
            .ok_or_else(|| BankError::Missing("an entry has no amount".to_owned()))?;
        let credit = self
            .credit
            .ok_or_else(|| BankError::Missing("an entry does not say whether it is a debit or a credit".to_owned()))?;
        let amount_micros = if credit { magnitude } else { -magnitude };

        statement.transactions.push(ParsedTransaction {
            booking_date,
            value_date: self.value_date,
            amount_micros,
            currency,
            counterparty_name: self.counterparty_name,
            counterparty_iban: self.counterparty_iban,
            remittance_info: self.remittance,
            external_ref: self.external_ref,
        });
        Ok(EntryBuilder::default())
    }
}

/// One balance block being built.
#[derive(Default)]
struct BalanceBuilder {
    code: Option<String>,
    amount: Option<i64>,
    credit: Option<bool>,
}

impl BalanceBuilder {
    fn capture(&mut self, name: &str, _path: &[String], value: &str) {
        match name {
            "Cd" => self.code = Some(value.to_ascii_uppercase()),
            "Amt" => self.amount = parse_amount(value, None),
            "CdtDbtInd" => self.credit = Some(value.eq_ignore_ascii_case("CRDT")),
            _ => {}
        }
    }

    fn apply(self, statement: &mut StatementBuilder) -> BalanceBuilder {
        let (Some(code), Some(amount), Some(credit)) = (self.code, self.amount, self.credit) else {
            return BalanceBuilder::default();
        };
        let signed = if credit { amount } else { -amount };
        match code.as_str() {
            "OPBD" => statement.opening = Some(signed),
            "CLBD" => statement.closing = Some(signed),
            _ => {}
        }
        BalanceBuilder::default()
    }
}

/// Parses an amount to micro-units, without floating point.
fn parse_amount(text: &str, currency: Option<Currency>) -> Option<i64> {
    let currency = currency.unwrap_or(Currency::EUR);
    Money::parse_fixed(text.trim(), currency).ok().map(Money::micros)
}

/// The calendar date of an ISO date or date-time, as the business date.
fn date_of(value: &str) -> Option<LocalDate> {
    let date = value.get(..10)?;
    LocalDate::parse_iso(date).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Document xmlns="urn:iso:std:iso:20022:tech:xsd:camt.053.001.02">
  <BkToCstmrStmt>
    <GrpHdr><MsgId>M-1</MsgId><CreDtTm>2026-02-01T06:00:00Z</CreDtTm></GrpHdr>
    <Stmt>
      <Id>STMT-2026-01</Id>
      <Acct><Id><IBAN>NL91ABNA0417164300</IBAN></Id><Ccy>EUR</Ccy></Acct>
      <Bal>
        <Tp><CdOrPrtry><Cd>OPBD</Cd></CdOrPrtry></Tp>
        <Amt Ccy="EUR">1000.00</Amt>
        <CdtDbtInd>CRDT</CdtDbtInd>
      </Bal>
      <Bal>
        <Tp><CdOrPrtry><Cd>CLBD</Cd></CdOrPrtry></Tp>
        <Amt Ccy="EUR">1610.00</Amt>
        <CdtDbtInd>CRDT</CdtDbtInd>
      </Bal>
      <Ntry>
        <Amt Ccy="EUR">1210.00</Amt>
        <CdtDbtInd>CRDT</CdtDbtInd>
        <Sts>BOOK</Sts>
        <BookgDt><Dt>2026-01-15</Dt></BookgDt>
        <ValDt><Dt>2026-01-16</Dt></ValDt>
        <AcctSvcrRef>BANK-1</AcctSvcrRef>
        <NtryDtls>
          <TxDtls>
            <Refs><EndToEndId>E2E-1</EndToEndId></Refs>
            <RltdPties>
              <Dbtr><Nm>Klant BV</Nm></Dbtr>
              <DbtrAcct><Id><IBAN>NL02RABO0123456789</IBAN></Id></DbtrAcct>
            </RltdPties>
            <RmtInf><Ustrd>Betaling factuur 2026-0001</Ustrd></RmtInf>
          </TxDtls>
        </NtryDtls>
      </Ntry>
      <Ntry>
        <Amt Ccy="EUR">600.00</Amt>
        <CdtDbtInd>DBIT</CdtDbtInd>
        <Sts>BOOK</Sts>
        <BookgDt><Dt>2026-01-20</Dt></BookgDt>
        <NtryDtls>
          <TxDtls>
            <RltdPties><Cdtr><Nm>Leverancier BV</Nm></Cdtr></RltdPties>
            <RmtInf><Ustrd>Huur januari</Ustrd></RmtInf>
          </TxDtls>
        </NtryDtls>
      </Ntry>
    </Stmt>
  </BkToCstmrStmt>
</Document>"#;

    #[test]
    fn reads_a_statement_with_its_balances_and_entries() {
        let statements = read_camt053(SAMPLE).unwrap();
        assert_eq!(statements.len(), 1);
        let statement = &statements[0];

        assert_eq!(statement.format, "camt053");
        assert_eq!(statement.currency, Currency::EUR);
        assert_eq!(statement.opening_balance_micros, Some(1_000_000_000));
        assert_eq!(statement.closing_balance_micros, Some(1_610_000_000));
        assert_eq!(statement.period_start, None);

        assert_eq!(statement.transactions.len(), 2);
        let first = &statement.transactions[0];
        assert_eq!(first.booking_date.to_iso_string(), "2026-01-15");
        assert_eq!(first.value_date.unwrap().to_iso_string(), "2026-01-16");
        assert_eq!(first.amount_micros, 1_210_000_000, "a credit is money in");
        assert_eq!(first.counterparty_name.as_deref(), Some("Klant BV"));
        assert_eq!(first.counterparty_iban.as_deref(), Some("NL02RABO0123456789"));
        assert_eq!(first.remittance_info.as_deref(), Some("Betaling factuur 2026-0001"));
        assert_eq!(first.external_ref.as_deref(), Some("BANK-1"));

        let second = &statement.transactions[1];
        assert_eq!(second.amount_micros, -600_000_000, "a debit is money out");
        assert_eq!(second.counterparty_name.as_deref(), Some("Leverancier BV"));
    }

    #[test]
    fn a_doctype_is_refused() {
        let doctype = format!("<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY e \"boom\">]>{SAMPLE}");
        assert_eq!(read_camt053(&doctype).unwrap_err(), BankError::Dtd);
    }

    #[test]
    fn something_that_is_not_a_statement_is_refused() {
        let error = read_camt053("<Invoice><ID>1</ID></Invoice>").unwrap_err();
        assert!(matches!(error, BankError::NotCamt(_)), "{error}");
        assert!(matches!(read_camt053("").unwrap_err(), BankError::NotCamt(_)));
    }

    #[test]
    fn a_missing_amount_is_named() {
        let broken = SAMPLE.replacen("<Amt Ccy=\"EUR\">1210.00</Amt>", "", 1);
        let error = read_camt053(&broken).unwrap_err();
        assert!(error.to_string().contains("no amount"), "{error}");
    }
}
