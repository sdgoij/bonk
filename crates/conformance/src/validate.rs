//! Core EN 16931 business rules.
//!
//! The engine deliberately covers the high-frequency arithmetic, VAT-category,
//! and code-list rules first. Official Schematron/XSLT remains the authority
//! for profiles and the remaining codelists; these checks provide immediate,
//! field-level feedback and a second line of defence before export.

use std::collections::{BTreeMap, BTreeSet};

use crux::{CountryCode, DocType, Finding, Money, Party, VatCategory, VatRate, tax_amount};

use crate::bt::Bt;
use crate::codelist::Codelist;
use crate::rule::{
    BR_02, BR_16, BR_CL_04, BR_CL_14, BR_CL_22, BR_CL_23, BR_CL_25, BR_CO_10, BR_CO_13, BR_CO_14, BR_CO_15, BR_CO_16,
    BR_CO_17, BR_CO_25, BR_CO_26, BR_IC_11, BR_O_12, PEPPOL_NL_R002, PEPPOL_R003, PEPPOL_R010, PEPPOL_R020, Rule,
    VATEX_CATEGORY_RULES, category_rules,
};
use crate::semantic::SemanticInvoice;

/// Runs the implemented EN 16931 rules over a semantic invoice.
///
/// Every finding is blocking: the caller should not issue or export while the
/// result is non-empty.
pub fn validate(invoice: &SemanticInvoice<'_>) -> Vec<Finding> {
    let mut findings = Vec::new();

    if invoice.number.trim().is_empty() {
        findings.push(BR_02.finding("number", "An invoice shall have an invoice number."));
    }

    if invoice.seller.vat_id.is_none() && invoice.seller.kvk_number.is_none() {
        findings.push(BR_CO_26.finding(
            "seller",
            "the seller needs a VAT identifier or a legal registration identifier",
        ));
    }

    validate_codelists(invoice, &mut findings);
    validate_peppol(invoice, &mut findings);

    // BR-CO-25 applies to invoices; a credit note has no payment due date.
    if invoice.doc_type == DocType::Invoice && invoice.amount_due.is_positive() && invoice.due_date.is_none() {
        findings.push(BR_CO_25.finding(
            "due_date",
            "an invoice with a positive amount due needs a payment due date",
        ));
    }

    if invoice.delivery_date.is_none()
        && invoice
            .totals
            .breakdown
            .iter()
            .any(|row| row.category == VatCategory::IntraCommunity)
    {
        findings.push(BR_IC_11.finding(
            "delivery_date",
            "intra-community supply requires an actual delivery date",
        ));
    }

    if invoice.lines.is_empty() {
        findings.push(BR_16.finding("lines", "An invoice shall have at least one invoice line."));
        return findings;
    }

    let Some(line_nets) = sum_line_nets(invoice, &mut findings) else {
        return findings;
    };

    validate_totals(invoice, line_nets, &mut findings);
    validate_breakdowns(invoice, &mut findings);
    validate_categories(invoice, &mut findings);
    validate_exemptions(invoice, &mut findings);

    findings
}

/// Checks every free-form code against its vendored code list.
///
/// Codes backed by a closed enum (`DocType`, `VatCategory`) cannot be wrong at
/// this point; their membership is asserted by the tests in `codelist` instead.
/// The checks here cover the fields a user or an import can fill with anything.
fn validate_codelists(invoice: &SemanticInvoice<'_>, findings: &mut Vec<Finding>) {
    let currency = invoice.currency.as_str();
    if !Codelist::CurrencyCode.contains(currency) {
        findings.push(BR_CL_04.finding(
            "currency",
            format!("{currency} is not an ISO 4217 alpha-3 currency code"),
        ));
    }

    party_codelists(
        invoice.seller,
        "seller",
        Bt::SellerCountry,
        Bt::SellerEndpointId,
        findings,
    );
    party_codelists(invoice.buyer, "buyer", Bt::BuyerCountry, Bt::BuyerEndpointId, findings);

    for (index, line) in invoice.lines.iter().enumerate() {
        let unit = line.unit.as_str();
        if !Codelist::UnitCode.contains(unit) {
            findings.push(BR_CL_23.finding(
                format!("lines[{index}].unit"),
                format!("{unit} is not a UN/ECE Recommendation 20 unit code"),
            ));
        }

        let reason_code = line
            .treatment
            .exemption_reason_code
            .as_deref()
            .map(str::trim)
            .filter(|code| !code.is_empty());
        if let Some(code) = reason_code
            && !Codelist::ExemptionReasonCode.contains_uppercase(code)
        {
            findings.push(BR_CL_22.finding(
                format!("lines[{index}].exemption_reason_code"),
                format!("{code} is not a CEF VATEX exemption reason code"),
            ));
        }
    }
}

/// Checks the Peppol BIS 3.0 rules that the domain model can express.
///
/// Ids come from `PEPPOL-EN16931-UBL.sch` (release 3.0.20). Rules about
/// allowances, payment mandates, or invoice periods are absent because the
/// model does not carry those groups yet; they are not silently assumed.
fn validate_peppol(invoice: &SemanticInvoice<'_>, findings: &mut Vec<Finding>) {
    if !invoice.profile.is_peppol() {
        return;
    }

    if invoice.buyer_reference.is_none() && invoice.order_reference.is_none() {
        findings.push(PEPPOL_R003.finding(
            "buyer_reference",
            "a Peppol invoice needs a buyer reference or an order reference",
        ));
    }
    endpoint_finding(&PEPPOL_R020, invoice.seller, "seller", findings);
    endpoint_finding(&PEPPOL_R010, invoice.buyer, "buyer", findings);

    if invoice.seller.address.country == CountryCode::NL
        && (invoice.seller.address.street.is_none() || invoice.seller.address.postal_code.is_none())
    {
        findings.push(PEPPOL_NL_R002.finding(
            "seller.address",
            "a Dutch seller's address needs a street name and a post code",
        ));
    }

    // NL-R-003 constrains the scheme of a legal registration identifier, not its
    // presence. The writer always emits a Dutch KvK as scheme 0106, so the rule
    // is satisfied structurally and is asserted by a syntax test instead of an
    // unreachable finding.

    for (index, line) in invoice.lines.iter().enumerate() {
        let Some(code) = line
            .treatment
            .exemption_reason_code
            .as_deref()
            .map(str::trim)
            .filter(|code| !code.is_empty())
        else {
            continue;
        };
        let code = code.to_ascii_uppercase();
        let Some((_, expected, rule)) = VATEX_CATEGORY_RULES.iter().find(|(known, _, _)| *known == code) else {
            continue;
        };
        if line.treatment.category != *expected {
            findings.push(rule.finding(
                format!("lines[{index}].vat_category"),
                format!(
                    "{code} requires VAT category {}, found {}",
                    expected.code(),
                    line.treatment.category.code()
                ),
            ));
        }
    }
}

/// Reports a missing or unusable endpoint identifier for one party.
fn endpoint_finding(rule: &Rule, party: &Party, role: &str, findings: &mut Vec<Finding>) {
    if party.peppol_endpoint().is_some() {
        return;
    }
    let message = if party.peppol_id.is_some() {
        format!("the {role}'s Peppol endpoint identifier must be written as EAS:value")
    } else {
        format!("the {role} needs a Peppol endpoint identifier")
    };
    findings.push(rule.finding(format!("{role}.peppol_id"), message));
}

fn party_codelists(party: &Party, role: &str, country_bt: Bt, endpoint_bt: Bt, findings: &mut Vec<Finding>) {
    let country = party.address.country.as_str();
    if !Codelist::CountryCode.contains(country) {
        findings.push(BR_CL_14.finding_at(
            country_bt,
            format!("{role}.address.country"),
            format!("{country} is not an ISO 3166-1 country code"),
        ));
    }

    let scheme = party.peppol_endpoint().map(|(scheme, _)| scheme);
    if let Some(scheme) = scheme
        && !Codelist::EndpointSchemeCode.contains(scheme)
    {
        findings.push(BR_CL_25.finding_at(
            endpoint_bt,
            format!("{role}.peppol_id"),
            format!("{scheme} is not a CEF EAS endpoint scheme"),
        ));
    }
}

fn sum_line_nets(invoice: &SemanticInvoice<'_>, findings: &mut Vec<Finding>) -> Option<Money> {
    let mut sum = Money::zero(invoice.currency);
    for (index, line) in invoice.lines.iter().enumerate() {
        let net = match line.net() {
            Ok(net) => net,
            Err(error) => {
                findings.push(BR_CO_10.finding(
                    format!("lines[{index}].quantity"),
                    format!("line net cannot be computed: {error}"),
                ));
                return None;
            }
        };
        if net.currency() != invoice.currency {
            findings.push(BR_CO_10.finding(
                format!("lines[{index}].unit_price"),
                "line currency does not match the invoice currency",
            ));
            return None;
        }
        sum = match sum.checked_add(net) {
            Ok(sum) => sum,
            Err(error) => {
                findings.push(BR_CO_10.finding("totals.net", format!("line net amounts cannot be summed: {error}")));
                return None;
            }
        };
    }
    Some(sum)
}

fn validate_totals(invoice: &SemanticInvoice<'_>, line_nets: Money, findings: &mut Vec<Finding>) {
    if invoice.totals.net != line_nets {
        findings.push(BR_CO_10.finding(
            "totals.net",
            format!(
                "sum of line net amounts is {}, but BT-106 is {}",
                money_text(line_nets),
                money_text(invoice.totals.net)
            ),
        ));
    }

    let expected_exclusive = invoice
        .totals
        .net
        .checked_sub(invoice.allowances)
        .and_then(|value| value.checked_add(invoice.charges));
    compare_amount(
        &BR_CO_13,
        "totals.tax_exclusive",
        "invoice total without VAT",
        invoice.tax_exclusive,
        expected_exclusive,
        findings,
    );

    let mut breakdown_tax = Money::zero(invoice.currency);
    let mut breakdown_ok = true;
    for row in &invoice.totals.breakdown {
        match breakdown_tax.checked_add(row.tax) {
            Ok(value) => breakdown_tax = value,
            Err(error) => {
                findings.push(BR_CO_14.finding(
                    "tax_breakdown",
                    format!("VAT category tax amounts cannot be summed: {error}"),
                ));
                breakdown_ok = false;
                break;
            }
        }
    }
    if breakdown_ok && invoice.totals.tax != breakdown_tax {
        findings.push(BR_CO_14.finding(
            "totals.tax",
            format!(
                "sum of VAT category tax amounts is {}, but BT-110 is {}",
                money_text(breakdown_tax),
                money_text(invoice.totals.tax)
            ),
        ));
    }

    let expected_inclusive = invoice.tax_exclusive.checked_add(invoice.totals.tax);
    compare_amount(
        &BR_CO_15,
        "totals.tax_inclusive",
        "invoice total with VAT",
        invoice.tax_inclusive,
        expected_inclusive,
        findings,
    );

    let expected_due = invoice
        .tax_inclusive
        .checked_sub(invoice.paid)
        .and_then(|value| value.checked_add(invoice.rounding));
    compare_amount(
        &BR_CO_16,
        "totals.amount_due",
        "amount due for payment",
        invoice.amount_due,
        expected_due,
        findings,
    );
}

fn validate_breakdowns(invoice: &SemanticInvoice<'_>, findings: &mut Vec<Finding>) {
    let bases = line_bases(invoice);

    for (index, row) in invoice.totals.breakdown.iter().enumerate() {
        let rate = match VatRate::from_basis_points(row.rate_bp) {
            Ok(rate) => rate,
            Err(error) => {
                findings.push(BR_CO_17.finding(
                    format!("tax_breakdown[{index}].rate"),
                    format!("invalid VAT category rate: {error}"),
                ));
                continue;
            }
        };

        let expected_tax = tax_amount(row.taxable_base, rate);
        match expected_tax {
            Ok(expected) if expected == row.tax => {}
            Ok(expected) => {
                let rule = category_rules(row.category).map_or(&BR_CO_17, |rules| rules.tax);
                findings.push(rule.finding(
                    format!("tax_breakdown[{index}].tax"),
                    format!(
                        "expected VAT category tax {}, found {}",
                        money_text(expected),
                        money_text(row.tax)
                    ),
                ));
            }
            Err(error) => {
                findings.push(BR_CO_17.finding(
                    format!("tax_breakdown[{index}].tax"),
                    format!("VAT category tax cannot be computed: {error}"),
                ));
            }
        }

        if invoice.allowances.is_zero() && invoice.charges.is_zero() {
            let Some(rules) = category_rules(row.category) else {
                continue;
            };
            let rate_key = if row.category == VatCategory::Standard {
                row.rate_bp
            } else {
                0
            };
            let expected = bases
                .get(&(row.category, rate_key))
                .copied()
                .unwrap_or_else(|| Money::zero(invoice.currency));
            if expected != row.taxable_base {
                findings.push(rules.base.finding(
                    format!("tax_breakdown[{index}].taxable_base"),
                    format!(
                        "expected taxable amount {}, found {}",
                        money_text(expected),
                        money_text(row.taxable_base)
                    ),
                ));
            }
        }
    }
}

fn line_bases(invoice: &SemanticInvoice<'_>) -> BTreeMap<(VatCategory, u16), Money> {
    let mut bases = BTreeMap::new();
    for line in invoice.lines {
        let Ok(net) = line.net() else {
            continue;
        };
        let rate = if line.treatment.category == VatCategory::Standard {
            line.treatment.rate.basis_points()
        } else {
            0
        };
        let entry = bases
            .entry((line.treatment.category, rate))
            .or_insert_with(|| Money::zero(invoice.currency));
        if let Ok(sum) = entry.checked_add(net) {
            *entry = sum;
        }
    }
    bases
}

fn validate_categories(invoice: &SemanticInvoice<'_>, findings: &mut Vec<Finding>) {
    let line_categories: BTreeSet<VatCategory> = invoice.lines.iter().map(|line| line.treatment.category).collect();
    let breakdown_categories: BTreeSet<VatCategory> = invoice.totals.breakdown.iter().map(|row| row.category).collect();

    for category in &line_categories {
        if let Some(rules) = category_rules(*category)
            && !breakdown_categories.contains(category)
        {
            findings.push(rules.presence.finding(
                "tax_breakdown",
                format!("VAT category {} has no VAT breakdown", category.code()),
            ));
        }
    }

    for (index, line) in invoice.lines.iter().enumerate() {
        let category = line.treatment.category;
        let Some(rules) = category_rules(category) else {
            continue;
        };
        let invalid = if category == VatCategory::Standard {
            line.treatment.rate.is_zero()
        } else {
            !line.treatment.rate.is_zero()
        };
        if invalid {
            findings.push(rules.rate.finding(
                format!("lines[{index}].rate"),
                if category == VatCategory::Standard {
                    "a standard-rated line must have a VAT rate greater than zero"
                } else {
                    "this VAT category requires a zero VAT rate"
                },
            ));
        }
    }

    if line_categories.contains(&VatCategory::OutsideScope) && line_categories.len() > 1 {
        findings.push(BR_O_12.finding(
            "lines",
            "an invoice with a not-subject-to-VAT line cannot mix other VAT categories",
        ));
    }
}

fn validate_exemptions(invoice: &SemanticInvoice<'_>, findings: &mut Vec<Finding>) {
    for (index, row) in invoice.totals.breakdown.iter().enumerate() {
        let Some(rules) = category_rules(row.category) else {
            continue;
        };
        let has_reason = invoice
            .lines
            .iter()
            .filter(|line| line.treatment.category == row.category)
            .any(|line| {
                line.treatment
                    .exemption_reason
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty())
                    || line
                        .treatment
                        .exemption_reason_code
                        .as_deref()
                        .is_some_and(|code| !code.trim().is_empty())
            });

        if rules.exemption_required && !has_reason {
            findings.push(rules.exemption.finding(
                format!("tax_breakdown[{index}].exemption_reason"),
                "this VAT category requires an exemption reason",
            ));
        } else if !rules.exemption_required && has_reason {
            findings.push(rules.exemption.finding(
                format!("tax_breakdown[{index}].exemption_reason"),
                "this VAT category must not have an exemption reason",
            ));
        }
    }
}

fn compare_amount(
    rule: &Rule,
    field: &str,
    description: &str,
    actual: Money,
    expected: Result<Money, crux::DomainError>,
    findings: &mut Vec<Finding>,
) {
    match expected {
        Ok(expected) if expected == actual => {}
        Ok(expected) => findings.push(rule.finding(
            field,
            format!(
                "{description}: expected {}, found {}",
                money_text(expected),
                money_text(actual)
            ),
        )),
        Err(error) => findings.push(rule.finding(field, format!("{description} cannot be computed: {error}"))),
    }
}

fn money_text(value: Money) -> String {
    value.to_fixed_string(2).unwrap_or_else(|_| value.micros().to_string())
}

#[cfg(test)]
mod tests {
    use crux::{
        Address, CountryCode, Currency, CustomerId, DraftLine, InvoiceDraft, InvoiceTotals, LineId, LocalDate, Money,
        Party, Quantity, SellerId, TaxTreatment, UnitCode, VatCategory, VatRate, calculate_totals,
    };

    use super::*;
    use crate::profile::Profile;

    fn party(name: &str) -> Party {
        Party::new(name, Address::new("Amsterdam", CountryCode::NL))
    }

    fn seller() -> Party {
        let mut party = party("Seller BV");
        party.vat_id = Some("NL123456782B01".to_owned());
        party.kvk_number = Some("12345678".to_owned());
        party
    }

    fn line(position: u32, major: i64, treatment: TaxTreatment) -> DraftLine {
        DraftLine {
            id: LineId::new(),
            position,
            name: format!("Line {position}"),
            description: None,
            quantity: Quantity::ONE,
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(major, Currency::EUR).unwrap(),
            price_base_quantity: Quantity::ONE,
            treatment,
        }
    }

    fn draft(lines: Vec<DraftLine>) -> InvoiceDraft {
        let mut draft = InvoiceDraft::new(
            SellerId::new(),
            CustomerId::new(),
            Currency::EUR,
            LocalDate::parse_iso("2026-10-27").unwrap(),
        );
        draft.due_date = Some(LocalDate::parse_iso("2026-11-26").unwrap());
        draft.lines = lines;
        draft
    }

    fn semantic_invoice<'a>(
        number: &'a str,
        draft: &'a InvoiceDraft,
        seller: &'a Party,
        buyer: &'a Party,
    ) -> SemanticInvoice<'a> {
        let totals = calculate_totals(&draft.lines).unwrap();
        SemanticInvoice::from_draft(number, draft, seller, buyer, totals, Profile::En16931)
    }

    fn peppol_invoice<'a>(
        number: &'a str,
        draft: &'a InvoiceDraft,
        seller: &'a Party,
        buyer: &'a Party,
    ) -> SemanticInvoice<'a> {
        let totals = calculate_totals(&draft.lines).unwrap();
        SemanticInvoice::from_draft(number, draft, seller, buyer, totals, Profile::PeppolBis3)
    }

    fn complete_party(name: &str, endpoint: &str) -> Party {
        let mut party = Party::new(
            name,
            Address {
                street: Some("Main 1".to_owned()),
                city: "Amsterdam".to_owned(),
                postal_code: Some("1000 AA".to_owned()),
                country: CountryCode::NL,
            },
        );
        party.vat_id = Some("NL123456782B01".to_owned());
        party.kvk_number = Some("12345678".to_owned());
        party.peppol_id = Some(endpoint.to_owned());
        party
    }

    fn has(findings: &[Finding], code: &str) -> bool {
        findings.iter().any(|finding| finding.code == code)
    }

    fn finding<'a>(findings: &'a [Finding], code: &str) -> &'a Finding {
        findings
            .iter()
            .find(|finding| finding.code == code)
            .unwrap_or_else(|| panic!("{code} was not emitted: {findings:?}"))
    }

    #[test]
    fn a_consistent_standard_invoice_is_clean() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn invoice_number_is_required() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        let invoice = semantic_invoice("", &draft, &seller, &buyer);

        assert!(has(&validate(&invoice), "BR-02"));
    }

    #[test]
    fn seller_identifier_is_required() {
        let buyer = party("Buyer BV");
        let draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        let unidentified = party("Seller BV");
        let invoice = semantic_invoice("2026-0001", &draft, &unidentified, &buyer);
        assert!(has(&validate(&invoice), "BR-CO-26"));

        let mut identified = party("Seller BV");
        identified.kvk_number = Some("12345678".to_owned());
        let invoice = semantic_invoice("2026-0001", &draft, &identified, &buyer);
        assert!(!has(&validate(&invoice), "BR-CO-26"));
    }

    #[test]
    fn an_empty_invoice_stops_at_br_16() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let draft = draft(Vec::new());
        let totals = InvoiceTotals {
            net: Money::zero(Currency::EUR),
            tax: Money::zero(Currency::EUR),
            gross: Money::zero(Currency::EUR),
            breakdown: Vec::new(),
        };
        let invoice = SemanticInvoice::from_draft("2026-0001", &draft, &seller, &buyer, totals, Profile::En16931);

        let findings = validate(&invoice);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "BR-16");
    }

    #[test]
    fn line_sum_mismatch_is_br_co_10() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        let mut invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);
        invoice.totals.net = Money::from_major_units(101, Currency::EUR).unwrap();

        assert!(has(&validate(&invoice), "BR-CO-10"));
    }

    #[test]
    fn total_arithmetic_has_specific_codes() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);

        let mut invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);
        invoice.tax_exclusive = Money::from_major_units(99, Currency::EUR).unwrap();
        assert!(has(&validate(&invoice), "BR-CO-13"));

        let mut invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);
        invoice.totals.tax = Money::from_major_units(22, Currency::EUR).unwrap();
        assert!(has(&validate(&invoice), "BR-CO-14"));

        let mut invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);
        invoice.tax_inclusive = Money::from_major_units(122, Currency::EUR).unwrap();
        assert!(has(&validate(&invoice), "BR-CO-15"));

        let mut invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);
        invoice.amount_due = Money::from_major_units(120, Currency::EUR).unwrap();
        assert!(has(&validate(&invoice), "BR-CO-16"));
    }

    #[test]
    fn breakdown_tax_and_base_use_category_rules() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);

        let mut invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);
        invoice.totals.breakdown[0].tax = Money::from_major_units(22, Currency::EUR).unwrap();
        assert!(has(&validate(&invoice), "BR-S-09"));

        let mut invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);
        invoice.totals.breakdown[0].taxable_base = Money::from_major_units(90, Currency::EUR).unwrap();
        assert!(has(&validate(&invoice), "BR-S-08"));
    }

    #[test]
    fn category_rate_pairing_is_checked() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::ZERO))]);
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        assert!(has(&validate(&invoice), "BR-S-05"));
    }

    #[test]
    fn exempt_breakdown_needs_an_exemption_reason() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let draft = draft(vec![line(1, 100, TaxTreatment::zero(VatCategory::Exempt))]);
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        assert!(has(&validate(&invoice), "BR-E-10"));
    }

    #[test]
    fn exempt_breakdown_with_a_reason_is_clean() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let treatment = TaxTreatment::zero(VatCategory::Exempt).with_exemption_reason("Exempt supply");
        let draft = draft(vec![line(1, 100, treatment)]);
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn exempt_line_rejects_a_non_zero_rate() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let treatment = TaxTreatment {
            category: VatCategory::Exempt,
            rate: VatRate::STANDARD_NL,
            exemption_reason: Some("Exempt supply".to_owned()),
            exemption_reason_code: None,
        };
        let draft = draft(vec![line(1, 100, treatment)]);
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        assert!(has(&validate(&invoice), "BR-E-05"));
    }

    #[test]
    fn intra_community_requires_a_delivery_date() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let treatment = TaxTreatment::zero(VatCategory::IntraCommunity).with_exemption_reason("Intra-community supply");
        let mut draft = draft(vec![line(1, 100, treatment)]);

        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);
        assert!(has(&validate(&invoice), "BR-IC-11"));

        draft.delivery_date = Some(LocalDate::parse_iso("2026-10-25").unwrap());
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);
        assert!(!has(&validate(&invoice), "BR-IC-11"));
    }

    #[test]
    fn outside_scope_cannot_mix_categories() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let draft = draft(vec![
            line(1, 100, TaxTreatment::zero(VatCategory::OutsideScope)),
            line(2, 50, TaxTreatment::standard(VatRate::STANDARD_NL)),
        ]);
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        assert!(has(&validate(&invoice), "BR-O-12"));
    }

    #[test]
    fn every_emitted_code_is_catalogued() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let draft = draft(vec![
            line(1, 100, TaxTreatment::standard(VatRate::ZERO)),
            line(2, 50, TaxTreatment::zero(VatCategory::Exempt)),
        ]);
        let invoice = semantic_invoice("", &draft, &seller, &buyer);
        let findings = validate(&invoice);

        assert!(!findings.is_empty());
        for finding in findings {
            assert!(
                crate::rule::rule(finding.code).is_some(),
                "{} is not in the catalogue",
                finding.code
            );
        }
    }

    #[test]
    fn a_currency_outside_iso_4217_is_br_cl_04() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let currency = Currency::parse("EUX").expect("three uppercase letters are syntactically valid");
        let mut draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        draft.currency = currency;
        draft.lines[0].unit_price = Money::from_major_units(100, currency).expect("price");
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        let finding = finding(&findings, "BR-CL-04");
        assert_eq!(finding.field, "currency");
        assert_eq!(finding.bt, Some("BT-5"));
    }

    #[test]
    fn a_country_outside_iso_3166_is_br_cl_14() {
        let seller = seller();
        let mut buyer = party("Buyer BV");
        buyer.address.country = CountryCode::parse("XX").expect("two uppercase letters are syntactically valid");
        let draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        let finding = finding(&findings, "BR-CL-14");
        assert_eq!(finding.field, "buyer.address.country");
        assert_eq!(finding.bt, Some("BT-55"));
    }

    #[test]
    fn a_unit_outside_recommendation_20_is_br_cl_23() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let mut draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        draft.lines[0].unit = UnitCode::parse("XXX").expect("three uppercase letters are syntactically valid");
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        let finding = finding(&findings, "BR-CL-23");
        assert_eq!(finding.field, "lines[0].unit");
        assert_eq!(finding.bt, Some("BT-130"));
    }

    #[test]
    fn an_unknown_exemption_reason_code_is_br_cl_22() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let treatment = TaxTreatment::zero(VatCategory::Exempt)
            .with_exemption_reason("Exempt supply")
            .with_exemption_reason_code("VATEX-XX-1");
        let draft = draft(vec![line(1, 100, treatment)]);
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        let finding = finding(&findings, "BR-CL-22");
        assert_eq!(finding.field, "lines[0].exemption_reason_code");
        assert_eq!(finding.bt, Some("BT-121"));
    }

    #[test]
    fn an_unknown_endpoint_scheme_is_br_cl_25() {
        let mut seller = seller();
        seller.peppol_id = Some("9999:12345678".to_owned());
        let mut buyer = party("Buyer BV");
        buyer.peppol_id = Some("0106:12345678".to_owned());
        let draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        let finding = finding(&findings, "BR-CL-25");
        assert_eq!(finding.field, "seller.peppol_id");
        assert_eq!(finding.bt, Some("BT-34"));
        assert!(
            !findings.iter().any(|finding| finding.field == "buyer.peppol_id"),
            "0106 is a valid EAS scheme: {findings:?}"
        );
    }

    #[test]
    fn a_lowercase_exemption_reason_code_is_accepted() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let treatment = TaxTreatment::zero(VatCategory::Exempt)
            .with_exemption_reason("Exempt supply")
            .with_exemption_reason_code("vatex-eu-79-c");
        let draft = draft(vec![line(1, 100, treatment)]);
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn a_positive_amount_due_needs_a_due_date() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let mut draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        draft.due_date = None;
        let invoice = semantic_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        let finding = finding(&findings, "BR-CO-25");
        assert_eq!(finding.field, "due_date");
        assert_eq!(finding.bt, Some("BT-9"));
    }

    #[test]
    fn a_credit_note_needs_no_due_date() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let mut draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        draft.doc_type = DocType::CreditNote;
        draft.due_date = None;
        let invoice = semantic_invoice("2026-C0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        assert!(!has(&findings, "BR-CO-25"), "{findings:?}");
    }

    #[test]
    fn peppol_requires_endpoint_identifiers_and_a_dutch_address() {
        let seller = seller();
        let buyer = party("Buyer BV");
        let mut draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        draft.buyer_reference = Some("PO-1".to_owned());
        let invoice = peppol_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        assert!(has(&findings, "PEPPOL-EN16931-R020"), "seller endpoint: {findings:?}");
        assert!(has(&findings, "PEPPOL-EN16931-R010"), "buyer endpoint: {findings:?}");
        assert!(has(&findings, "NL-R-002"), "dutch address: {findings:?}");
        assert!(!has(&findings, "PEPPOL-EN16931-R003"), "a reference is present");
    }

    #[test]
    fn a_peppol_invoice_with_complete_parties_is_clean() {
        let seller = complete_party("Seller BV", "0106:12345678");
        let buyer = complete_party("Buyer BV", "0106:87654321");
        let mut draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        draft.buyer_reference = Some("PO-1".to_owned());
        let invoice = peppol_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn peppol_requires_a_buyer_reference_or_order_reference() {
        let seller = complete_party("Seller BV", "0106:12345678");
        let buyer = complete_party("Buyer BV", "0106:87654321");

        let draft_without_reference = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        let invoice = peppol_invoice("2026-0001", &draft_without_reference, &seller, &buyer);
        assert!(has(&validate(&invoice), "PEPPOL-EN16931-R003"));

        let mut with_order = draft_without_reference.clone();
        with_order.order_reference = Some("ORD-1".to_owned());
        let invoice = peppol_invoice("2026-0001", &with_order, &seller, &buyer);
        assert!(!has(&validate(&invoice), "PEPPOL-EN16931-R003"));
    }

    #[test]
    fn a_malformed_endpoint_identifier_is_not_an_endpoint() {
        let mut seller = complete_party("Seller BV", "0106:12345678");
        seller.peppol_id = Some("12345678".to_owned());
        let buyer = complete_party("Buyer BV", "0106:87654321");
        let mut draft = draft(vec![line(1, 100, TaxTreatment::standard(VatRate::STANDARD_NL))]);
        draft.buyer_reference = Some("PO-1".to_owned());
        let invoice = peppol_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        let finding = finding(&findings, "PEPPOL-EN16931-R020");
        assert_eq!(finding.field, "seller.peppol_id");
        assert!(finding.message.contains("EAS:value"), "{}", finding.message);
    }

    #[test]
    fn a_vatex_code_must_match_the_vat_category() {
        let seller = complete_party("Seller BV", "0106:12345678");
        let buyer = complete_party("Buyer BV", "0106:87654321");
        let treatment = TaxTreatment::zero(VatCategory::Exempt).with_exemption_reason_code("VATEX-EU-AE");
        let mut draft = draft(vec![line(1, 100, treatment)]);
        draft.buyer_reference = Some("PO-1".to_owned());
        let invoice = peppol_invoice("2026-0001", &draft, &seller, &buyer);

        let findings = validate(&invoice);
        let finding = finding(&findings, "PEPPOL-EN16931-P0107");
        assert_eq!(finding.field, "lines[0].vat_category");
        assert_eq!(finding.bt, Some("BT-121"));
    }
}
