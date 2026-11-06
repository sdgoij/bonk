//! MT940 (SWIFT) bank statements.
//!
//! MT940 is what most European banks still hand out from online banking: a flat
//! list of `:NN:` tags, one field per line, with continuation lines below the tag
//! they belong to. It predates ISO 20022 and has no schema, so the reader is
//! narrow on purpose. It takes the balance fields, the `:61:` booking lines, and
//! the `:86:` remittance text, and refuses a file whose tags or arithmetic it
//! cannot follow rather than inventing a value.
//!
//! Three choices are worth stating, because each one is a place where a reader
//! *could* be clever and be wrong:
//!
//! * `:86:` is stored as it arrives, its continuation lines folded into one line
//!   with a single space. Banks stuff structured subfields into it (`/IBAN/…`,
//!   `/NAME/…`, `/REMI/…`), but the layout differs per bank, and decoding the
//!   wrong layout invents a counterparty. Keeping the text whole also means the
//!   matcher still finds an invoice number in it, and a later extension can
//!   decode it without the original having been thrown away.
//! * The bank reference after `//` in `:61:` becomes the line's external
//!   reference. `:20:` is the *statement's* sender reference and appears once on
//!   every line, so it identifies no line; the sixteen-character customer
//!   reference before `//` is routinely the placeholder `NONREF`.
//! * `RC`/`RD` are read as reversals: reversing a credit debits the account and
//!   reversing a debit credits it. Any other mark is refused, because the sign
//!   of an amount is the one thing a reader must never guess.
//!
//! A two-digit year is read as `20xx`. MT940 predates the four-digit year, and a
//! pre-2000 statement is not something this reader supports; picking a century
//! would silently move the books.

use crux::{Currency, LocalDate, Money};
use store::bank::{ParsedStatement, ParsedTransaction};

use crate::bank_file::{BankError, MAX_DOCUMENT_BYTES};

/// Reads a MT940 statement.
///
/// One file holds one statement block, so the result carries exactly one
/// statement; it is a list because that is what every statement reader returns
/// and what the import path walks. A file with no `:60F:` opening balance is not
/// a statement we can reconcile, so it is refused instead of stored with a
/// missing balance.
pub fn read_mt940(input: &str) -> Result<Vec<ParsedStatement>, BankError> {
    if input.len() > MAX_DOCUMENT_BYTES {
        return Err(BankError::TooLarge);
    }

    let mut opening: Option<(LocalDate, Currency, i64)> = None;
    let mut closing: Option<(LocalDate, i64)> = None;
    let mut currency: Option<Currency> = None;
    let mut transactions: Vec<ParsedTransaction> = Vec::new();

    for field in tags(input)? {
        match field.tag.as_str() {
            "60F" => {
                let (date, code, micros) = balance_of(&field.value, "the :60F: opening balance")?;
                currency = Some(code);
                opening = Some((date, code, micros));
            }
            "62F" => {
                let (date, code, micros) = balance_of(&field.value, "the :62F: closing balance")?;
                // A statement that starts with the closing balance is odd but
                // still readable; taking the currency here means the file says
                // what it means even when :60F: is the one that is missing.
                currency = currency.or(Some(code));
                closing = Some((date, micros));
            }
            "61" => {
                let code = currency.ok_or_else(|| {
                    BankError::Missing(
                        "a :61: booking line comes before the :60F: opening balance, so its currency is unknown"
                            .to_owned(),
                    )
                })?;
                transactions.push(booking_line(&field.value, code)?);
            }
            "86" => attach_remittance(&mut transactions, &field.value),
            _ => {}
        }
    }

    let Some((period_start, currency, opening_micros)) = opening else {
        return Err(BankError::Missing(
            "the file has no :60F: opening balance, so it is not a statement this reader can reconcile".to_owned(),
        ));
    };

    Ok(vec![ParsedStatement {
        format: "mt940",
        source_name: None,
        currency,
        period_start: Some(period_start),
        period_end: closing.map(|(date, _)| date),
        opening_balance_micros: Some(opening_micros),
        closing_balance_micros: closing.map(|(_, micros)| micros),
        transactions,
    }])
}

/// One `:NN:` field with its continuation lines folded in.
struct Field {
    tag: String,
    value: String,
}

/// Splits a file into fields.
///
/// A line that starts with `:` opens a field; any other line belongs to the field
/// above it. A line before the first tag is SWIFT block framing (`{1:F01…}`, the
/// `-` trailer), which carries no field and is skipped. A tag that opens and
/// never closes the field name — `:60F:C…` with no second colon — is refused,
/// because guessing where the tag ends would shift every value on the line.
fn tags(input: &str) -> Result<Vec<Field>, BankError> {
    let mut fields: Vec<Field> = Vec::new();
    for (index, raw) in input.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // `{1:F01…}` and `{2:I940}` frame the message; `-}` ends it. None of them
        // carries a field, and nothing after the trailer does either.
        if line.starts_with('-') {
            break;
        }
        if line.starts_with('{') || line.starts_with('}') {
            continue;
        }
        let Some(rest) = line.strip_prefix(':') else {
            if let Some(last) = fields.last_mut() {
                last.value.push(' ');
                last.value.push_str(line);
            }
            continue;
        };
        let (tag, value) = rest.split_once(':').ok_or_else(|| {
            BankError::Malformed(format!("line {} opens a tag that never closes: {line:?}", index + 1))
        })?;
        let digits = tag.len() == 2 || tag.len() == 3;
        let well_formed = digits
            && tag.as_bytes()[..2].iter().all(u8::is_ascii_digit)
            && tag.as_bytes().get(2).is_none_or(|byte| byte.is_ascii_alphabetic());
        if !well_formed {
            return Err(BankError::Malformed(format!(
                "line {} does not start a MT940 tag: {line:?}",
                index + 1
            )));
        }
        fields.push(Field {
            tag: tag.to_ascii_uppercase(),
            value: value.to_owned(),
        });
    }
    Ok(fields)
}

/// Appends a `:86:` block to the booking line above it.
///
/// A `:86:` with no booking line above it is information to the account owner
/// rather than a payment's remittance, so there is nothing to attach it to and it
/// is dropped rather than turned into a phantom transaction.
fn attach_remittance(transactions: &mut [ParsedTransaction], value: &str) {
    let Some(last) = transactions.last_mut() else {
        return;
    };
    let text = value.trim();
    if text.is_empty() {
        return;
    }
    match last.remittance_info.as_mut() {
        Some(existing) => {
            existing.push(' ');
            existing.push_str(text);
        }
        None => last.remittance_info = Some(text.to_owned()),
    }
}

/// Reads a `:60F:`/`:62F:` value: mark, date, currency, amount.
fn balance_of(value: &str, what: &str) -> Result<(LocalDate, Currency, i64), BankError> {
    let mut rest = value.trim();
    let debit = debit_mark(&mut rest, what)?;
    let date = take_date(&mut rest, what)?;
    let (code, tail) = rest
        .split_at_checked(3)
        .ok_or_else(|| BankError::Malformed(format!("{what} names no currency: {value:?}")))?;
    let currency = Currency::parse(code)
        .map_err(|_| BankError::Malformed(format!("{what} has an unreadable currency: {code:?}")))?;
    let micros = amount_of(tail, currency, what)?;
    Ok((date, currency, if debit { -micros } else { micros }))
}

/// Reads a `:61:` value: the booking line.
fn booking_line(value: &str, currency: Currency) -> Result<ParsedTransaction, BankError> {
    let what = "the :61: booking line";
    let mut rest = value.trim();
    let booking_date = take_date(&mut rest, what)?;

    // The entry date is optional and four digits long; the value date is six, so
    // what follows must be checked before it is taken as the entry date.
    let mut value_date = None;
    if let Some((head, tail)) = rest.split_at_checked(4)
        && head.bytes().all(|byte| byte.is_ascii_digit())
        && let Some(date) = entry_date(head, booking_date)
    {
        value_date = Some(date);
        rest = tail;
    }

    let debit = debit_mark(&mut rest, what)?;

    // An optional one-letter funds code sits between the mark and the amount.
    if rest.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        let (_, tail) = rest.split_at(1);
        rest = tail;
    }

    let (number, tail) = split_amount(rest);
    let magnitude = amount_of(number, currency, what)?;
    let external_ref = tail
        .split_once("//")
        .map(|(_, bank)| bank.trim())
        .filter(|bank| !bank.is_empty())
        .map(str::to_owned);

    Ok(ParsedTransaction {
        booking_date,
        value_date,
        amount_micros: if debit { -magnitude } else { magnitude },
        currency,
        // `:86:` is kept whole rather than decoded, so no counterparty is known
        // from the file itself. An extension that learns a bank's layout can fill
        // these in later from the text that is stored.
        counterparty_name: None,
        counterparty_iban: None,
        remittance_info: None,
        external_ref,
    })
}

/// True when the mark says the account was debited.
fn debit_mark(rest: &mut &str, what: &str) -> Result<bool, BankError> {
    let (debit, width) = if rest.starts_with("RC") {
        // Reversing a credit takes the money back out.
        (true, 2)
    } else if rest.starts_with("RD") {
        (false, 2)
    } else if rest.starts_with('C') {
        (false, 1)
    } else if rest.starts_with('D') {
        (true, 1)
    } else {
        return Err(BankError::Malformed(format!(
            "{what} has no debit/credit mark: {:?}",
            *rest
        )));
    };
    let (_, tail) = rest.split_at(width);
    *rest = tail;
    Ok(debit)
}

/// Takes a six-digit `YYMMDD` date off the front.
fn take_date(rest: &mut &str, what: &str) -> Result<LocalDate, BankError> {
    let (head, tail) = rest
        .split_at_checked(6)
        .ok_or_else(|| BankError::Malformed(format!("{what} has no date: {:?}", *rest)))?;
    let date =
        yymmdd(head).ok_or_else(|| BankError::Malformed(format!("{what} has a date that is not YYMMDD: {head:?}")))?;
    *rest = tail;
    Ok(date)
}

/// A six-digit SWIFT date, read as a year in this century.
fn yymmdd(value: &str) -> Option<LocalDate> {
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = 2000 + value.get(0..2)?.parse::<i32>().ok()?;
    let month = value.get(2..4)?.parse::<u8>().ok()?;
    let day = value.get(4..6)?.parse::<u8>().ok()?;
    LocalDate::from_ymd(year, month, day).ok()
}

/// A four-digit `MMDD` entry date, in the year of the value date.
fn entry_date(mmdd: &str, value_date: LocalDate) -> Option<LocalDate> {
    let year = value_date.to_iso_string().get(0..4)?.parse::<i32>().ok()?;
    let month = mmdd.get(0..2)?.parse::<u8>().ok()?;
    let day = mmdd.get(2..4)?.parse::<u8>().ok()?;
    LocalDate::from_ymd(year, month, day).ok()
}

/// Splits the amount off the front of a booking line's remainder.
fn split_amount(text: &str) -> (&str, &str) {
    let end = text
        .find(|character: char| !character.is_ascii_digit() && character != ',')
        .unwrap_or(text.len());
    text.split_at(end)
}

/// Reads a comma-decimal amount to micro-units, without floating point.
fn amount_of(text: &str, currency: Currency, what: &str) -> Result<i64, BankError> {
    let number = text.trim();
    if number.is_empty() {
        return Err(BankError::Malformed(format!("{what} has no amount")));
    }
    // MT940 writes `1000,00`; the domain parses a dot decimal and nothing else.
    let dotted = number.replace(',', ".");
    Money::parse_fixed(&dotted, currency)
        .map(Money::micros)
        .map_err(|_| BankError::Malformed(format!("{what} has an amount that is not a number: {number:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = ":20:BONK-2026-02
:25:NL91ABNA0417164300
:28C:1/1
:60F:C260201EUR1000,00
:61:2602200220C121,00NTRFNONREF//BANK-1
:86:Betaling factuur 2026-0001
:61:260225D60,50NTRFNONREF//BANK-2
:86:Bankkosten februari
:62F:C260228EUR1060,50
";

    #[test]
    fn reads_a_statement_with_its_balances_and_entries() {
        let statements = read_mt940(SAMPLE).unwrap();
        assert_eq!(statements.len(), 1, "one file is one statement");
        let statement = &statements[0];

        assert_eq!(statement.format, "mt940");
        assert_eq!(statement.currency, Currency::EUR);
        assert_eq!(statement.opening_balance_micros, Some(1_000_000_000));
        assert_eq!(statement.closing_balance_micros, Some(1_060_500_000));
        assert_eq!(statement.period_start.unwrap().to_iso_string(), "2026-02-01");
        assert_eq!(statement.period_end.unwrap().to_iso_string(), "2026-02-28");

        assert_eq!(statement.transactions.len(), 2);
        let first = &statement.transactions[0];
        assert_eq!(first.booking_date.to_iso_string(), "2026-02-20");
        assert_eq!(first.value_date.unwrap().to_iso_string(), "2026-02-20");
        assert_eq!(first.amount_micros, 121_000_000, "a credit is money in");
        assert_eq!(first.remittance_info.as_deref(), Some("Betaling factuur 2026-0001"));
        assert_eq!(first.external_ref.as_deref(), Some("BANK-1"));
        assert_eq!(first.counterparty_name, None);
        assert_eq!(first.counterparty_iban, None);

        let second = &statement.transactions[1];
        assert_eq!(second.amount_micros, -60_500_000, "a debit is money out");
        assert_eq!(second.remittance_info.as_deref(), Some("Bankkosten februari"));
        assert_eq!(second.value_date, None, "no entry date was given");
    }

    #[test]
    fn the_movements_reconcile_with_the_balances() {
        let statement = &read_mt940(SAMPLE).unwrap()[0];
        let movements: i64 = statement.transactions.iter().map(|line| line.amount_micros).sum();
        let opening = statement.opening_balance_micros.unwrap();
        let closing = statement.closing_balance_micros.unwrap();
        assert_eq!(closing - opening, movements);
    }

    #[test]
    fn remittance_information_keeps_its_continuation_lines() {
        let file = ":60F:C260201EUR0,00
:61:260220C10,00NTRF
:86:/IBAN/NL02RABO0123456789
/NAME/Klant BV
/REMI/Factuur 2026-0001
:62F:C260228EUR10,00
";
        let statement = &read_mt940(file).unwrap()[0];
        assert_eq!(
            statement.transactions[0].remittance_info.as_deref(),
            Some("/IBAN/NL02RABO0123456789 /NAME/Klant BV /REMI/Factuur 2026-0001"),
            "the text is kept whole so the matcher can still read it"
        );
    }

    #[test]
    fn a_crlf_file_reads_like_an_lf_one() {
        let crlf = SAMPLE.replace('\n', "\r\n");
        assert_eq!(read_mt940(&crlf).unwrap(), read_mt940(SAMPLE).unwrap());
    }

    #[test]
    fn a_file_without_an_opening_balance_is_refused() {
        let error = read_mt940(":20:X\n:62F:C260228EUR0,00\n").unwrap_err();
        assert!(matches!(error, BankError::Missing(_)), "{error}");
        assert!(error.to_string().contains(":60F:"), "{error}");
    }

    #[test]
    fn a_statement_with_no_bookings_is_accepted() {
        let statements = read_mt940(":60F:C260201EUR500,00\n:62F:C260228EUR500,00\n").unwrap();
        assert!(statements[0].transactions.is_empty());
        assert_eq!(statements[0].opening_balance_micros, Some(500_000_000));
    }

    #[test]
    fn an_amount_that_is_not_a_number_is_named() {
        let error = read_mt940(":60F:C260201EUR1000,00\n:61:260220C12,3,4NTRF\n").unwrap_err();
        assert!(error.to_string().contains("not a number"), "{error}");
    }

    #[test]
    fn a_booking_without_a_debit_credit_mark_is_refused() {
        let error = read_mt940(":60F:C260201EUR1000,00\n:61:260220121,00NTRF\n").unwrap_err();
        assert!(error.to_string().contains("debit/credit mark"), "{error}");
    }

    #[test]
    fn a_reversal_takes_the_money_the_other_way() {
        let file = ":60F:C260201EUR1000,00
:61:260220RC121,00NTRF
:61:260221RD60,50NTRF
:62F:C260228EUR939,50
";
        let statement = &read_mt940(file).unwrap()[0];
        assert_eq!(statement.transactions[0].amount_micros, -121_000_000);
        assert_eq!(statement.transactions[1].amount_micros, 60_500_000);
    }

    #[test]
    fn a_line_that_does_not_open_a_tag_is_skipped() {
        // SWIFT block framing and the trailer carry no field of their own.
        let framed = format!("{{1:F01ABNANL2AXXX0000000000}}{{2:I940}}{{4:\n{SAMPLE}-}}\n");
        assert_eq!(read_mt940(&framed).unwrap(), read_mt940(SAMPLE).unwrap());
    }

    #[test]
    fn a_tag_that_never_closes_is_refused() {
        let error = read_mt940(":60F\n:60F:C260201EUR1,00\n").unwrap_err();
        assert!(matches!(error, BankError::Malformed(_)), "{error}");
    }

    #[test]
    fn an_oversized_file_is_refused_before_it_is_read() {
        let huge = "x".repeat(MAX_DOCUMENT_BYTES + 1);
        assert_eq!(read_mt940(&huge).unwrap_err(), BankError::TooLarge);
    }
}
