# ADR-009: Typst layout with PDF/A-3 post-processing

**Status:** Accepted provisionally, 2026-10-27 (confirmed by the M0 PDF spike)

## Context

Factur-X requires a visually correct invoice inside a PDF/A-3 container with an
embedded CII file, XMP metadata, an sRGB output intent, embedded fonts, and
deterministic bytes. Hand-drawing text with a low-level PDF library is weeks of
work; a browser engine is a hundred megabytes and still lacks PDF/A.

## Decision

Render layout with Typst (Apache-2.0, embeddable), using bundled fonts and a
restricted `World` that exposes no filesystem or network to templates. Then
attach the XML, add the XMP and output intent, and normalise the document id in
a post-processing step. `bonk-render` owns this pipeline; `veraPDF` gates the
output in CI.

Preview uses `pdfium-render` (BSD/MIT); MuPDF is rejected for licensing.

## Consequences

- Professional layout and pagination without a browser dependency.
- PDF/A-3 conformance still requires deliberate post-processing, which is the
  riskiest part of M4 and gets its own spike and test gate.
- Templates are Typst source, sandboxed, and hashed into the artefact record.

## Alternatives

- **Headless Chromium**: large, slow start, and no PDF/A support.
- **MuPDF**: AGPL, unusable in a commercial product.
- **`printpdf` alone**: viable for simple layouts, but table and pagination work
  would be ours to write.
