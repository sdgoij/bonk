# Spike: what renders the PDF

Question: is Typst the right PDF backend for this application?

Typst gives us everything an invoice needs — typesetting, pagination, fonts,
PDF export — and a great deal an invoice never uses. This note measures what it
costs and what the alternatives cost, so the decision is about numbers instead of
taste.

## What Typst costs

Measured on the Windows MSVC release build:

| | release binary |
|---|---|
| before any PDF renderer (the gui and its stack) | 25 MB |
| with Typst linked in | 62 MB |
| with Typst linked in, fonts removed | 62 MB → 60 MB (fonts were ~10 MB of the 72 MB build before other changes) |

The dependency graph went from 580 to 741 packages. The largest compiled inputs
(rlib size, an upper bound before the linker drops unreachable code):

| crate | rlib | what it is | used here |
|---|---|---|---|
| `typst-library` | 63 MB | the language's standard library and element model | partly |
| `citationberg` | 20 MB | CSL citation styles | **never** |
| `write-fonts` | 17 MB | font subsetting for embedding | yes |
| `wasmi` | 17 MB | a WebAssembly interpreter for Typst plugins | **never** |
| `read-fonts` | 15 MB | OpenType parsing | yes |
| `hayagriva` | 15 MB | bibliography engine | **never** |
| `wasmparser` | 14 MB | WASM validation | **never** |
| `hayro-interpret` | 12 MB | PDF interpreter, for PDF images | no |
| `typst-layout` | 9 MB | line breaking, page breaking, grids | yes |
| `krilla` | 8 MB | PDF writer, tagging, PDF/UA | yes |
| `typst-html` | 5 MB | HTML export | **never** |
| `usvg`/`resvg`/`tiny-skia` | 7 MB | SVG rendering, for the preview | preview only |

None of the unused parts can be switched off: `wasmi` is a required dependency of
`typst-library`, `typst-html` of `typst`, and the citation crates come through
`typst-library` too. Typst is built to be an application; embedding it means
embedding all of it.

## What the PDF-writing part alone costs

A release binary that opens one page, draws text in an embedded font, and
strokes table rules through `krilla`:

| | release binary |
|---|---|
| empty Rust binary, same profile | 0.13 MB |
| `krilla` drawing one invoice-shaped page | **3.4 MB** |

So the PDF writer, font parsing, subsetting and embedding cost about 3.3 MB.
Everything else in Typst — about 33 MB — is the language, the layout engine, the
bibliography, and the plugin runtime.

## Options

| option | binary | what we write | what we give up |
|---|---|---|---|
| Typst linked in (today) | 62 MB | nothing | nothing, except size |
| Typst CLI | 25 MB | nothing | an external tool, and the PDF tests that skip without it |
| **krilla + own layout** | **~28 MB** | measure, wrap, table, pagination (~600–900 lines) | Typst's complete typesetting; templates become code |
| `printpdf` + own layout | ~27 MB | the same | the same, plus no PDF/A support |
| `svg2pdf` | ~28 MB | layout expressed as SVG | awkward, still no text layout |
| `genpdf` | — | little | unmaintained since 2021; `deny` would flag it |
| WeasyPrint / Chromium / wkhtmltopdf | — | little | a runtime to install: the thing we are getting rid of |

`krilla` is the PDF writer Typst itself uses (`typst-pdf` depends on it), by a
Typst contributor, MIT OR Apache-2.0, MSRV 1.92. It documents PDF/A-3 support
including embedded files, which is exactly what Factur-X needs in M4.

## The recommendation

Take `krilla` and write the invoice layout ourselves.

- The preview stops needing a rasteriser: the same layout model can be drawn with
  egui, which the window already links. That drops `typst-render`, `resvg` and
  `hayro` as well.
- A bundled font becomes affordable again: one family at ~500 KB gives
  reproducible output without Typst's 10 MB of bundled faces.
- Factur-X in M4 needs PDF/A-3 with an embedded XML file, which is a documented
  krilla feature rather than something to bolt on.

What it costs us in work is the layout nobody sees: measuring text, wrapping
descriptions, table columns, and page breaks when a line list overflows. That is
the same work for any non-Typst option, and it is bounded — an invoice is a
header, a party block, a table, totals, and a note.

What it costs us in capability is real but narrow: no complex-script shaping
beyond what `rustybuzz` does, no bidirectional layout, no math, no user-editable
templates unless we design them. Our market prints Dutch and English invoices.

## Not decided

The next step is a prototype, not a commit: render one real invoice through
krilla, put it beside the Typst output, and check the layout, the bytes, and the
binary size. If it holds up, the Typst dependency goes and `render` becomes a
layout module plus a PDF writer.
