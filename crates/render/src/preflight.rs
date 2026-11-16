//! What the renderer can see is wrong before it draws anything.
//!
//! A preflight finding is not a refusal. The document is issued and filed either
//! way; these are the things a person can still change — a character the bundled
//! font has no glyph for, an amount too wide for the page — reported with the field
//! that carries them instead of surfacing as a blank box in a PDF nobody re-reads.

// Measuring a page and a run of text is floating point. Amounts are integers from
// `crux` and never pass through this arithmetic, which is also why the workspace
// denies float arithmetic everywhere else.
#![allow(clippy::float_arithmetic)]

use harfrust::Font;

use crate::layout::FONT_REGULAR;
use crate::template::Section;
use crate::text::{self, Metrics};
use crate::{InvoiceData, Template};

/// Something the document will be drawn badly unless it is changed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Preflight {
    /// Stable code, e.g. `M4-001`.
    pub code: &'static str,
    /// Path of the value that carries the problem, e.g. `lines[2].name`.
    pub field: String,
    /// What will go wrong, in the words the window shows.
    pub message: String,
}

/// The characters the bundled font cannot draw.
const NO_GLYPH: &str = "M4-001";

/// A line the page cannot hold.
const TOO_WIDE: &str = "M4-002";

/// The problems `template` and `data` will produce together.
pub(crate) fn check(template: &Template, data: &InvoiceData) -> Vec<Preflight> {
    let Some(font) = Font::new(FONT_REGULAR, 0) else {
        // A font that will not load is the renderer's problem, and it says so when
        // it tries to draw; there is nothing to add here.
        return Vec::new();
    };
    let metrics = Metrics::of(&font);
    let mut findings = Vec::new();

    for (field, value) in drawn_text(template, data) {
        if let Some(message) = missing_glyphs(&font, value) {
            findings.push(Preflight {
                code: NO_GLYPH,
                field,
                message,
            });
        }
    }

    if template.sections.contains(&Section::Totals) {
        findings.extend(totals_too_wide(template, data, &font, metrics));
    }
    findings
}

/// Every value the template draws, with the field that carries it.
///
/// Only the sections the template names are checked: a layout that leaves the lines
/// out does not draw their names, so a character it cannot draw in one of them is
/// not a problem for that document.
fn drawn_text<'a>(template: &Template, data: &'a InvoiceData) -> Vec<(String, &'a str)> {
    let mut drawn = Vec::new();
    if template.sections.contains(&Section::Header) {
        drawn.push(("title".to_owned(), data.title.as_str()));
        drawn.push(("number".to_owned(), data.number.as_str()));
        drawn.push(("seller.name".to_owned(), data.seller.name.as_str()));
        drawn.push(("seller.address".to_owned(), data.seller.address.as_str()));
        if let Some(vat) = &data.seller.vat {
            drawn.push(("seller.vat".to_owned(), vat.as_str()));
        }
    }
    if template.sections.contains(&Section::Parties) {
        drawn.push(("customer.name".to_owned(), data.customer.name.as_str()));
        drawn.push(("customer.address".to_owned(), data.customer.address.as_str()));
        if let Some(vat) = &data.customer.vat {
            drawn.push(("customer.vat".to_owned(), vat.as_str()));
        }
    }
    if template.sections.contains(&Section::Lines) {
        for (index, line) in data.lines.iter().enumerate() {
            for (name, value) in [
                ("name", &line.name),
                ("qty", &line.qty),
                ("price", &line.price),
                ("vat", &line.vat),
                ("total", &line.total),
            ] {
                drawn.push((format!("lines[{index}].{name}"), value.as_str()));
            }
        }
    }
    if template.sections.contains(&Section::Note)
        && let Some(note) = &data.notes
    {
        drawn.push(("note".to_owned(), note.as_str()));
    }
    drawn.retain(|(_, value)| !value.trim().is_empty());
    drawn
}

/// Names the characters the font cannot draw, so they can be replaced.
///
/// A glyph id of zero is the font's "not this one" box. Whitespace is skipped: a
/// space that maps to no glyph is still a space, and nothing is lost.
fn missing_glyphs(font: &Font, value: &str) -> Option<String> {
    let mut missing: Vec<char> = Vec::new();
    for glyph in text::shape(font, value) {
        if glyph.glyph != 0 {
            continue;
        }
        let Some(character) = value[glyph.cluster..].chars().next() else {
            continue;
        };
        if character.is_whitespace() || missing.contains(&character) {
            continue;
        }
        missing.push(character);
    }
    if missing.is_empty() {
        return None;
    }

    let named: Vec<String> = missing
        .iter()
        .take(3)
        .map(|character| format!("{character:?} (U+{:04X})", *character as u32))
        .collect();
    let mut message = format!("the embedded font has no glyph for {}", named.join(", "));
    if let Some(rest) = missing.len().checked_sub(named.len()).filter(|rest| *rest > 0) {
        message.push_str(&format!(" and {rest} more character(s)"));
    }
    message.push_str(", so it is drawn as a blank box; replace it or the page will show a gap");
    Some(message)
}

/// The totals rows are drawn right-aligned on one line each, so an amount wider
/// than the page runs off the left margin instead of wrapping.
fn totals_too_wide(template: &Template, data: &InvoiceData, font: &Font, metrics: Metrics) -> Vec<Preflight> {
    let content = template.page.width_pt - template.page.margin_left_pt - template.page.margin_right_pt;
    let size = template.text.size_pt;
    let bold_font = Font::new(crate::layout::FONT_BOLD, 0);
    let mut findings = Vec::new();
    for (field, text, bold) in [
        ("totals.net", format!("{} {}", data.net, data.currency), false),
        ("totals.tax", format!("{} {}", data.tax, data.currency), false),
        ("totals.gross", format!("{} {}", data.gross, data.currency), true),
    ] {
        // The gross row is bold, and bold is wider, so it is measured with the
        // font it is drawn in.
        let measured = if bold { bold_font.as_ref().unwrap_or(font) } else { font };
        let width = text::width(measured, metrics, &text, size);
        if width > content {
            findings.push(Preflight {
                code: TOO_WIDE,
                field: field.to_owned(),
                message: format!(
                    "{text} needs {width:.1} pt but the page gives {content:.1} pt, so it will run off the page"
                ),
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Language, PartyData};

    fn sample() -> InvoiceData {
        InvoiceData {
            title: "Invoice".to_owned(),
            labels: Language::English.labels(),
            number: "2026-0001".to_owned(),
            issue_date: "2026-10-27".to_owned(),
            due_date: Some("2026-11-26".to_owned()),
            currency: "EUR".to_owned(),
            seller: PartyData {
                name: "Seller BV".to_owned(),
                address: "Main 1, Amsterdam, NL".to_owned(),
                kvk: None,
                vat: Some("NL123456782B01".to_owned()),
            },
            customer: PartyData {
                name: "Customer BV".to_owned(),
                address: "Rotterdam, NL".to_owned(),
                kvk: None,
                vat: None,
            },
            lines: vec![crate::LineData {
                name: "Consulting".to_owned(),
                qty: "2".to_owned(),
                price: "50.00".to_owned(),
                vat: "21".to_owned(),
                total: "100.00".to_owned(),
            }],
            net: "100.00".to_owned(),
            tax: "21.00".to_owned(),
            gross: "121.00".to_owned(),
            notes: None,
        }
    }

    #[test]
    fn a_document_the_font_can_draw_has_nothing_to_say() {
        assert_eq!(check(&Template::default(), &sample()), Vec::new());
    }

    #[test]
    fn a_character_the_font_cannot_draw_is_named_with_its_field() {
        let mut data = sample();
        data.lines[0].name = "Consulting 中".to_owned();
        let findings = check(&Template::default(), &data);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].code, NO_GLYPH);
        assert_eq!(findings[0].field, "lines[0].name");
        assert!(findings[0].message.contains("U+4E2D"), "{}", findings[0].message);
    }

    #[test]
    fn a_section_the_template_leaves_out_is_not_checked() {
        // The compact layout draws the lines; a template with no lines section does
        // not, so an item name it cannot draw is not a problem for that document.
        let mut data = sample();
        data.lines[0].name = "Consulting 中".to_owned();
        let mut template = Template::default();
        template.sections.retain(|section| *section != Section::Lines);
        assert_eq!(check(&template, &data), Vec::new());
    }

    #[test]
    fn an_amount_wider_than_the_page_is_reported() {
        let mut data = sample();
        // A fourth class of digits: wider than the page, not a rounding error.
        data.net = format!("{}.90", "9".repeat(200));
        data.gross = data.net.clone();
        let findings = check(&Template::default(), &data);
        assert!(
            findings.iter().any(|finding| finding.code == TOO_WIDE
                && finding.field == "totals.net"
                && finding.message.contains("run off the page")),
            "{findings:?}"
        );
    }

    #[test]
    fn every_character_is_named_once_with_a_count() {
        let mut data = sample();
        data.customer.name = "顧客 中 中 文".to_owned();
        let findings = check(&Template::default(), &data);
        assert_eq!(findings.len(), 1, "{findings:?}");
        let message = &findings[0].message;
        assert_eq!(
            message.matches("U+4E2D").count(),
            1,
            "a repeated character is named once: {message}"
        );
        assert!(message.contains("U+9867"), "{message}");
        assert!(message.contains("and 1 more"), "{message}");
    }
}
