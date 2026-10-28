# ADR-019: Read Factur-X containers with lopdf, never the visual layer

**Status:** Accepted, 2026-10-28

## Context

Factur-X and ZUGFeRD deliver an EN 16931 invoice as an XML attachment inside a
PDF/A-3 container. A received invoice has to enter the payable register without
the user extracting anything by hand, and the file is untrusted: it may be
malformed, encrypted, or crafted to exhaust memory.

M3.1 gave the project UBL and CII readers that share one bounded XML tree. What
was missing was getting the XML out of the container.

## Decision

`bonk-import` reads PDFs with `lopdf` (MIT, pure Rust). It locates the attachment
through the document catalog's `/EmbeddedFiles` name tree, falls back to a scan
of `/Type /EmbeddedFile` streams for producers that omit the tree, decodes the
stream under a fixed size bound, and cross-checks the XMP packet's
`fx:DocumentFileName` and `fx:DocumentType` against what it actually found.

It never reads the page content. The visual layer is a rendering, not the
invoice; treating it as a source would invent terms the sender never wrote in the
XML, and OCR is not a plan of record for v1.

Encrypted containers are refused rather than guessed at. Bounds apply to the
input size, the number of attachments, the depth of the name tree, and every
decoded stream, so a small hostile file cannot cost unbounded memory.

## Consequences

- PDF imports produce the same record as XML imports: the parsed terms in
  `document`/`document_line`, the PDF bytes as received in `blob`, and
  `source_kind = 'pdf'` in `import_record`.
- The same invoice arriving once as XML and once inside a PDF is caught by the
  supplier-number check, not by the payload hash, which differs by container.
- The dependency brings `flate2`, `aes`, `cbc`, and `encoding_rs` into the build.
  It is used only on the import path.
- Extraction is not validation. The embedded XML still goes through the CII or
  UBL reader and the usual findings; a container without readable XML is
  rejected rather than approximated.
- Visual-only PDFs — scans, print pipelines — stay out of scope.

## Alternatives

- **`pdfium-render`**: already chosen for preview (ADR-009), but it is a C++
  binding aimed at rendering; extraction through it would be indirect and heavier.
- **Hand-rolled PDF parser**: a PDF parser is not the product; the maintenance
  risk is worse than the dependency.
- **Text extraction from the page content**: rejected above; it is a second,
  lossy representation of the invoice with no authority.
