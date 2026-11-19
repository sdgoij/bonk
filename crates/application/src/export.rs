//! Exports of document lists for people and other systems.
//!
//! CSV is for spreadsheets and accountants, JSON for tooling. Both are
//! deterministic for the same rows: an export whose shape changes between runs
//! is not an export.
//!
//! Amounts are written as decimal strings, never as JSON numbers. A JSON number
//! is a float to most parsers, and money must not round-trip through a float.

use store::{SearchHit, StoreError, vat::IcpListing};

/// Header of the CSV export.
pub const CSV_HEADER: &str = "number,direction,party,issue_date,currency,gross,status";

/// Renders rows as RFC 4180 CSV with `\n` line endings.
pub fn to_csv(hits: &[SearchHit]) -> Result<String, StoreError> {
    let mut out = String::from(CSV_HEADER);
    out.push('\n');
    for hit in hits {
        let row = [
            hit.number.as_str(),
            hit.direction.code(),
            hit.party.as_str(),
            &hit.issue_date.to_iso_string(),
            hit.currency.as_str(),
            &hit.gross
                .to_fixed_string(2)
                .map_err(|error| StoreError::Data(error.to_string()))?,
            hit.status.code(),
        ];
        for (index, field) in row.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            push_csv_field(&mut out, field);
        }
        out.push('\n');
    }
    Ok(out)
}

/// Header of the ICP listing CSV.
pub const ICP_CSV_HEADER: &str = "period,country,vat_id,customer,supplies";

/// Renders the ICP listing as RFC 4180 CSV with `\n` line endings.
pub fn icp_csv(listing: &IcpListing) -> Result<String, StoreError> {
    let mut out = String::from(ICP_CSV_HEADER);
    out.push('\n');
    let period = format!("{}-{:02}", listing.year, listing.period_no);
    for row in &listing.rows {
        let supplies = crux::Money::from_micros(row.supplies_micros, listing.currency)
            .to_fixed_string(2)
            .map_err(|error| StoreError::Data(error.to_string()))?;
        let fields = [
            period.as_str(),
            row.country.as_str(),
            row.vat_id.as_str(),
            row.customer.as_str(),
            supplies.as_str(),
        ];
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            push_csv_field(&mut out, field);
        }
        out.push('\n');
    }
    Ok(out)
}

/// Renders rows as a pretty-printed JSON array, keys in a fixed order.
pub fn to_json(hits: &[SearchHit]) -> Result<String, StoreError> {
    let mut rows = Vec::with_capacity(hits.len());
    for hit in hits {
        rows.push(serde_json::json!({
            "number": hit.number,
            "direction": hit.direction.code(),
            "party": hit.party,
            "issue_date": hit.issue_date.to_iso_string(),
            "currency": hit.currency.as_str(),
            "gross": hit.gross.to_fixed_string(2).map_err(|error| StoreError::Data(error.to_string()))?,
            "status": hit.status.code(),
            "id": hit.id.to_string(),
        }));
    }
    serde_json::to_string_pretty(&rows).map_err(|error| StoreError::Data(error.to_string()))
}

/// Quotes a field only when it needs it, doubling inner quotes.
fn push_csv_field(out: &mut String, field: &str) {
    let needs_quotes = field.contains([',', '"', '\n', '\r']);
    if !needs_quotes {
        out.push_str(field);
        return;
    }
    out.push('"');
    for character in field.chars() {
        if character == '"' {
            out.push('"');
        }
        out.push(character);
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use crux::{Currency, DocumentId, LocalDate, Money};
    use store::vat::{IcpListing, IcpRow};
    use store::{Direction, DocumentStatus, SearchHit};

    use super::*;

    fn hit(number: &str, party: &str, gross: i64, status: DocumentStatus) -> SearchHit {
        SearchHit {
            id: DocumentId::new(),
            direction: Direction::Outbound,
            number: number.to_owned(),
            party: party.to_owned(),
            issue_date: LocalDate::parse_iso("2026-10-27").expect("date"),
            currency: Currency::EUR,
            gross: Money::from_micros(gross, Currency::EUR),
            status,
            matched: true,
        }
    }

    #[test]
    fn csv_has_a_header_and_one_row_per_document() {
        let rows = [
            hit("2026-0001", "Customer BV", 121_000_000, DocumentStatus::Approved),
            hit("2026-0002", "Café Zuid BV", 50_000_000, DocumentStatus::Received),
        ];
        let csv = to_csv(&rows).expect("render");

        let mut lines = csv.lines();
        assert_eq!(lines.next(), Some(CSV_HEADER));
        assert_eq!(
            lines.next(),
            Some("2026-0001,outbound,Customer BV,2026-10-27,EUR,121.00,approved")
        );
        assert_eq!(
            lines.next(),
            Some("2026-0002,outbound,Café Zuid BV,2026-10-27,EUR,50.00,received")
        );
        assert_eq!(lines.next(), None);
    }

    #[test]
    fn csv_quotes_what_would_break_the_columns() {
        let rows = [hit(
            "2026-0003",
            "Acme, Inc \"West\"",
            1_000_000,
            DocumentStatus::Received,
        )];
        let csv = to_csv(&rows).expect("render");
        assert!(
            csv.contains("\"Acme, Inc \"\"West\"\"\""),
            "commas and quotes are escaped: {csv}"
        );
        assert_eq!(csv.lines().count(), 2, "an escaped comma is still one row");
    }

    #[test]
    fn csv_of_nothing_is_just_the_header() {
        assert_eq!(to_csv(&[]).expect("render"), format!("{CSV_HEADER}\n"));
    }

    #[test]
    fn the_icp_csv_is_one_row_per_buyer_and_keeps_a_negative_credit() {
        let listing = IcpListing {
            currency: Currency::EUR,
            year: 2026,
            period_no: 1,
            rows: vec![
                IcpRow {
                    country: "DE".to_owned(),
                    vat_id: "DE123456789".to_owned(),
                    customer: "Kunde, GmbH".to_owned(),
                    supplies_micros: 1_000_000_000,
                },
                IcpRow {
                    country: "FR".to_owned(),
                    vat_id: "FR12345678901".to_owned(),
                    customer: "Client SA".to_owned(),
                    supplies_micros: -250_000_000,
                },
            ],
            problems: Vec::new(),
            basis: Vec::new(),
        };
        let csv = icp_csv(&listing).expect("render");

        let mut lines = csv.lines();
        assert_eq!(lines.next(), Some(ICP_CSV_HEADER));
        assert_eq!(lines.next(), Some("2026-01,DE,DE123456789,\"Kunde, GmbH\",1000.00"));
        assert_eq!(lines.next(), Some("2026-01,FR,FR12345678901,Client SA,-250.00"));
        assert_eq!(lines.next(), None);
    }

    #[test]
    fn json_keeps_amounts_as_strings() {
        let rows = [hit("2026-0001", "Customer BV", 121_000_000, DocumentStatus::Paid)];
        let json = to_json(&rows).expect("render");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        let first = &parsed[0];
        assert_eq!(first["number"], "2026-0001");
        assert_eq!(first["direction"], "outbound");
        assert_eq!(first["party"], "Customer BV");
        assert_eq!(first["issue_date"], "2026-10-27");
        assert_eq!(first["currency"], "EUR");
        assert_eq!(first["gross"].as_str(), Some("121.00"), "a string, not a float");
        assert_eq!(first["status"], "paid");
    }

    #[test]
    fn json_of_nothing_is_an_empty_array() {
        assert_eq!(to_json(&[]).expect("render"), "[]");
    }
}
