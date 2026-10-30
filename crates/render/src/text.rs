//! Shaping the bundled font with `harfrust`.
//!
//! One shaper serves both jobs: the layout measures with it, and the PDF writer
//! places the glyphs it returns. That is also why `krilla`'s `simple-text`
//! helper is switched off — it shapes with `rustybuzz`, which is unmaintained
//! (RUSTSEC-2026-0206), and shaping here keeps that crate out of the graph.

#![allow(clippy::float_arithmetic)]

use harfrust::{Buffer, Font, ShapeOptions, ShaperFont};

/// Font metrics in font units.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Metrics {
    /// Design units per em.
    pub units_per_em: f32,
    /// Height above the baseline.
    pub ascender: f32,
    /// Depth below the baseline, negative.
    pub descender: f32,
}

impl Metrics {
    /// Reads the metrics of `font`, falling back to a typical serif profile.
    pub(crate) fn of(font: &Font) -> Self {
        let metrics = font.metrics();
        let units_per_em = f32::from(metrics.units_per_em);
        let (ascender, descender) = metrics
            .hhea_line
            .map_or((0.8 * units_per_em, -0.2 * units_per_em), |line| {
                (line.ascender.to_f64() as f32, line.descender.to_f64() as f32)
            });
        Self {
            units_per_em,
            ascender,
            descender,
        }
    }

    /// Distance between two baselines at `size`.
    pub(crate) fn line_height(self, size: f32) -> f32 {
        (self.ascender - self.descender) / self.units_per_em * size
    }

    /// Distance from the top of a line to its baseline at `size`.
    pub(crate) fn ascent(self, size: f32) -> f32 {
        self.ascender / self.units_per_em * size
    }
}

/// One positioned glyph, in font units.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shaped {
    /// Glyph identifier in the font.
    pub glyph: u32,
    /// Byte offset of the cluster the glyph belongs to.
    pub cluster: usize,
    /// Horizontal advance.
    pub advance: i32,
    /// Horizontal offset.
    pub x_offset: i32,
    /// Vertical offset.
    pub y_offset: i32,
    /// Vertical advance, zero for horizontal text.
    pub y_advance: i32,
}

/// Shapes one line of text. An error yields no glyphs rather than failing the
/// document: the buffer is always given a direction, so this is unreachable in
/// practice.
pub(crate) fn shape(font: &Font, text: &str) -> Vec<Shaped> {
    if text.is_empty() {
        return Vec::new();
    }
    let shaper = ShaperFont::new(font);
    let mut buffer = Buffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    if harfrust::shape(&shaper, &mut buffer, ShapeOptions::new()).is_err() {
        return Vec::new();
    }
    buffer
        .glyph_infos()
        .iter()
        .zip(buffer.glyph_positions())
        .map(|(info, position)| Shaped {
            glyph: info.glyph_id,
            cluster: info.cluster as usize,
            advance: position.x_advance,
            x_offset: position.x_offset,
            y_offset: position.y_offset,
            y_advance: position.y_advance,
        })
        .collect()
}

/// The advance width of `text` at `size`.
pub(crate) fn width(font: &Font, metrics: Metrics, text: &str, size: f32) -> f32 {
    let units: i32 = shape(font, text).iter().map(|glyph| glyph.advance).sum();
    units as f32 / metrics.units_per_em * size
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{FONT_BOLD, FONT_REGULAR};

    fn regular() -> Font {
        Font::new(FONT_REGULAR, 0).expect("bundled font")
    }

    #[test]
    fn metrics_come_from_the_font() {
        let metrics = Metrics::of(&regular());
        assert!((metrics.units_per_em - 2048.0).abs() < 0.01, "{metrics:?}");
        assert!(metrics.ascender > 1500.0, "{metrics:?}");
        assert!(metrics.descender < 0.0, "{metrics:?}");
        assert!(metrics.line_height(10.0) > 10.0);
        assert!(metrics.ascent(10.0) < metrics.line_height(10.0));
    }

    #[test]
    fn longer_text_is_wider() {
        let font = regular();
        let metrics = Metrics::of(&font);
        let narrow = width(&font, metrics, "iiii", 10.0);
        let wide = width(&font, metrics, "WWWW", 10.0);
        assert!(wide > narrow, "{wide} <= {narrow}");
        assert_eq!(width(&font, metrics, "", 10.0), 0.0);
    }

    #[test]
    fn shaping_preserves_the_text_as_clusters() {
        let shaped = shape(&regular(), "Hello");
        assert_eq!(shaped.len(), 5);
        assert_eq!(shaped[0].cluster, 0);
        assert!(shaped.iter().all(|glyph| glyph.advance > 0));
    }

    #[test]
    fn clusters_are_byte_offsets() {
        // The PDF writer slices the source text by cluster, so a multi-byte
        // character must not shift the offsets.
        let shaped = shape(&regular(), "a·b");
        let clusters: Vec<usize> = shaped.iter().map(|glyph| glyph.cluster).collect();
        assert_eq!(clusters, vec![0, 1, 3], "{clusters:?}");
        let text = "a·b";
        for (index, glyph) in shaped.iter().enumerate() {
            let end = shaped.get(index + 1).map_or(text.len(), |next| next.cluster);
            assert_eq!(&text[glyph.cluster..end], ["a", "·", "b"][index]);
        }
    }

    #[test]
    fn bold_is_wider_than_regular() {
        let regular = regular();
        let bold = Font::new(FONT_BOLD, 0).expect("bundled font");
        let metrics = Metrics::of(&regular);
        assert!(width(&bold, metrics, "Invoice", 10.0) > width(&regular, metrics, "Invoice", 10.0));
    }
}
