//! The bundled fonts: one serif family, embedded compressed.
//!
//! Noto Serif CJK covers Latin, Greek, Cyrillic, kana, hangul, and the unified
//! ideographs, so an invoice from a supplier in any of those scripts still draws —
//! and PDF/A refuses a `.notdef`, so coverage is a correctness question rather
//! than a typographic one. Two weights of that family are 50 MB raw; brotli takes
//! them to about two thirds of that, so they are embedded compressed and inflated
//! once, on first use.
//!
//! Inflating a face takes about a quarter of a second in a release build, and the
//! pair is inflated once per process. `warm` exists so a front end can pay that
//! before it needs the font rather than in the middle of a frame.

use std::io::Read;
use std::sync::OnceLock;

use crate::RenderError;

/// The body font, brotli-compressed.
static REGULAR_BR: &[u8] = include_bytes!("../fonts/NotoSerifCJKsc-Regular.otf.br");

/// The bold font, brotli-compressed.
static BOLD_BR: &[u8] = include_bytes!("../fonts/NotoSerifCJKsc-Bold.otf.br");

static REGULAR: OnceLock<Result<Vec<u8>, ()>> = OnceLock::new();
static BOLD: OnceLock<Result<Vec<u8>, ()>> = OnceLock::new();

/// The body font, inflated on first use.
///
/// The bytes come from this binary, so a failure here is a broken build rather
/// than a condition to design around; it is still returned as an error instead of
/// panicking, because a renderer that panics takes the archive's window with it.
pub fn regular() -> Result<&'static [u8], RenderError> {
    inflate(&REGULAR, REGULAR_BR)
}

/// The bold font, inflated on first use.
pub fn bold() -> Result<&'static [u8], RenderError> {
    inflate(&BOLD, BOLD_BR)
}

/// Inflates both faces, for a front end that would rather pay on startup than on
/// the first frame that draws a document.
pub fn warm() {
    let _ = regular();
    let _ = bold();
}

/// How large a face inflates to. The sources are about this size; the hint only
/// saves the growth steps, and a wrong hint costs nothing but a reallocation.
const FACE_BYTES: usize = 26 * 1024 * 1024;

fn inflate(
    cell: &'static OnceLock<Result<Vec<u8>, ()>>,
    compressed: &'static [u8],
) -> Result<&'static [u8], RenderError> {
    cell.get_or_init(|| {
        let mut out = Vec::with_capacity(FACE_BYTES);
        brotli_decompressor::Decompressor::new(compressed, 8192)
            .read_to_end(&mut out)
            .map(|_| out)
            .map_err(|_| ())
    })
    .as_deref()
    .map_err(|()| RenderError::Font)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_faces_inflate_to_a_font_harfrust_can_parse() {
        for face in [regular(), bold()] {
            let bytes = face.expect("the embedded faces are intact");
            assert!(bytes.len() > 1_000_000, "a CJK face is megabytes, not kilobytes");
            assert!(
                harfrust::Font::new(bytes, 0).is_some(),
                "the inflated bytes are not a font"
            );
        }
    }

    #[test]
    fn inflating_twice_is_inflating_once() {
        // The second call must hand back the same allocation: a face is inflated
        // per process, not per document.
        let first = regular().unwrap().as_ptr();
        let second = regular().unwrap().as_ptr();
        assert_eq!(first, second);
    }
}
