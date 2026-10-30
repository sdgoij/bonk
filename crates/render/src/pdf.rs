//! Writing the layout model to PDF.
//!
//! Krilla owns the file format and the fonts; this module only walks the pages
//! and turns each operation into a drawing call. No layout decisions happen
//! here, so the PDF and the window preview cannot disagree about where a line
//! sits.
//!
//! Text is shaped here rather than in `krilla`: the glyphs that were measured
//! during layout are the glyphs that get placed, and `krilla`'s `simple-text`
//! helper (and the unmaintained shaper behind it) stays out of the graph.

#![allow(clippy::float_arithmetic)]

use krilla::Document;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point};
use krilla::metadata::{DateTime, Metadata};
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, Stroke};
use krilla::text::{Font, GlyphId, KrillaGlyph};

use crate::RenderError;
use crate::layout::{Colour, Document as Layout, FONT_BOLD, FONT_REGULAR, Op};
use crate::text::{self, Metrics, Shaped};

/// Writes a laid-out document as PDF bytes.
pub(crate) fn write(document: &Layout) -> Result<Vec<u8>, RenderError> {
    let regular = harfrust::Font::new(FONT_REGULAR, 0).ok_or(RenderError::Font)?;
    let bold = harfrust::Font::new(FONT_BOLD, 0).ok_or(RenderError::Font)?;
    let regular_pdf = Font::new(FONT_REGULAR.into(), 0).ok_or(RenderError::Font)?;
    let bold_pdf = Font::new(FONT_BOLD.into(), 0).ok_or(RenderError::Font)?;
    let metrics = Metrics::of(&regular);

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
                    text: line,
                } => {
                    let (shape_font, pdf_font) = if *is_bold {
                        (&bold, &bold_pdf)
                    } else {
                        (&regular, &regular_pdf)
                    };
                    let shaped = text::shape(shape_font, line);
                    if shaped.is_empty() {
                        continue;
                    }
                    let glyphs = pdf_glyphs(&shaped, line, metrics);
                    surface.set_stroke(None);
                    surface.set_fill(Some(fill(*colour)));
                    surface.draw_glyphs(
                        Point::from_xy(at.x, at.y + metrics.ascent(*size)),
                        &glyphs,
                        pdf_font.clone(),
                        line,
                        *size,
                        false,
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

/// Converts shaped glyphs into the writer's form.
///
/// The text range of a glyph is the span of its cluster: for left-to-right text
/// that is the glyph's own cluster up to the next one, which is what lets the
/// PDF carry a `ToUnicode` map and stay copyable.
fn pdf_glyphs(shaped: &[Shaped], text: &str, metrics: Metrics) -> Vec<KrillaGlyph> {
    let units_per_em = metrics.units_per_em;
    shaped
        .iter()
        .enumerate()
        .map(|(index, glyph)| {
            let start = glyph.cluster.min(text.len());
            let mut end_index = index + 1;
            while shaped.get(end_index).is_some_and(|next| next.cluster == glyph.cluster) {
                end_index += 1;
            }
            let end = shaped
                .get(end_index)
                .map_or(text.len(), |next| next.cluster.min(text.len()))
                .max(start);
            KrillaGlyph::new(
                GlyphId::new(glyph.glyph),
                glyph.advance as f32 / units_per_em,
                glyph.x_offset as f32 / units_per_em,
                glyph.y_offset as f32 / units_per_em,
                glyph.y_advance as f32 / units_per_em,
                start..end,
                None,
            )
        })
        .collect()
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
