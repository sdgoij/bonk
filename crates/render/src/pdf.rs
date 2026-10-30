//! Writing the layout model to PDF.
//!
//! Krilla owns the file format and the fonts; this module only walks the pages
//! and turns each operation into a drawing call. No layout decisions happen
//! here, so the PDF and the window preview cannot disagree about where a line
//! sits.

#![allow(clippy::float_arithmetic)]

use krilla::Document;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point};
use krilla::metadata::{DateTime, Metadata};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, Stroke};
use krilla::text::{Font, TextDirection};

use crate::RenderError;
use crate::layout::{Colour, Document as Layout, FONT_BOLD, FONT_REGULAR, Op, ascent};

/// Writes a laid-out document as PDF bytes.
pub(crate) fn write(document: &Layout) -> Result<Vec<u8>, RenderError> {
    let regular = Font::new(FONT_REGULAR.into(), 0).ok_or(RenderError::Font)?;
    let bold = Font::new(FONT_BOLD.into(), 0).ok_or(RenderError::Font)?;

    let mut pdf = Document::new();
    let mut metadata = Metadata::new()
        .title(document.title.clone())
        .creator("bonk".to_owned())
        .producer(format!("bonk {}", crate::RENDERER_VERSION));
    if let Some(created) = document.created {
        metadata = metadata.creation_date(DateTime::new(created.year).month(created.month).day(created.day));
    }
    pdf.set_metadata(metadata);

    for page in &document.pages {
        let settings = PageSettings::from_wh(page.width, page.height)
            .ok_or_else(|| RenderError::Pdf("the page size is not a positive, finite number".to_owned()))?;
        let mut pdf_page = pdf.start_page_with(settings);
        let mut surface = pdf_page.surface();
        surface.set_fill(None);
        surface.set_stroke(None);

        for op in &page.ops {
            match op {
                Op::Text {
                    at,
                    size,
                    bold: is_bold,
                    colour,
                    text,
                } => {
                    surface.set_stroke(None);
                    surface.set_fill(Some(fill(*colour)));
                    let font = if *is_bold { bold.clone() } else { regular.clone() };
                    surface.draw_text(
                        Point::from_xy(at.x, at.y + ascent(*size)),
                        font,
                        *size,
                        text,
                        false,
                        TextDirection::Auto,
                    );
                }
                Op::Rule {
                    from,
                    to,
                    width,
                    colour,
                } => {
                    surface.set_fill(None);
                    surface.set_stroke(Some(Stroke {
                        paint: paint(*colour),
                        width: *width,
                        ..Stroke::default()
                    }));
                    let mut path = PathBuilder::new();
                    path.move_to(from.x, from.y);
                    path.line_to(to.x, to.y);
                    if let Some(path) = path.finish() {
                        surface.draw_path(&path);
                    }
                }
            }
        }

        surface.finish();
        pdf_page.finish();
    }

    pdf.finish().map_err(|error| RenderError::Pdf(error.to_string()))
}

fn paint(colour: Colour) -> krilla::paint::Paint {
    rgb::Color::new(colour.red, colour.green, colour.blue).into()
}

fn fill(colour: Colour) -> Fill {
    Fill {
        paint: paint(colour),
        opacity: NormalizedF32::ONE,
        rule: FillRule::default(),
    }
}
