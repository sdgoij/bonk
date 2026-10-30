# ADR-009: `krilla` for PDF with our own layout

**Status:** Amended 2026-10-30; supersedes the Typst decision of 2026-10-27

## Context

Factur-X requires a visually correct invoice inside a PDF/A-3 container with an
embedded CII file, XMP metadata, an sRGB output intent, embedded fonts, and
deterministic bytes. Hand-drawing text with a low-level PDF library is weeks of
work; a browser engine is a hundred megabytes and still lacks PDF/A.

The first decision was to embed Typst, which solves typesetting and PDF export
in one Apache-2.0 library. Measured afterwards (`.agents/notes/spikes/pdf-rendering.md`),
that library added 37 MB to a 25 MB binary and 161 packages, most of them for
capabilities an invoice never uses — citations, WASM plugins, HTML export,
bibliographies. None of them can be switched off: Typst is built to be an
application, and embedding it means embedding all of it.

## Decision

Write the PDF with `krilla` (MIT OR Apache-2.0, MSRV 1.92, PDF/A-3 support
including embedded files) and own the layout above it:

- `render::layout` measures the bundled DejaVu Serif with `rustybuzz`, wraps text,
  resolves `pt`/`fr` column widths, and paginates into `Document { pages }` of
  text and rule operations in points, y down.
- `render::pdf` walks that model into `krilla`. No layout decisions happen there,
  so the PDF and the window preview cannot disagree.
- The window draws the same model with egui, using the same font files, which
  removes the PNG rasteriser and its dependencies.

Templates are declarative JSON, not code: page, type, rules, section order, table
columns, label overrides, and a footer with a fixed placeholder table. Unknown
keys are refused and every validation error carries a field path.

PDF/A-3 work in M4 is unchanged in shape: attach the XML, add the XMP and output
intent, and normalise the document id, with veraPDF as the gate.

## Consequences

- The release binary is about 32 MB instead of 62 MB, and the dependency graph is
  161 packages smaller.
- We own measure, wrap, table layout, and page breaks (~900 lines). An invoice is
  a header, a party block, a table, totals, and a note, so the work is bounded.
- We give up Typst's typesetting: no bidirectional layout, no complex-script
  shaping beyond `rustybuzz`, no maths, and no user-programmable templates.
- A template can make an invoice look wrong but cannot make it *be* wrong, cannot
  compute, and cannot read a file. That is a smaller security surface than a
  sandboxed typesetting language.
- Reproducible output is easier: the binary carries one font family (DejaVu Serif
  regular and bold, ~740 KB) instead of Typst's bundled set.

## Alternatives

- **Typst linked in** (the previous decision): correct output, but 37 MB and 161
  packages for capabilities we do not use.
- **Typst CLI**: no binary cost, but issuing depends on an installed tool and the
  bytes depend on the host.
- **`printpdf` + own layout**: comparable size, but no PDF/A support to build on.
- **Headless Chromium / WeasyPrint / wkhtmltopdf**: a runtime to install, and no
  PDF/A.
- **`genpdf`**: unmaintained since 2021.
