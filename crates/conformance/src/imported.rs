//! Validating a document somebody else wrote.
//!
//! The rule engine reads one view, [`SemanticInvoice`], and that view borrows a
//! draft: its lines are the composer's and its totals are what the domain
//! calculation produced. A received document has neither — it states amounts
//! instead of computing them — so this module turns what the reader extracted
//! into the same view. The same rule code then runs over both, which is what
//! makes a disagreement with the official validator a disagreement about a rule
//! rather than about which path happened to execute.

use crux::{DraftLine, LineId, Money};
use syntax::read::ImportedInvoice;

use crate::profile::Profile;
use crate::semantic::SemanticInvoice;

/// A received document's lines, in the shape the rule engine reads.
///
/// The line net amounts are kept as the document states them (BT-131): a
/// received document's line nets are claims, and the rules that compare them
/// with its own totals are the point of validating it.
pub struct ImportedSemantics {
    lines: Vec<DraftLine>,
    line_nets: Vec<Money>,
}

impl ImportedSemantics {
    /// Converts the lines of a received document.
    pub fn new(invoice: &ImportedInvoice) -> Self {
        let mut lines = Vec::with_capacity(invoice.lines.len());
        let mut line_nets = Vec::with_capacity(invoice.lines.len());
        for line in &invoice.lines {
            lines.push(DraftLine {
                id: LineId::new(),
                position: line.position,
                name: line.name.clone(),
                description: line.description.clone(),
                quantity: line.quantity,
                unit: line.unit,
                unit_price: line.unit_price,
                price_base_quantity: line.base_quantity,
                treatment: line.treatment.clone(),
            });
            line_nets.push(line.line_net);
        }
        Self { lines, line_nets }
    }

    /// The view the rules read.
    ///
    /// Document-level allowances, charges, payments, and rounding are zero: the
    /// reader does not model them yet, so the rules that depend on them are
    /// comparing against nothing rather than against a guess.
    pub fn view<'a>(&'a self, invoice: &'a ImportedInvoice) -> SemanticInvoice<'a> {
        let zero = Money::zero(invoice.currency);
        SemanticInvoice {
            number: &invoice.number,
            issue_date: invoice.issue_date,
            due_date: invoice.due_date,
            delivery_date: invoice.delivery_date,
            doc_type: invoice.doc_type,
            currency: invoice.currency,
            buyer_reference: invoice.buyer_reference.as_deref(),
            order_reference: invoice.order_reference.as_deref(),
            seller: &invoice.seller,
            buyer: &invoice.customer,
            lines: &self.lines,
            totals: invoice.totals.clone(),
            // BT-109 and BT-112 are what the reader filled `net` and `gross` with.
            tax_exclusive: invoice.totals.net,
            tax_inclusive: invoice.totals.gross,
            allowances: zero,
            charges: zero,
            paid: zero,
            rounding: zero,
            amount_due: invoice.totals.gross,
            stated_line_total: Some(invoice.line_extension_total),
            stated_line_nets: Some(&self.line_nets),
            profile: Profile::from_specification_identifier(invoice.profile.as_deref().unwrap_or_default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crux::{
        Address, CountryCode, Currency, CustomerId, DocType, DraftLine, InvoiceDraft, InvoiceTotals, LineId, LocalDate,
        Money, Party, Quantity, SellerId, TaxBreakdown, TaxTreatment, UnitCode, VatCategory, VatRate, calculate_totals,
    };
    use syntax::{UblInvoice, UblLine, write_invoice};

    use super::*;
    use crate::validate::validate;

    fn party(name: &str) -> Party {
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
        party.peppol_id = Some("0106:12345678".to_owned());
        party
    }

    /// A UBL invoice that satisfies the rules, written by our own writer.
    fn written() -> String {
        let treatment = TaxTreatment::standard(VatRate::STANDARD_NL);
        let line = UblLine {
            id: 1,
            name: "Consulting",
            description: None,
            quantity: Quantity::from_units(2).expect("quantity"),
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(50, Currency::EUR).expect("price"),
            base_quantity: Quantity::ONE,
            line_net: Money::from_major_units(100, Currency::EUR).expect("line net"),
            treatment: &treatment,
        };
        let lines = [line];
        let totals = InvoiceTotals {
            net: Money::from_major_units(100, Currency::EUR).expect("net"),
            tax: Money::from_major_units(21, Currency::EUR).expect("tax"),
            gross: Money::from_major_units(121, Currency::EUR).expect("gross"),
            breakdown: vec![TaxBreakdown {
                category: VatCategory::Standard,
                rate_bp: 2100,
                taxable_base: Money::from_major_units(100, Currency::EUR).expect("base"),
                tax: Money::from_major_units(21, Currency::EUR).expect("row tax"),
            }],
        };
        let seller = party("Seller BV");
        let customer = party("Customer BV");
        let invoice = UblInvoice {
            number: "2026-0001",
            issue_date: LocalDate::parse_iso("2026-10-27").expect("date"),
            due_date: Some(LocalDate::parse_iso("2026-11-26").expect("due")),
            delivery_date: None,
            doc_type: DocType::Invoice,
            currency: Currency::EUR,
            buyer_reference: Some("PO-1"),
            order_reference: None,
            note: None,
            preceding_invoice: None,
            seller: &seller,
            customer: &customer,
            lines: &lines,
            totals: &totals,
        };
        write_invoice(&invoice).expect("the writer accepts its own view")
    }

    fn codes(xml: &str) -> Vec<&'static str> {
        let invoice = syntax::read::read_invoice(xml).expect("the reader accepts what we wrote");
        let semantics = ImportedSemantics::new(&invoice);
        let mut codes: Vec<&'static str> = validate(&semantics.view(&invoice))
            .into_iter()
            .map(|finding| finding.code)
            .collect();
        codes.sort_unstable();
        codes.dedup();
        codes
    }

    #[test]
    fn a_document_we_wrote_comes_back_clean() {
        assert!(codes(&written()).is_empty(), "{:?}", codes(&written()));
    }

    /// The document states BT-106 and BT-131 lower than its own total, so it
    /// disagrees with itself in exactly the two places the official validator
    /// reports. Our engine sees the same two, through the stated amounts.
    #[test]
    fn a_document_that_disagrees_with_itself_is_reported() {
        let mut xml = written();
        // Both the document-level line total and the line's own net, as the
        // expected-findings gate mutates them.
        xml = xml.replace(
            "<cbc:LineExtensionAmount currencyID=\"EUR\">100.00</cbc:LineExtensionAmount>",
            "<cbc:LineExtensionAmount currencyID=\"EUR\">90.00</cbc:LineExtensionAmount>",
        );
        assert_ne!(xml, written(), "the fixture must contain the element");

        assert_eq!(codes(&xml), ["BR-CO-13", "BR-S-08"]);
    }

    /// The view is a view: the lines it exposes are the document's statements,
    /// not a recomputation that would hide the difference.
    #[test]
    fn the_view_carries_what_the_document_states() {
        let xml = written();
        let invoice = syntax::read::read_invoice(&xml).expect("import");
        let semantics = ImportedSemantics::new(&invoice);
        let view = semantics.view(&invoice);

        assert_eq!(view.stated_line_total, Some(invoice.line_extension_total));
        assert_eq!(view.stated_line_nets, Some(semantics.line_nets.as_slice()));
        assert_eq!(view.line_net(0).map(|net| net.micros()), Some(100_000_000));
        assert_eq!(view.lines.len(), 1);
        assert_eq!(view.lines[0].name, "Consulting");
        assert_eq!(
            view.profile,
            Profile::PeppolBis3,
            "the writer claims the Peppol profile"
        );
    }

    /// A draft keeps computing its own amounts: nothing about this changed the
    /// path the composer uses.
    #[test]
    fn a_draft_still_computes_its_line_nets() {
        let mut draft = InvoiceDraft::new(
            SellerId::new(),
            CustomerId::new(),
            Currency::EUR,
            LocalDate::parse_iso("2026-10-27").unwrap(),
        );
        draft.lines.push(DraftLine {
            id: LineId::new(),
            position: 1,
            name: "Consulting".to_owned(),
            description: None,
            quantity: Quantity::from_units(2).unwrap(),
            unit: UnitCode::PIECE,
            unit_price: Money::from_major_units(50, Currency::EUR).unwrap(),
            price_base_quantity: Quantity::ONE,
            treatment: TaxTreatment::standard(VatRate::STANDARD_NL),
        });
        draft.due_date = Some(LocalDate::parse_iso("2026-11-26").unwrap());
        let totals = calculate_totals(&draft.lines).unwrap();
        let seller = party("Seller BV");
        let buyer = party("Customer BV");
        let view = SemanticInvoice::from_draft("2026-0001", &draft, &seller, &buyer, totals, Profile::En16931);

        assert_eq!(view.stated_line_total, None);
        assert_eq!(view.stated_line_nets, None);
        assert_eq!(view.line_net(0).map(|net| net.micros()), Some(100_000_000));
        assert!(validate(&view).is_empty(), "{:?}", validate(&view));
    }
}
