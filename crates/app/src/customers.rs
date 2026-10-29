//! Customer interchange: a plain CSV the window can import and export.
//!
//! The file is deliberately boring: one header row, one customer per row, and
//! the same columns the customer form and the party table carry. A customer that
//! is already on file — by VAT identifier, Peppol endpoint, or legal name and
//! city — is skipped rather than duplicated, because the archive treats a party
//! as an identity, not a list entry.

use std::collections::HashSet;

use crux::{Address, CountryCode, CustomerId, Party};
use store::{CustomerRecord, StoreError, insert_customer, list_customers};

const HEADER: &str = "name,street,postal,city,country,vat,kvk,peppol,contact,email";

/// One row read from a customer file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Row {
    /// Line in the file, for a useful error.
    pub line: usize,
    /// Legal name.
    pub name: String,
    /// Street and number.
    pub street: String,
    /// Postal code.
    pub postal: String,
    /// City.
    pub city: String,
    /// ISO 3166-1 alpha-2 country.
    pub country: String,
    /// VAT identifier.
    pub vat: String,
    /// Dutch chamber of commerce number.
    pub kvk: String,
    /// Peppol endpoint, `EAS:value`.
    pub peppol: String,
    /// Contact person.
    pub contact: String,
    /// Contact email.
    pub email: String,
}

/// What an import did.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Report {
    /// Customers written to the archive.
    pub imported: usize,
    /// Rows that were already on file.
    pub skipped: usize,
    /// Rows that could not be read, with the line and the reason.
    pub errors: Vec<String>,
}

/// Serialises the party table as CSV.
pub fn export(customers: &[CustomerRecord]) -> String {
    let mut out = String::with_capacity(HEADER.len() + customers.len() * 96);
    out.push_str(HEADER);
    out.push('\n');
    for customer in customers {
        let party = &customer.party;
        let fields = [
            party.legal_name.as_str(),
            party.address.street.as_deref().unwrap_or(""),
            party.address.postal_code.as_deref().unwrap_or(""),
            party.address.city.as_str(),
            party.address.country.as_str(),
            party.vat_id.as_deref().unwrap_or(""),
            party.kvk_number.as_deref().unwrap_or(""),
            party.peppol_id.as_deref().unwrap_or(""),
            party.contact_name.as_deref().unwrap_or(""),
            party.email.as_deref().unwrap_or(""),
        ];
        let line: Vec<String> = fields.iter().map(|field| quote(field)).collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

/// Reads a customer file.
pub fn parse(text: &str) -> Result<Vec<Row>, String> {
    let records = records(text)?;
    let Some((_, header)) = records.first() else {
        return Err("the file is empty".to_owned());
    };
    let header: Vec<String> = header.iter().map(|cell| cell.trim().to_ascii_lowercase()).collect();
    let column = |names: &[&str]| header.iter().position(|cell| names.contains(&cell.as_str()));

    let name =
        column(&["name", "legal_name", "legal name"]).ok_or_else(|| "the header has no name column".to_owned())?;
    let street = column(&["street", "address"]);
    let postal = column(&["postal", "postal_code", "post code", "zip"]);
    let city = column(&["city", "town"]);
    let country = column(&["country", "country_code"]);
    let vat = column(&["vat", "vat_id", "vat number"]);
    let kvk = column(&["kvk", "kvk_number", "coc", "chamber of commerce"]);
    let peppol = column(&["peppol", "peppol_id", "endpoint"]);
    let contact = column(&["contact", "contact_name"]);
    let email = column(&["email", "e-mail"]);

    let mut rows = Vec::new();
    for (line, fields) in records.into_iter().skip(1) {
        if fields.iter().all(|field| field.trim().is_empty()) {
            continue;
        }
        let get = |index: Option<usize>| {
            index
                .and_then(|index| fields.get(index))
                .map(|value| value.trim().to_owned())
                .unwrap_or_default()
        };
        rows.push(Row {
            line,
            name: get(Some(name)),
            street: get(street),
            postal: get(postal),
            city: get(city),
            country: {
                let value = get(country);
                if value.is_empty() {
                    "NL".to_owned()
                } else {
                    value.to_ascii_uppercase()
                }
            },
            vat: get(vat).to_ascii_uppercase(),
            kvk: get(kvk),
            peppol: get(peppol),
            contact: get(contact),
            email: get(email),
        });
    }
    Ok(rows)
}

/// Writes rows into the archive, skipping parties already on file.
pub fn import(database: &store::Database, rows: &[Row], now: &str) -> Result<Report, StoreError> {
    let connection = database.connection();
    let existing = list_customers(connection)?;
    let mut seen = Seen::default();
    for customer in &existing {
        seen.remember(&customer.party);
    }

    let mut report = Report::default();
    for row in rows {
        if row.name.is_empty() {
            report
                .errors
                .push(format!("line {}: a customer needs a name", row.line));
            continue;
        }
        if row.city.is_empty() {
            report
                .errors
                .push(format!("line {}: a customer needs a city", row.line));
            continue;
        }
        let country = match CountryCode::parse(&row.country) {
            Ok(country) => country,
            Err(error) => {
                report.errors.push(format!("line {}: {error}", row.line));
                continue;
            }
        };
        if seen.contains(row) {
            report.skipped += 1;
            continue;
        }

        let mut party = Party::new(
            row.name.as_str(),
            Address {
                street: nonempty(&row.street),
                city: row.city.as_str().to_owned(),
                postal_code: nonempty(&row.postal),
                country,
            },
        );
        party.vat_id = nonempty(&row.vat).map(|value| value.to_ascii_uppercase());
        party.kvk_number = nonempty(&row.kvk);
        party.peppol_id = nonempty(&row.peppol);
        party.contact_name = nonempty(&row.contact);
        party.email = nonempty(&row.email);

        insert_customer(
            connection,
            &CustomerRecord {
                id: CustomerId::new(),
                party: party.clone(),
            },
            now,
        )?;
        seen.remember(&party);
        report.imported += 1;
    }
    Ok(report)
}

/// The keys that decide whether a party is already on file.
#[derive(Default)]
struct Seen {
    vats: HashSet<String>,
    endpoints: HashSet<String>,
    names: HashSet<(String, String)>,
}

impl Seen {
    fn remember(&mut self, party: &Party) {
        if let Some(vat) = normalise(party.vat_id.as_deref()) {
            self.vats.insert(vat);
        }
        if let Some(endpoint) = normalise(party.peppol_id.as_deref()) {
            self.endpoints.insert(endpoint);
        }
        self.names.insert((
            normalise(Some(party.legal_name.as_str())).unwrap_or_default(),
            normalise(Some(party.address.city.as_str())).unwrap_or_default(),
        ));
    }

    fn contains(&self, row: &Row) -> bool {
        if let Some(vat) = normalise(Some(row.vat.as_str()))
            && self.vats.contains(&vat)
        {
            return true;
        }
        if let Some(endpoint) = normalise(Some(row.peppol.as_str()))
            && self.endpoints.contains(&endpoint)
        {
            return true;
        }
        self.names.contains(&(
            normalise(Some(row.name.as_str())).unwrap_or_default(),
            normalise(Some(row.city.as_str())).unwrap_or_default(),
        ))
    }
}

fn normalise(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_lowercase();
    if value.is_empty() { None } else { Some(value) }
}

fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() { None } else { Some(value.to_owned()) }
}

fn quote(value: &str) -> String {
    if value
        .chars()
        .any(|character| matches!(character, ',' | '"' | '\n' | '\r'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

/// Splits CSV text into records, keeping the line each record started on.
fn records(text: &str) -> Result<Vec<(usize, Vec<String>)>, String> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut line = 1_usize;
    let mut started_on = 1_usize;
    let mut characters = text.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '"' if quoted => {
                if characters.peek() == Some(&'"') {
                    characters.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            }
            '"' if field.is_empty() && !quoted => quoted = true,
            ',' if !quoted => record.push(std::mem::take(&mut field)),
            '\r' if !quoted => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                line += 1;
                record.push(std::mem::take(&mut field));
                records.push((started_on, std::mem::take(&mut record)));
                started_on = line;
            }
            '\n' if !quoted => {
                line += 1;
                record.push(std::mem::take(&mut field));
                records.push((started_on, std::mem::take(&mut record)));
                started_on = line;
            }
            other => field.push(other),
        }
    }
    if quoted {
        return Err(format!("line {started_on}: the closing quote is missing"));
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push((started_on, record));
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::{Database, KdfParams, KeyStore};

    const NOW: &str = "2026-10-28T12:00:00Z";

    fn customer(name: &str, city: &str, vat: Option<&str>) -> CustomerRecord {
        let mut party = Party::new(name, Address::new(city, CountryCode::NL));
        party.address.street = Some("Main 1".to_owned());
        party.address.postal_code = Some("1000 AA".to_owned());
        party.vat_id = vat.map(str::to_owned);
        party.kvk_number = Some("12345678".to_owned());
        party.peppol_id = Some("0106:12345678".to_owned());
        party.contact_name = Some("Sam".to_owned());
        party.email = Some("sam@example.test".to_owned());
        CustomerRecord {
            id: CustomerId::new(),
            party,
        }
    }

    fn database() -> (tempfile::TempDir, Database) {
        let directory = tempfile::TempDir::new().unwrap();
        let keystore = KeyStore::create(directory.path().join("keystore.bin"), "pass", KdfParams::TEST).unwrap();
        let database = Database::create(directory.path().join("bonk.db"), &keystore).unwrap();
        (directory, database)
    }

    #[test]
    fn export_then_parse_round_trips() {
        let awkward = customer("Smith, \"Sons\" & Co", "Bergen op Zoom", Some("NL123456782B01"));
        let plain = customer("Plain BV", "Delft", None);
        let text = export(&[awkward.clone(), plain.clone()]);
        let rows = parse(&text).unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name, awkward.party.legal_name);
        assert_eq!(rows[0].city, "Bergen op Zoom");
        assert_eq!(rows[0].vat, "NL123456782B01");
        assert_eq!(rows[0].kvk, "12345678");
        assert_eq!(rows[0].peppol, "0106:12345678");
        assert_eq!(rows[0].email, "sam@example.test");
        assert_eq!(rows[1].name, "Plain BV");
        assert!(rows[1].vat.is_empty());
    }

    #[test]
    fn import_adds_new_parties_and_skips_duplicates() {
        let (_directory, database) = database();
        let existing = customer("Acme BV", "Rotterdam", Some("NL123456782B01"));
        insert_customer(database.connection(), &existing, NOW).unwrap();

        let text = format!(
            "{HEADER}\n\"Acme BV\",Main 1,1000 AA,Rotterdam,NL,NL123456782B01,12345678,,\nNew BV,Main 2,2000 BB,Utrecht,NL,NL123456788B01,,,Sam,sam@new.test\nBad BV,Main 3,3000 CC,Nowhere,Netherlands,,,,,\n"
        );
        let rows = parse(&text).unwrap();
        assert_eq!(rows.len(), 3);

        let report = import(&database, &rows, NOW).unwrap();
        assert_eq!(report.imported, 1);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].contains("line 4"), "{report:?}");

        let customers = list_customers(database.connection()).unwrap();
        assert_eq!(customers.len(), 2);
        assert!(customers.iter().any(|customer| customer.party.legal_name == "New BV"));
    }

    #[test]
    fn a_missing_header_or_quote_is_explained() {
        assert!(parse("").unwrap_err().contains("empty"));
        assert!(parse("street,city\nMain,Delft\n").unwrap_err().contains("name"));
        assert!(parse(&format!("{HEADER}\n\"Broken,Main,1000 AA,Delft,NL,,,,\n")).is_err());
    }

    #[test]
    fn a_country_defaults_to_the_netherlands() {
        let rows = parse("name,city\nSolo BV,Delft\n").unwrap();
        assert_eq!(rows[0].country, "NL");
    }
}
