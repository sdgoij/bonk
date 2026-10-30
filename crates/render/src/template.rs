//! The invoice template: data, not code.
//!
//! A template chooses what the document says and how it is arranged — section
//! order, labels, table columns, margins, fonts, colours, and a footer with
//! placeholders. It cannot compute, loop, or read a file: the engine owns
//! wrapping, page breaks, and totals, so a template can make an invoice look
//! wrong but never make it *be* wrong.
//!
//! Unknown keys are refused rather than ignored. A typo in a template is
//! otherwise a setting that silently does nothing, which is worse than an error.

#![allow(clippy::float_arithmetic)]

use serde::Deserialize;

use crate::RenderError;

/// The template schema this binary understands.
pub const SCHEMA_VERSION: u32 = 1;

/// A complete invoice template.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Template {
    /// Schema version; a template from the future is refused, not guessed at.
    pub version: u32,
    /// Human-readable name, for the picker.
    pub name: String,
    /// Page size and margins.
    pub page: PageStyle,
    /// Type.
    pub text: TextStyle,
    /// Rules and separators.
    pub rules: RuleStyle,
    /// Which blocks the document draws, in order.
    pub sections: Vec<Section>,
    /// The lines table.
    pub lines: LineStyle,
    /// Words the template overrides.
    pub labels: LabelOverrides,
    /// The footer, when the template has one.
    pub footer: Option<Footer>,
}

impl Default for Template {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            name: "Classic".to_owned(),
            page: PageStyle::default(),
            text: TextStyle::default(),
            rules: RuleStyle::default(),
            sections: vec![
                Section::Header,
                Section::Parties,
                Section::Lines,
                Section::Totals,
                Section::Note,
                Section::Footer,
            ],
            lines: LineStyle::default(),
            labels: LabelOverrides::default(),
            footer: None,
        }
    }
}

impl Template {
    /// Parses a template, mapping syntax errors to a source location.
    pub fn parse(json: &str) -> Result<Self, RenderError> {
        let template: Self = serde_json::from_str(json).map_err(|error| RenderError::Template {
            field: format!("line {}, column {}", error.line(), error.column()),
            message: error.to_string(),
        })?;
        template.validate()?;
        Ok(template)
    }

    /// Checks what the type system cannot: version, colours, widths, placeholders.
    pub fn validate(&self) -> Result<(), RenderError> {
        if self.version != SCHEMA_VERSION {
            return Err(RenderError::Template {
                field: "version".to_owned(),
                message: format!(
                    "this build understands schema version {SCHEMA_VERSION}, not version {}",
                    self.version
                ),
            });
        }
        if self.text.family.is_empty() {
            return Err(RenderError::Template {
                field: "text.family".to_owned(),
                message: "a template needs at least one font family".to_owned(),
            });
        }
        if self.page.width_pt <= 0.0 || self.page.height_pt <= 0.0 {
            return Err(RenderError::Template {
                field: "page".to_owned(),
                message: "the page needs a positive width and height".to_owned(),
            });
        }
        if self.page.margin_left_pt + self.page.margin_right_pt >= self.page.width_pt
            || self.page.margin_top_pt + self.page.margin_bottom_pt >= self.page.height_pt
        {
            return Err(RenderError::Template {
                field: "page".to_owned(),
                message: "the margins leave no room for content".to_owned(),
            });
        }
        for (field, value) in [("text.colour", &self.text.colour), ("rules.colour", &self.rules.colour)] {
            if crate::layout::colour(value).is_none() {
                return Err(RenderError::Template {
                    field: field.to_owned(),
                    message: format!("{value:?} is not a colour; write \"#rrggbb\""),
                });
            }
        }
        if self.lines.columns.is_empty() {
            return Err(RenderError::Template {
                field: "lines.columns".to_owned(),
                message: "a table needs at least one column".to_owned(),
            });
        }
        for (index, column) in self.lines.columns.iter().enumerate() {
            column.width.parse::<Width>().map_err(|message| RenderError::Template {
                field: format!("lines.columns[{index}].width"),
                message,
            })?;
        }
        for (index, section) in self.sections.iter().enumerate() {
            if self.sections[..index].contains(section) {
                return Err(RenderError::Template {
                    field: format!("sections[{index}]"),
                    message: "each section appears at most once".to_owned(),
                });
            }
        }
        if let Some(footer) = &self.footer {
            for name in placeholders(&footer.text) {
                if !crate::layout::PLACEHOLDERS.contains(&name.as_str()) {
                    return Err(RenderError::Template {
                        field: "footer.text".to_owned(),
                        message: format!(
                            "unknown placeholder {{{{{name}}}}}; known ones are {}",
                            crate::layout::PLACEHOLDERS.join(", ")
                        ),
                    });
                }
            }
        }
        Ok(())
    }
}

/// Everything a template names, with the built-in values.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PageStyle {
    /// Width in points. A4 is 595.28.
    pub width_pt: f32,
    /// Height in points. A4 is 841.89.
    pub height_pt: f32,
    /// Inset from the top edge.
    pub margin_top_pt: f32,
    /// Inset from the right edge.
    pub margin_right_pt: f32,
    /// Inset from the bottom edge.
    pub margin_bottom_pt: f32,
    /// Inset from the left edge.
    pub margin_left_pt: f32,
}

impl Default for PageStyle {
    fn default() -> Self {
        Self {
            width_pt: 595.28,
            height_pt: 841.89,
            margin_top_pt: 51.0,
            margin_right_pt: 57.0,
            margin_bottom_pt: 51.0,
            margin_left_pt: 57.0,
        }
    }
}

/// Type for the document.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TextStyle {
    /// Families in preference order; the first one the renderer has is used.
    pub family: Vec<String>,
    /// Body size.
    pub size_pt: f32,
    /// Title size.
    pub title_size_pt: f32,
    /// Body colour, `#rrggbb`.
    pub colour: String,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: vec!["DejaVu Serif".to_owned()],
            size_pt: 9.5,
            title_size_pt: 18.0,
            colour: "#111111".to_owned(),
        }
    }
}

/// Rules and separators.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct RuleStyle {
    /// Rule colour, `#rrggbb`.
    pub colour: String,
    /// Rule thickness.
    pub width_pt: f32,
}

impl Default for RuleStyle {
    fn default() -> Self {
        Self {
            colour: "#999999".to_owned(),
            width_pt: 0.5,
        }
    }
}

/// A block of the document.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    /// Title, number, and dates.
    Header,
    /// Seller and customer blocks.
    Parties,
    /// The lines table.
    Lines,
    /// Totals.
    Totals,
    /// The free-text note.
    Note,
    /// The footer.
    Footer,
}

/// The lines table.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LineStyle {
    /// Columns, left to right.
    pub columns: Vec<Column>,
}

impl Default for LineStyle {
    fn default() -> Self {
        Self {
            columns: vec![
                Column::new(Field::Item, "1fr"),
                Column::new(Field::Qty, "50pt"),
                Column::new(Field::UnitPrice, "70pt"),
                Column::new(Field::Vat, "45pt"),
                Column::new(Field::Total, "75pt"),
            ],
        }
    }
}

/// One column of the lines table.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    /// Which fact the column shows.
    pub field: Field,
    /// `"<n>pt"` for a fixed width, `"<n>fr"` for a share of what is left.
    pub width: String,
    /// Text alignment.
    #[serde(default)]
    pub align: Align,
}

impl Column {
    fn new(field: Field, width: &str) -> Self {
        Self {
            field,
            width: width.to_owned(),
            align: Align::default(),
        }
    }
}

/// A fact a column can show.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// Item name.
    Item,
    /// Quantity and unit.
    Qty,
    /// Unit price.
    UnitPrice,
    /// VAT rate.
    Vat,
    /// Line net.
    Total,
}

/// Horizontal alignment.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    /// Left.
    #[default]
    Left,
    /// Right.
    Right,
}

/// A parsed column width.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Width {
    /// A fixed width in points.
    Points(f32),
    /// A share of the width left after the fixed columns.
    Share(f32),
}

impl std::str::FromStr for Width {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        let (number, unit) = value.split_at(value.len().saturating_sub(2));
        let amount: f32 = number
            .trim()
            .parse()
            .map_err(|_| format!("{value:?} is not a width; write \"60pt\" or \"1fr\""))?;
        if !amount.is_finite() || amount <= 0.0 {
            return Err(format!("{value:?} must be greater than zero"));
        }
        match unit {
            "pt" => Ok(Self::Points(amount)),
            "fr" => Ok(Self::Share(amount)),
            _ => Err(format!("{value:?} is not a width; write \"60pt\" or \"1fr\"")),
        }
    }
}

/// Words the template overrides.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LabelOverrides {
    /// Replaces the "Due" label.
    pub due: Option<String>,
    /// Replaces the "Bill to" label.
    pub bill_to: Option<String>,
    /// Replaces the "Net" label.
    pub net: Option<String>,
    /// Replaces the "VAT" label.
    pub vat: Option<String>,
    /// Replaces the "Total" label.
    pub total: Option<String>,
    /// Replaces the "Item" heading.
    pub item: Option<String>,
    /// Replaces the "Qty" heading.
    pub qty: Option<String>,
    /// Replaces the "Unit price" heading.
    pub unit_price: Option<String>,
}

/// The footer line.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Footer {
    /// Text with `{{placeholders}}`.
    pub text: String,
    /// Footer size.
    #[serde(default = "default_footer_size")]
    pub size_pt: f32,
}

fn default_footer_size() -> f32 {
    7.5
}

/// Names inside `{{...}}` in a string.
pub fn placeholders(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        found.push(after[..end].trim().to_owned());
        rest = &after[end + 2..];
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_template_matches_the_schema() {
        let template = Template::default();
        assert_eq!(template.version, SCHEMA_VERSION);
        template.validate().unwrap();
        assert_eq!(template.sections.first(), Some(&Section::Header));
    }

    #[test]
    fn widths_are_points_or_shares() {
        assert_eq!("60pt".parse::<Width>().unwrap(), Width::Points(60.0));
        assert_eq!("1fr".parse::<Width>().unwrap(), Width::Share(1.0));
        assert!("60".parse::<Width>().is_err());
        assert!("0pt".parse::<Width>().is_err());
        assert!("-5pt".parse::<Width>().is_err());
    }

    #[test]
    fn a_template_from_the_future_is_refused() {
        let error = Template::parse(r#"{"version": 2}"#).unwrap_err();
        assert!(error.to_string().contains("version 2"), "{error}");
    }

    #[test]
    fn unknown_keys_and_placeholders_are_errors() {
        let typo = Template::parse(r#"{"text": {"size_pts": 12}}"#).unwrap_err();
        assert!(typo.to_string().contains("size_pts"), "{typo}");

        let unknown = Template::parse(r#"{"footer": {"text": "KvK {{seller.kvk}} {{nope}}"}}"#).unwrap_err();
        assert!(unknown.to_string().contains("nope"), "{unknown}");
    }

    #[test]
    fn placeholders_are_found_in_order() {
        assert_eq!(placeholders("a {{one}} b {{ two }}"), vec!["one", "two"]);
        assert!(placeholders("no braces").is_empty());
        assert!(placeholders("unclosed {{oops").is_empty());
    }

    #[test]
    fn a_section_may_not_repeat() {
        let error = Template::parse(r#"{"sections": ["lines", "lines"]}"#).unwrap_err();
        assert!(error.to_string().contains("sections[1]"), "{error}");
    }
}
