//! Laying an invoice onto pages.
//!
//! The engine owns everything a template cannot: measuring text, wrapping it,
//! resolving column widths, and starting a new page when the lines table runs
//! off the bottom. It emits a [`Document`] of pages of draw operations in
//! points, with the y axis pointing down from the top-left corner of the page,
//! so the same model can be written to PDF and drawn in the window.
//!
//! Amounts never reach this module as numbers: `InvoiceData` carries them
//! pre-formatted. The only arithmetic here is geometry.

#![allow(clippy::float_arithmetic)]

use crate::template::{Align, Column, Field, LabelOverrides, Section, Template, Width};
use crate::text::Metrics;
use crate::{InvoiceData, Labels, LineData, RenderError};

/// The body font, compiled into the binary.
pub const FONT_REGULAR: &[u8] = include_bytes!("../fonts/DejaVuSerif.ttf");

/// The bold font.
pub const FONT_BOLD: &[u8] = include_bytes!("../fonts/DejaVuSerif-Bold.ttf");

/// The names a footer may use between `{{` and `}}`.
pub const PLACEHOLDERS: &[&str] = &[
    "seller.name",
    "seller.kvk",
    "seller.vat",
    "customer.name",
    "customer.vat",
    "invoice.number",
    "invoice.issue_date",
    "invoice.due_date",
    "totals.net",
    "totals.tax",
    "totals.gross",
    "page.number",
];

/// Space between the two header columns.
const COLUMN_GUTTER: f32 = 28.0;

/// Space between blocks of the document.
const BLOCK_GAP: f32 = 16.0;

/// Inset of a table cell's text from the row's top and bottom.
const ROW_PADDING: f32 = 3.0;

/// A laid-out document: one or more pages of drawing operations.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    /// Pages in reading order; never empty.
    pub pages: Vec<Page>,
    /// The PDF title, used for metadata.
    pub title: String,
    /// The creation date recorded in the PDF, derived from the issue date.
    pub created: Option<Created>,
}

/// A date written into PDF metadata.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Created {
    /// Year, 0-9999.
    pub year: u16,
    /// Month, 1-12.
    pub month: u8,
    /// Day, 1-31.
    pub day: u8,
}

/// One page of a document.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// Page width in points.
    pub width: f32,
    /// Page height in points.
    pub height: f32,
    /// Everything drawn on the page, in paint order.
    pub ops: Vec<Op>,
}

/// A point in page space, in points, measured from the top-left corner.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Point {
    /// Distance from the left edge.
    pub x: f32,
    /// Distance from the top edge.
    pub y: f32,
}

/// An opaque `#rrggbb` colour.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Colour {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
}

/// One drawing operation.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A single line of text. `at` is the top-left of the line's em box.
    Text {
        /// Top-left corner.
        at: Point,
        /// Font size in points.
        size: f32,
        /// Whether the line uses the bold face.
        bold: bool,
        /// Ink colour.
        colour: Colour,
        /// The text itself; already wrapped.
        text: String,
    },
    /// A straight line, used for table rules.
    Rule {
        /// Start.
        from: Point,
        /// End.
        to: Point,
        /// Stroke width in points.
        width: f32,
        /// Ink colour.
        colour: Colour,
    },
}

/// Parses `#rrggbb`, the only colour notation a template may use.
pub fn colour(value: &str) -> Option<Colour> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(Colour {
        red: u8::from_str_radix(&hex[0..2], 16).ok()?,
        green: u8::from_str_radix(&hex[2..4], 16).ok()?,
        blue: u8::from_str_radix(&hex[4..6], 16).ok()?,
    })
}

/// Lays `data` out with `template`.
pub fn layout(template: &Template, data: &InvoiceData) -> Result<Document, RenderError> {
    template.validate()?;

    let regular = harfrust::Font::new(FONT_REGULAR, 0).ok_or(RenderError::Font)?;
    let bold = harfrust::Font::new(FONT_BOLD, 0).ok_or(RenderError::Font)?;
    let metrics = Metrics::of(&regular);

    let ink = colour(&template.text.colour).unwrap_or_default();
    let rule_ink = colour(&template.rules.colour).unwrap_or_default();
    let words = Words::new(&template.labels, data.labels);

    let compositor = Compositor {
        template,
        data,
        words,
        measurer: Measurer { regular, bold, metrics },
        ink,
        rule_ink,
        canvas: Canvas::new(&template.page),
    };
    compositor.run()
}

/// Shapes text to find out how wide it is, and breaks it into lines.
struct Measurer {
    regular: harfrust::Font,
    bold: harfrust::Font,
    metrics: Metrics,
}

impl Measurer {
    /// The advance width of `text` at `size`.
    fn width(&self, text: &str, size: f32, bold: bool) -> f32 {
        let font = if bold { &self.bold } else { &self.regular };
        crate::text::width(font, self.metrics, text, size)
    }

    /// Greedy word wrap to `width`. A word wider than the column is broken
    /// between characters rather than allowed to overflow.
    fn wrap(&self, text: &str, size: f32, bold: bool, width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut current = String::new();
            for word in paragraph.split_whitespace() {
                let candidate = if current.is_empty() {
                    word.to_owned()
                } else {
                    format!("{current} {word}")
                };
                if self.width(&candidate, size, bold) <= width {
                    current = candidate;
                    continue;
                }
                if !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                }
                if self.width(word, size, bold) <= width {
                    current = word.to_owned();
                    continue;
                }
                let mut chunk = String::new();
                for character in word.chars() {
                    let candidate = format!("{chunk}{character}");
                    if !chunk.is_empty() && self.width(&candidate, size, bold) > width {
                        lines.push(std::mem::take(&mut chunk));
                    }
                    chunk.push(character);
                }
                current = chunk;
            }
            lines.push(current);
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }
}

/// The template's words, after the built-in labels and any overrides.
struct Words<'a> {
    due: &'a str,
    bill_to: &'a str,
    net: &'a str,
    vat: &'a str,
    total: &'a str,
    item: &'a str,
    qty: &'a str,
    unit_price: &'a str,
}

impl<'a> Words<'a> {
    fn new(overrides: &'a LabelOverrides, labels: Labels) -> Self {
        Self {
            due: overrides.due.as_deref().unwrap_or(labels.due),
            bill_to: overrides.bill_to.as_deref().unwrap_or(labels.bill_to),
            net: overrides.net.as_deref().unwrap_or(labels.net),
            vat: overrides.vat.as_deref().unwrap_or(labels.vat),
            total: overrides.total.as_deref().unwrap_or(labels.total),
            item: overrides.item.as_deref().unwrap_or(labels.item),
            qty: overrides.qty.as_deref().unwrap_or(labels.qty),
            unit_price: overrides.unit_price.as_deref().unwrap_or(labels.unit_price),
        }
    }

    fn heading(&self, field: Field) -> &str {
        match field {
            Field::Item => self.item,
            Field::Qty => self.qty,
            Field::UnitPrice => self.unit_price,
            Field::Vat => self.vat,
            Field::Total => self.total,
        }
    }
}

/// One run of lines to draw at a position.
struct Paragraph<'a> {
    size: f32,
    colour: Colour,
    lines: &'a [(&'a str, bool)],
}

/// A page under construction, with the cursor at the next free line.
struct Canvas {
    width: f32,
    height: f32,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    cursor: f32,
    pages: Vec<Page>,
    current: Vec<Op>,
}

impl Canvas {
    fn new(page: &crate::template::PageStyle) -> Self {
        Self {
            width: page.width_pt,
            height: page.height_pt,
            left: page.margin_left_pt,
            right: page.margin_right_pt,
            top: page.margin_top_pt,
            bottom: page.height_pt - page.margin_bottom_pt,
            cursor: page.margin_top_pt,
            pages: Vec::new(),
            current: Vec::new(),
        }
    }

    /// Usable width between the left and right margins.
    fn content_width(&self) -> f32 {
        self.width - self.left - self.right
    }

    /// The x of the right margin.
    fn right_edge(&self) -> f32 {
        self.width - self.right
    }

    /// Makes room for `height`, breaking the page when needed.
    ///
    /// Returns whether a new page was started. An empty page is never broken,
    /// so a block taller than the page overflows instead of looping.
    fn ensure(&mut self, height: f32) -> bool {
        if self.cursor + height <= self.bottom {
            return false;
        }
        if self.current.is_empty() {
            return false;
        }
        self.pages.push(Page {
            width: self.width,
            height: self.height,
            ops: std::mem::take(&mut self.current),
        });
        self.cursor = self.top;
        true
    }

    fn push(&mut self, op: Op) {
        self.current.push(op);
    }

    /// Draws a paragraph at `top`, returning the y below its last line.
    fn block(&mut self, m: &Measurer, x: f32, width: f32, top: f32, paragraph: &Paragraph<'_>) -> f32 {
        let mut y = top;
        for (text, bold) in paragraph.lines {
            if text.is_empty() {
                continue;
            }
            for line in m.wrap(text, paragraph.size, *bold, width) {
                if line.is_empty() {
                    // A blank line in a note is a paragraph break, not a gap
                    // to be swallowed.
                    y += m.metrics.line_height(paragraph.size);
                    continue;
                }
                self.push(Op::Text {
                    at: Point { x, y },
                    size: paragraph.size,
                    bold: *bold,
                    colour: paragraph.colour,
                    text: line,
                });
                y += m.metrics.line_height(paragraph.size);
            }
        }
        y
    }

    /// Draws a horizontal rule across the content width.
    fn rule(&mut self, y: f32, width: f32, colour: Colour) {
        self.push(Op::Rule {
            from: Point { x: self.left, y },
            to: Point {
                x: self.width - self.right,
                y,
            },
            width,
            colour,
        });
    }

    fn finish(mut self) -> Vec<Page> {
        if !self.current.is_empty() || self.pages.is_empty() {
            self.pages.push(Page {
                width: self.width,
                height: self.height,
                ops: std::mem::take(&mut self.current),
            });
        }
        self.pages
    }
}

/// Lays the sections out in order.
struct Compositor<'a> {
    template: &'a Template,
    data: &'a InvoiceData,
    words: Words<'a>,
    measurer: Measurer,
    ink: Colour,
    rule_ink: Colour,
    canvas: Canvas,
}

impl<'a> Compositor<'a> {
    fn run(mut self) -> Result<Document, RenderError> {
        for section in &self.template.sections {
            match section {
                Section::Header => self.header(),
                Section::Parties => self.parties(),
                Section::Lines => self.lines(),
                Section::Totals => self.totals(),
                Section::Note => self.note(),
                Section::Footer => {}
            }
        }

        let Self {
            template,
            data,
            measurer,
            canvas,
            ..
        } = self;
        let mut pages = canvas.finish();
        if template.sections.contains(&Section::Footer) {
            for (index, page) in pages.iter_mut().enumerate() {
                page.ops.extend(footer_ops(template, data, &measurer, page, index + 1));
            }
        }
        Ok(Document {
            pages,
            title: format!("{} {}", data.title, data.number),
            created: created_from(&data.issue_date),
        })
    }

    fn header(&mut self) {
        let title_size = self.template.text.title_size_pt;
        let size = self.template.text.size_pt;
        let colour = self.ink;
        let column = (self.canvas.content_width() - COLUMN_GUTTER) / 2.0;
        let left = self.canvas.left;
        let right = left + column + COLUMN_GUTTER;
        let top = self.canvas.cursor;

        let title = format!("{} {}", self.data.title, self.data.number);
        let mut left_bottom = self.canvas.block(
            &self.measurer,
            left,
            column,
            top,
            &Paragraph {
                size: title_size,
                colour,
                lines: &[(&title, true)],
            },
        );
        left_bottom = self.canvas.block(
            &self.measurer,
            left,
            column,
            left_bottom + 2.0,
            &Paragraph {
                size,
                colour,
                lines: &[(&self.data.issue_date, false)],
            },
        );
        if let Some(due) = &self.data.due_date {
            let line = format!("{} {due}", self.words.due);
            left_bottom = self.canvas.block(
                &self.measurer,
                left,
                column,
                left_bottom,
                &Paragraph {
                    size,
                    colour,
                    lines: &[(&line, false)],
                },
            );
        }

        let mut right_bottom = self.canvas.block(
            &self.measurer,
            right,
            column,
            top,
            &Paragraph {
                size,
                colour,
                lines: &[(&self.data.seller.name, true), (&self.data.seller.address, false)],
            },
        );
        if let Some(vat) = &self.data.seller.vat {
            let line = format!("{} {vat}", self.words.vat);
            right_bottom = self.canvas.block(
                &self.measurer,
                right,
                column,
                right_bottom,
                &Paragraph {
                    size,
                    colour,
                    lines: &[(&line, false)],
                },
            );
        }

        self.canvas.cursor = left_bottom.max(right_bottom) + BLOCK_GAP;
    }

    fn parties(&mut self) {
        let size = self.template.text.size_pt;
        let colour = self.ink;
        let left = self.canvas.left;
        let width = self.canvas.content_width();
        let top = self.canvas.cursor;

        let mut y = self.canvas.block(
            &self.measurer,
            left,
            width,
            top,
            &Paragraph {
                size,
                colour,
                lines: &[(self.words.bill_to, true)],
            },
        );
        y += 2.0;
        y = self.canvas.block(
            &self.measurer,
            left,
            width,
            y,
            &Paragraph {
                size,
                colour,
                lines: &[(&self.data.customer.name, false), (&self.data.customer.address, false)],
            },
        );
        if let Some(vat) = &self.data.customer.vat {
            let line = format!("{} {vat}", self.words.vat);
            y = self.canvas.block(
                &self.measurer,
                left,
                width,
                y,
                &Paragraph {
                    size,
                    colour,
                    lines: &[(&line, false)],
                },
            );
        }

        self.canvas.cursor = y + BLOCK_GAP;
    }

    fn lines(&mut self) {
        let columns = self.template.lines.columns.clone();
        if columns.is_empty() || self.data.lines.is_empty() {
            return;
        }
        let size = self.template.text.size_pt;
        let line_height = self.measurer.metrics.line_height(size);
        let widths = resolve_widths(&columns, self.canvas.content_width());
        let edges = column_edges(self.canvas.left, &widths);
        let headings: Vec<String> = columns
            .iter()
            .map(|column| self.words.heading(column.field).to_owned())
            .collect();
        let header_height = line_height + 2.0 * ROW_PADDING;

        self.canvas.cursor += BLOCK_GAP;
        self.canvas.ensure(header_height + line_height);
        self.table_header(&columns, &headings, &edges, &widths, header_height);

        let colour = self.ink;
        let row_width = self.template.rules.width_pt;
        let rule_ink = self.rule_ink;

        for line in &self.data.lines {
            let cells: Vec<String> = columns
                .iter()
                .map(|column| cell(line, column.field).to_owned())
                .collect();
            let wrapped: Vec<Vec<String>> = cells
                .iter()
                .zip(&widths)
                .map(|(text, width)| self.measurer.wrap(text, size, false, *width))
                .collect();
            let count = wrapped.iter().map(Vec::len).max().unwrap_or(1);
            let height = count as f32 * line_height + 2.0 * ROW_PADDING;

            if self.canvas.ensure(height) {
                // A continued table repeats its heading on the new page.
                self.table_header(&columns, &headings, &edges, &widths, header_height);
            }

            let top = self.canvas.cursor;
            for (index, column) in columns.iter().enumerate() {
                let mut y = top + ROW_PADDING;
                for text in &wrapped[index] {
                    if text.is_empty() {
                        continue;
                    }
                    let x = match column.align {
                        Align::Left => edges[index],
                        Align::Right => edges[index] + widths[index] - self.measurer.width(text, size, false),
                    };
                    self.canvas.push(Op::Text {
                        at: Point { x, y },
                        size,
                        bold: false,
                        colour,
                        text: text.clone(),
                    });
                    y += line_height;
                }
            }
            self.canvas.cursor = top + height;
            self.canvas.rule(top + height, row_width, rule_ink);
        }
    }

    fn table_header(&mut self, columns: &[Column], headings: &[String], edges: &[f32], widths: &[f32], height: f32) {
        let size = self.template.text.size_pt;
        let top = self.canvas.cursor;
        self.canvas.rule(top, self.template.rules.width_pt, self.rule_ink);
        for (index, heading) in headings.iter().enumerate() {
            let x = match columns[index].align {
                Align::Left => edges[index],
                Align::Right => edges[index] + widths[index] - self.measurer.width(heading, size, true),
            };
            self.canvas.push(Op::Text {
                at: Point {
                    x,
                    y: top + ROW_PADDING,
                },
                size,
                bold: true,
                colour: self.ink,
                text: heading.clone(),
            });
        }
        self.canvas.cursor = top + height;
        self.canvas
            .rule(top + height, self.template.rules.width_pt, self.rule_ink);
    }

    fn totals(&mut self) {
        let size = self.template.text.size_pt;
        let line_height = self.measurer.metrics.line_height(size);
        let rows = [
            (self.words.net, self.data.net.as_str()),
            (self.words.vat, self.data.tax.as_str()),
            (self.words.total, self.data.gross.as_str()),
        ];
        let height = rows.len() as f32 * (line_height + 2.0) + BLOCK_GAP;
        self.canvas.cursor += BLOCK_GAP;
        self.canvas.ensure(height);
        let right = self.canvas.right_edge();
        let mut y = self.canvas.cursor;
        for (index, (label, value)) in rows.iter().enumerate() {
            let bold = index + 1 == rows.len();
            let text = format!("{label} {value} {}", self.data.currency);
            let x = right - self.measurer.width(&text, size, bold);
            self.canvas.push(Op::Text {
                at: Point { x, y },
                size,
                bold,
                colour: self.ink,
                text,
            });
            y += line_height + 2.0;
        }
        self.canvas.cursor = y;
    }

    fn note(&mut self) {
        let Some(note) = &self.data.notes else {
            return;
        };
        if note.trim().is_empty() {
            return;
        }
        let size = self.template.text.size_pt;
        let width = self.canvas.content_width();
        let left = self.canvas.left;
        self.canvas.cursor += BLOCK_GAP;
        let bottom = self.canvas.block(
            &self.measurer,
            left,
            width,
            self.canvas.cursor,
            &Paragraph {
                size,
                colour: self.ink,
                lines: &[(note, false)],
            },
        );
        self.canvas.cursor = bottom;
    }
}

/// Resolves a column's `pt`/`fr` width against the space available.
fn resolve_widths(columns: &[Column], content: f32) -> Vec<f32> {
    let mut fixed = 0.0;
    let mut shares = 0.0;
    for column in columns {
        match column.width.parse::<Width>() {
            Ok(Width::Points(points)) => fixed += points,
            Ok(Width::Share(share)) => shares += share,
            Err(_) => {}
        }
    }
    let remaining = (content - fixed).max(0.0);
    columns
        .iter()
        .map(|column| match column.width.parse::<Width>() {
            Ok(Width::Points(points)) => points,
            Ok(Width::Share(share)) if shares > 0.0 => remaining * share / shares,
            _ => 0.0,
        })
        .collect()
}

/// The x of each column's left edge.
fn column_edges(left: f32, widths: &[f32]) -> Vec<f32> {
    let mut x = left;
    widths
        .iter()
        .map(|width| {
            let at = x;
            x += width;
            at
        })
        .collect()
}

/// The text of one table cell.
fn cell(line: &LineData, field: Field) -> &str {
    match field {
        Field::Item => &line.name,
        Field::Qty => &line.qty,
        Field::UnitPrice => &line.price,
        Field::Vat => &line.vat,
        Field::Total => &line.total,
    }
}

/// Draws the footer on one finished page.
fn footer_ops(template: &Template, data: &InvoiceData, measurer: &Measurer, page: &Page, number: usize) -> Vec<Op> {
    let Some(footer) = &template.footer else {
        return Vec::new();
    };
    let size = footer.size_pt;
    let colour = colour(&template.text.colour).unwrap_or_default();
    let width = page.width - template.page.margin_left_pt - template.page.margin_right_pt;
    let text = substitute(&footer.text, data, number);
    let mut ops = Vec::new();
    let mut y = page.height - template.page.margin_bottom_pt + 6.0;
    for line in measurer.wrap(&text, size, false, width) {
        if line.is_empty() {
            continue;
        }
        ops.push(Op::Text {
            at: Point {
                x: template.page.margin_left_pt,
                y,
            },
            size,
            bold: false,
            colour,
            text: line,
        });
        y += measurer.metrics.line_height(size);
    }
    ops
}

/// Replaces `{{name}}` with the fact it names. Unknown names become empty;
/// templates are validated before they reach here.
fn substitute(text: &str, data: &InvoiceData, page: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        out.push_str(&value(after[..end].trim(), data, page));
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

fn value(name: &str, data: &InvoiceData, page: usize) -> String {
    match name {
        "seller.name" => data.seller.name.clone(),
        "seller.kvk" => data.seller.kvk.clone().unwrap_or_default(),
        "seller.vat" => data.seller.vat.clone().unwrap_or_default(),
        "customer.name" => data.customer.name.clone(),
        "customer.vat" => data.customer.vat.clone().unwrap_or_default(),
        "invoice.number" => data.number.clone(),
        "invoice.issue_date" => data.issue_date.clone(),
        "invoice.due_date" => data.due_date.clone().unwrap_or_default(),
        "totals.net" => data.net.clone(),
        "totals.tax" => data.tax.clone(),
        "totals.gross" => data.gross.clone(),
        "page.number" => page.to_string(),
        _ => String::new(),
    }
}

/// Midnight on the issue date, when it parses.
fn created_from(issue_date: &str) -> Option<Created> {
    use time::format_description::well_known::Iso8601;
    let date = time::Date::parse(issue_date, &Iso8601::DATE).ok()?;
    Some(Created {
        year: u16::try_from(date.year()).unwrap_or(0),
        month: date.month() as u8,
        day: date.day(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{Footer, Template};
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
                kvk: Some("12345678".to_owned()),
                vat: Some("NL123456782B01".to_owned()),
            },
            customer: PartyData {
                name: "Customer BV".to_owned(),
                address: "Rotterdam, NL".to_owned(),
                kvk: None,
                vat: None,
            },
            lines: vec![LineData {
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

    fn texts(document: &Document) -> Vec<String> {
        document
            .pages
            .iter()
            .flat_map(|page| &page.ops)
            .filter_map(|op| match op {
                Op::Text { text, .. } => Some(text.clone()),
                Op::Rule { .. } => None,
            })
            .collect()
    }

    #[test]
    fn colours_are_hex_only() {
        assert_eq!(
            colour("#ff8000"),
            Some(Colour {
                red: 255,
                green: 128,
                blue: 0
            })
        );
        assert_eq!(colour("ff8000"), None);
        assert_eq!(colour("#fff"), None);
        assert_eq!(colour("#gg0000"), None);
    }

    #[test]
    fn a_default_invoice_is_one_page_of_known_facts() {
        let document = layout(&Template::default(), &sample()).expect("layout");
        assert_eq!(document.pages.len(), 1);
        let page = &document.pages[0];
        assert!((page.width - 595.28).abs() < 0.01);
        let found = texts(&document);
        for expected in [
            "Invoice 2026-0001",
            "2026-10-27",
            "Due 2026-11-26",
            "Seller BV",
            "Bill to",
            "Customer BV",
            "Item",
            "Qty",
            "Unit price",
            "VAT",
            "Total",
            "Consulting",
            "Net 100.00 EUR",
            "Total 121.00 EUR",
        ] {
            assert!(
                found.iter().any(|text| text == expected),
                "missing {expected:?} in {found:?}"
            );
        }
        // Every operation stays inside the page.
        for op in &page.ops {
            match op {
                Op::Text { at, .. } => {
                    assert!(at.x >= 0.0 && at.x < page.width, "text off the page: {at:?}");
                    assert!(at.y >= 0.0 && at.y < page.height, "text off the page: {at:?}");
                }
                Op::Rule { from, to, .. } => {
                    assert!(from.y >= 0.0 && to.y < page.height, "rule off the page");
                }
            }
        }
    }

    #[test]
    fn a_long_table_paginates_and_repeats_its_heading() {
        let mut data = sample();
        data.lines = (0..120)
            .map(|index| LineData {
                name: format!("Line {index}"),
                qty: "1".to_owned(),
                price: "10.00".to_owned(),
                vat: "21".to_owned(),
                total: "10.00".to_owned(),
            })
            .collect();
        let document = layout(&Template::default(), &data).expect("layout");
        assert!(document.pages.len() > 1, "120 lines do not fit on one page");
        for (index, page) in document.pages.iter().enumerate() {
            let has_rows = page
                .ops
                .iter()
                .any(|op| matches!(op, Op::Text { text, .. } if text.starts_with("Line ")));
            let headings = page
                .ops
                .iter()
                .filter(|op| matches!(op, Op::Text { text, bold: true, .. } if text == "Item"))
                .count();
            if has_rows {
                assert_eq!(headings, 1, "page {index} has no table heading");
            }
        }
        // Totals live on the last page.
        let last = document.pages.last().expect("a page");
        assert!(
            last.ops
                .iter()
                .any(|op| matches!(op, Op::Text { text, .. } if text == "Total 121.00 EUR")),
            "the totals moved with the table"
        );
    }

    #[test]
    fn a_long_description_wraps_inside_its_column() {
        let mut data = sample();
        data.lines[0].name =
            "A very long description that cannot possibly fit on a single line of the item column".to_owned();
        let document = layout(&Template::default(), &data).expect("layout");
        let found = texts(&document);
        let fragments: Vec<&String> = found
            .iter()
            .filter(|text| text.starts_with("A very long") || text.starts_with("a single"))
            .collect();
        assert_eq!(fragments.len(), 2, "the description wraps: {found:?}");
        assert_eq!(fragments[0], &"A very long description that cannot possibly fit on");
        assert_eq!(fragments[1], &"a single line of the item column");
        assert_eq!(format!("{} {}", fragments[0], fragments[1]), data.lines[0].name);
    }

    #[test]
    fn sections_reorder_the_document() {
        let template = Template {
            sections: vec![Section::Note, Section::Header, Section::Footer],
            ..Template::default()
        };
        let mut data = sample();
        data.notes = Some("Thanks".to_owned());
        let document = layout(&template, &data).expect("layout");
        let found = texts(&document);
        let note = found.iter().position(|text| text == "Thanks").expect("note");
        let header = found
            .iter()
            .position(|text| text == "Invoice 2026-0001")
            .expect("header");
        assert!(note < header, "the note is drawn before the header");
    }

    #[test]
    fn the_footer_uses_placeholders_and_page_numbers() {
        let template = Template {
            footer: Some(Footer {
                text: "{{seller.name}} · KvK {{seller.kvk}} · page {{page.number}}".to_owned(),
                size_pt: 7.5,
            }),
            ..Template::default()
        };
        let mut data = sample();
        data.lines = (0..120)
            .map(|index| LineData {
                name: format!("Line {index}"),
                qty: "1".to_owned(),
                price: "10.00".to_owned(),
                vat: "21".to_owned(),
                total: "10.00".to_owned(),
            })
            .collect();
        let document = layout(&template, &data).expect("layout");
        for (index, page) in document.pages.iter().enumerate() {
            let expected = format!("Seller BV · KvK 12345678 · page {}", index + 1);
            assert!(
                page.ops
                    .iter()
                    .any(|op| matches!(op, Op::Text { text, .. } if text == &expected)),
                "page {} lacks {expected:?}",
                index + 1
            );
        }
    }

    #[test]
    fn a_footer_that_is_not_in_the_sections_is_not_drawn() {
        let template = Template {
            footer: Some(Footer {
                text: "footer text".to_owned(),
                size_pt: 7.5,
            }),
            sections: vec![Section::Header, Section::Lines, Section::Totals],
            ..Template::default()
        };
        let document = layout(&template, &data()).expect("layout");
        assert!(!texts(&document).iter().any(|text| text == "footer text"));
    }

    fn data() -> InvoiceData {
        sample()
    }

    #[test]
    fn the_creation_date_comes_from_the_issue_date() {
        let document = layout(&Template::default(), &sample()).expect("layout");
        assert_eq!(
            document.created,
            Some(Created {
                year: 2026,
                month: 10,
                day: 27
            })
        );
        let mut data = sample();
        data.issue_date = "not a date".to_owned();
        assert_eq!(layout(&Template::default(), &data).expect("layout").created, None);
    }

    #[test]
    fn column_widths_split_the_remainder_by_shares() {
        let columns = Template::default().lines.columns;
        let widths = resolve_widths(&columns, 481.28);
        let expected = [241.28, 50.0, 70.0, 45.0, 75.0];
        assert_eq!(widths.len(), expected.len());
        for (width, expected) in widths.iter().zip(expected) {
            assert!((width - expected).abs() < 0.01, "{width} != {expected}");
        }
        // The edges start at the left margin and tile the content box.
        let edges = column_edges(57.0, &widths);
        assert!((edges[0] - 57.0).abs() < 0.01);
        let last = edges[edges.len() - 1] + widths[widths.len() - 1];
        assert!((last - (57.0 + 481.28)).abs() < 0.01, "{last}");
    }

    #[test]
    fn a_template_with_no_lines_draws_none() {
        let template = Template {
            sections: vec![Section::Totals],
            ..Template::default()
        };
        let document = layout(&template, &sample()).expect("layout");
        assert_eq!(document.pages.len(), 1);
        assert!(texts(&document).iter().all(|text| !text.contains("Consulting")));
    }

    #[test]
    fn wrapping_breaks_a_word_that_cannot_fit() {
        let regular = harfrust::Font::new(FONT_REGULAR, 0).expect("font");
        let measurer = Measurer {
            bold: harfrust::Font::new(FONT_BOLD, 0).expect("font"),
            metrics: Metrics::of(&regular),
            regular,
        };
        let lines = measurer.wrap("abcdefghij", 9.5, false, 10.0);
        assert!(lines.len() > 1, "a ten-letter word does not fit at 10pt");
        for line in lines {
            assert!(measurer.width(&line, 9.5, false) <= 10.0 + 0.001);
        }
    }
}
