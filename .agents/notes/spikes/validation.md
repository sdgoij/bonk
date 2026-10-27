# Spike: executing the official EN 16931 artefacts

**Date:** opened 2026-10-27
**Status:** Closed, 2026-10-27 — the artefacts are XSLT 2.0, so a native in-process
binding is not viable; the authority path runs the official validator as an
external process. ADR-006 was amended accordingly.
**Decision it overturned:** ADR-006 (native binding).

## Question

Can we run the official EN 16931 and Peppol BIS Schematron/XSLT artefacts
in-process, offline, on Windows/macOS/Linux, with acceptable packaging cost?

## What was checked

1. The published pipeline is `EN16931-UBL-validation.xslt`, generated from the
   Schematron by ph-schematron (Schematron 1.3.12, last update 2024-04-06), plus
   the codelist Schematron `EN16931-UBL-codes.sch`.
2. The XSLT declares `version="2.0"` and uses `xsl:function`, `for-each-group`,
   and XPath 2.0 functions such as `upper-case()`. Counted 184 uses of the
   XSLT-2.0-only constructs in the 908 KB stylesheet.
3. Candidate native engines: libxslt (XSLT 1.0 only), libxslt + EXSLT (still
   1.0), Xalan-C (1.0). There is no maintained, embeddable XSLT 2.0 engine in
   Rust or C that we would be willing to ship.
4. Mustang 2.17.0 carries the same artefacts inside the jar and runs them under
   Saxon-HE. It is the same authority, reached through a JVM.

## Conclusion

- **Native in-process execution: no.** The premise of ADR-006 ("a thin libxslt
  wrapper") does not hold; libxslt cannot parse the published stylesheet at all.
  Writing an XPath 2.0 subset interpreter to run it would be a second product
  with its own bugs, which is exactly what ADR-006 exists to prevent.
- **Authority path: external process.** `conformance-official::oracle` runs a
  configured validator, and `conformance-official::report` parses its report into
  typed failures (rule id, BT reference, XPath, message). The parser is pure Rust
  and unit-tested against a captured report, so the harness does not depend on
  log scraping.
- **Shipping: unaffected.** The oracle needs Java, so it stays behind the
  `official-validation` feature and is only used by `tools/validate-m1.sh` and
  `cargo run -p xtask -- conformance <dir>`. The product itself keeps working
  offline with the Rust rule engine.

## What the oracle immediately found

`xtask conformance` over the M1 corpus surfaced **BR-CO-25** ("a positive amount
due needs a payment due date or payment terms"), which the Rust engine did not
implement: an invoice issued without a due date passed our validation and failed
the official one. The rule is now implemented and blocking for invoices.

That is the value of the oracle in one line: our engine is fast, but it is not
the authority, and the difference is measurable.

## Follow-ups

- Keep `tools/validate-m1.sh` as the end-to-end gate; `xtask conformance` is the
  reusable validator over any directory.
- When Peppol publishes the next artefact release, re-run both and diff the rule
  ids that fire.
