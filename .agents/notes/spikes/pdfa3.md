# Spike: PDF/A-3 and Factur-X from Typst

**Date:** opened 2026-10-27
**Status:** Open. Runs before M4; no conclusion yet.
**Decision it can overturn:** ADR-009.

## Question

Can we take a Typst-rendered invoice and produce a file that veraPDF accepts as
PDF/A-3, with the CII XML attached as an alternative representation and XMP
declaring the Factur-X profile — deterministically, on all three platforms?

## Why it matters

Factur-X is a hard requirement for a subset of counterparties, and PDF/A-3
conformance is not something you can eyeball. If the Typst output cannot be
post-processed into conformance, the renderer choice changes before M4, not
during it.

## Procedure

1. Compile a two-page invoice with Typst using embedded fonts; record output
   size and render time.
2. Inspect the produced PDF: font embedding, `ToUnicode`, colour spaces,
   transparency, document id.
3. Post-process with `lopdf`/`pdf-writer`:
   - add `/AFRelationship /Alternative` and an `EmbeddedFiles` name-tree entry;
   - add `/AF` to the catalog;
   - add XMP (`pdfaid:part=3`, `pdfaid:conformance=B`, plus `fx:` fields);
   - add an sRGB `OutputIntent` with a bundled ICC profile;
   - replace the random document id with one derived from the content hash.
4. Validate with veraPDF (container) for `PDF/A-3B` and `PDF/A-3U`.
5. Hash the output twice on two machines; bytes must match.

## Decision criteria

- **Go**: zero veraPDF failures on all templates × profiles, deterministic
  bytes, render under 200 ms for a 3-page invoice.
- **Adjust**: post-processing is possible but fragile → restrict template
  features (no transparency, no gradients, limited image formats) and document
  the constraint.
- **Fallback**: generate the PDF directly with `printpdf` and accept slower
  template work.

## Known risks

- Typst's PDF export may already set metadata that conflicts with PDF/A rules.
- Font subsetting must produce valid `ToUnicode` maps for Dutch and Polish
  diacritics, and for CJK customer names.
- Determinism can break on embedded timestamps; every source must be replaced
  with a value derived from the invoice.
