//! Tier-1 validation: coherence rules that do not need the EN 16931 codelists.
//!
//! These run on every keystroke, so they must be cheap and unambiguous. Codes
//! are stable; the business-term references are what the UI shows.

use crate::findings::Finding;
use crate::invoice::InvoiceDraft;
use crate::party::Party;
use crate::totals::calculate_totals;

/// Validates a draft against the parties that will appear on it.
pub fn validate_draft(draft: &InvoiceDraft, seller: &Party, customer: &Party) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(seller.validate("seller"));
    findings.extend(customer.validate("customer"));

    if draft.lines.is_empty() {
        findings.push(Finding::blocking(
            "T1-001",
            Some("BG-25"),
            "lines",
            "at least one invoice line is required",
        ));
    }

    let mut positions = std::collections::BTreeSet::new();
    for (index, line) in draft.lines.iter().enumerate() {
        let field = |suffix: &str| format!("lines[{index}].{suffix}");

        if !positions.insert(line.position) {
            findings.push(Finding::blocking(
                "T1-018",
                None,
                field("position"),
                "line positions must be unique",
            ));
        }
        if line.name.trim().is_empty() {
            findings.push(Finding::blocking(
                "T1-002",
                Some("BT-153"),
                field("name"),
                "item name is required",
            ));
        }
        if line.quantity.micros() <= 0 {
            findings.push(Finding::blocking(
                "T1-003",
                Some("BT-129"),
                field("quantity"),
                "quantity must be greater than zero",
            ));
        }
        if line.price_base_quantity.micros() <= 0 {
            findings.push(Finding::blocking(
                "T1-004",
                Some("BT-149"),
                field("price_base_quantity"),
                "price base quantity must be greater than zero",
            ));
        }
        if line.unit_price.is_negative() {
            findings.push(Finding::blocking(
                "T1-005",
                Some("BT-146"),
                field("unit_price"),
                "unit price must not be negative; use a credit note",
            ));
        }
        if line.unit_price.currency() != draft.currency {
            findings.push(Finding::blocking(
                "T1-019",
                Some("BT-5"),
                field("unit_price"),
                "line currency differs from the document currency",
            ));
        }
        if let Err(error) = line.treatment.validate() {
            findings.push(Finding::blocking(
                "T1-006",
                Some("BT-118"),
                field("vat_category"),
                error.to_string(),
            ));
        }
        if line.treatment.needs_exemption_reason()
            && line.treatment.exemption_reason.is_none()
            && line.treatment.exemption_reason_code.is_none()
        {
            findings.push(Finding::warning(
                "T1-013",
                Some("BT-120"),
                field("exemption_reason"),
                "an exemption reason is expected for this VAT category",
            ));
        }
        match line.net() {
            Ok(net) if net.is_zero() => findings.push(Finding::warning(
                "T1-015",
                Some("BT-131"),
                field("unit_price"),
                "line amount rounds to zero",
            )),
            Ok(_) => {}
            Err(error) => findings.push(Finding::blocking(
                "T1-020",
                Some("BT-131"),
                field("quantity"),
                error.to_string(),
            )),
        }
    }

    if let Some(due) = draft.due_date
        && due < draft.issue_date
    {
        findings.push(Finding::blocking(
            "T1-007",
            Some("BT-9"),
            "due_date",
            "due date is before the issue date",
        ));
    }

    if !draft.lines.is_empty() {
        match calculate_totals(&draft.lines) {
            Ok(totals) => {
                if totals.gross.is_zero() {
                    findings.push(Finding::warning(
                        "T1-014",
                        Some("BT-112"),
                        "lines",
                        "document total is zero",
                    ));
                }
            }
            Err(error) => findings.push(Finding::blocking("T1-001", Some("BT-106"), "lines", error.to_string())),
        }
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::date::LocalDate;
    use crate::ids::{CustomerId, LineId, SellerId};
    use crate::invoice::{DraftLine, InvoiceDraft};
    use crate::money::{Currency, Money};
    use crate::party::{Address, CountryCode, Party};
    use crate::quantity::{Quantity, UnitCode};
    use crate::vat::{TaxTreatment, VatRate};

    fn party(name: &str, vat: Option<&str>) -> Party {
        let mut party = Party::new(name, Address::new("Amsterdam", CountryCode::NL));
        party.vat_id = vat.map(str::to_owned);
        party
    }

    fn line(name: &str, net_minor: i64, rate: u16) -> DraftLine {
        DraftLine {
            id: LineId::new(),
            position: 1,
            name: name.to_owned(),
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_minor_units(net_minor, Currency::EUR).unwrap(),
            price_base_quantity: Quantity::ONE,
            treatment: TaxTreatment::standard(VatRate::from_basis_points(rate).unwrap()),
        }
    }

    fn draft(lines: Vec<DraftLine>) -> InvoiceDraft {
        let mut draft = InvoiceDraft::new(
            SellerId::new(),
            CustomerId::new(),
            Currency::EUR,
            LocalDate::parse_iso("2026-10-27").unwrap(),
        );
        draft.lines = lines;
        draft
    }

    fn blocking_count(findings: &[Finding]) -> usize {
        findings
            .iter()
            .filter(|finding| finding.severity == crate::findings::Severity::Blocking)
            .count()
    }

    #[test]
    fn a_clean_draft_has_no_blocking_findings() {
        let findings = validate_draft(
            &draft(vec![line("Consulting", 10_000, 2100)]),
            &party("Seller BV", Some("NL123456782B01")),
            &party("Customer BV", None),
        );
        assert_eq!(blocking_count(&findings), 0, "{findings:#?}");
    }

    #[test]
    fn an_empty_draft_is_blocking() {
        let findings = validate_draft(
            &draft(Vec::new()),
            &party("Seller BV", None),
            &party("Customer BV", None),
        );
        assert!(findings.iter().any(|finding| finding.code == "T1-001"));
        assert_eq!(blocking_count(&findings), 1);
    }

    #[test]
    fn bad_line_fields_are_found_per_line() {
        let mut bad = line(" ", 0, 2100);
        bad.quantity = Quantity::ZERO;
        bad.price_base_quantity = Quantity::ZERO;
        bad.unit_price = Money::from_major_units(-1, Currency::EUR).unwrap();
        let findings = validate_draft(
            &draft(vec![bad]),
            &party("Seller BV", None),
            &party("Customer BV", None),
        );
        for code in ["T1-002", "T1-003", "T1-004", "T1-005"] {
            let finding = findings
                .iter()
                .find(|finding| finding.code == code)
                .unwrap_or_else(|| panic!("missing {code}: {findings:#?}"));
            assert!(
                finding.field.starts_with("lines[0]."),
                "{code} points at {}",
                finding.field
            );
        }
    }

    #[test]
    fn due_date_before_issue_date_is_blocking() {
        let mut document = draft(vec![line("Consulting", 10_000, 2100)]);
        document.due_date = Some(LocalDate::parse_iso("2026-10-01").unwrap());
        let findings = validate_draft(&document, &party("Seller BV", None), &party("Customer BV", None));
        assert!(findings.iter().any(|finding| finding.code == "T1-007"));
    }

    #[test]
    fn malformed_vat_id_is_blocking_and_bad_checksum_is_a_warning() {
        let document = draft(vec![line("Consulting", 10_000, 2100)]);
        let malformed = validate_draft(
            &document,
            &party("Seller BV", Some("NL123")),
            &party("Customer BV", None),
        );
        assert!(malformed.iter().any(|finding| finding.code == "T1-010"));

        let checksum = validate_draft(
            &document,
            &party("Seller BV", Some("NL123456788B01")),
            &party("Customer BV", None),
        );
        let finding = checksum
            .iter()
            .find(|finding| finding.code == "T1-011")
            .expect("checksum warning");
        assert_eq!(finding.severity, crate::findings::Severity::Warning);

        let valid = validate_draft(
            &document,
            &party("Seller BV", Some("NL123456782B01")),
            &party("Customer BV", None),
        );
        assert!(!valid.iter().any(|finding| finding.code == "T1-011"));
    }

    #[test]
    fn zero_document_total_is_a_warning_not_a_block() {
        let findings = validate_draft(
            &draft(vec![line("Free sample", 0, 2100)]),
            &party("Seller BV", None),
            &party("Customer BV", None),
        );
        assert!(findings.iter().any(|finding| finding.code == "T1-014"));
        assert_eq!(blocking_count(&findings), 0);
    }

    #[test]
    fn duplicate_positions_are_blocking() {
        let mut first = line("A", 1_000, 2100);
        let mut second = line("B", 1_000, 2100);
        first.position = 1;
        second.position = 1;
        let findings = validate_draft(
            &draft(vec![first, second]),
            &party("Seller BV", None),
            &party("Customer BV", None),
        );
        assert!(findings.iter().any(|finding| finding.code == "T1-018"));
    }
}
