//! The element helpers both syntax writers share.
//!
//! A writer here is a dumb serialiser: fixed element order, no timestamps, no
//! pretty-print variation, so the bytes of the same invoice hash the same. The
//! helpers write to a `String` rather than a tree because the order is decided by
//! the caller, and the caller is a straight-line function.

use crux::Money;

/// Opens an element at a depth.
pub(crate) fn open(out: &mut String, depth: usize, tag: &str) {
    indent(out, depth);
    out.push('<');
    out.push_str(tag);
    out.push_str(">\n");
}

/// Closes an element at a depth.
pub(crate) fn close(out: &mut String, depth: usize, tag: &str) {
    indent(out, depth);
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

/// An element whose text is the value.
pub(crate) fn text(out: &mut String, depth: usize, tag: &str, value: &str) {
    text_attrs(out, depth, tag, &[], value);
}

/// An element with attributes and text.
pub(crate) fn text_attrs(out: &mut String, depth: usize, tag: &str, attrs: &[(&str, &str)], value: &str) {
    indent(out, depth);
    out.push('<');
    out.push_str(tag);
    for (name, attribute) in attrs {
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        escape(out, attribute);
        out.push('"');
    }
    out.push('>');
    escape(out, value);
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

/// An amount in the document currency, two decimals, with the currency named.
pub(crate) fn amount(out: &mut String, depth: usize, tag: &str, currency: &str, value: Money) {
    let rendered = value.to_fixed_string(2).unwrap_or_else(|_| "0.00".to_owned());
    text_attrs(out, depth, tag, &[("currencyID", currency)], &rendered);
}

/// An amount without a currency attribute, which is what both CII and UBL use for
/// line-level money.
pub(crate) fn money(out: &mut String, depth: usize, tag: &str, value: Money) {
    text(
        out,
        depth,
        tag,
        &value.to_fixed_string(2).unwrap_or_else(|_| "0.00".to_owned()),
    );
}

/// A VAT rate as a percentage, without a trailing zero pair.
pub(crate) fn rate_basis_points(basis_points: u16) -> String {
    let whole = basis_points / 100;
    let fraction = basis_points % 100;
    if fraction == 0 {
        whole.to_string()
    } else if fraction.is_multiple_of(10) {
        format!("{whole}.{}", fraction / 10)
    } else {
        format!("{whole}.{fraction:02}")
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn escape(out: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(character),
        }
    }
}
