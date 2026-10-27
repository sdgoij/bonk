# Spike: executing the official EN 16931 artefacts

**Date:** opened 2026-10-27
**Status:** Open. Runs before M2; no conclusion yet.
**Decision it can overturn:** ADR-006.

## Question

Can we run the official EN 16931 and Peppol BIS Schematron/XSLT artefacts
in-process, offline, on Windows/macOS/Linux, with acceptable packaging cost?

## Procedure

1. Take the official EN 16931 validation artefacts (Schematron plus the
   XSLT that compiles/executes it) and a Peppol BIS Billing 3.0 artefact set.
2. Run them over five known-good and five known-bad invoices using:
   - **A.** `libxslt` through a thin FFI wrapper, statically linked;
   - **B.** a pure-Rust Schematron engine, compared against A;
   - **C.** the Java oracle (Mustang/KoSIT) in CI only, as the reference.
3. Measure: startup cost, time per invoice, binary size delta, and build
   complexity per platform (especially MSVC static linking and macOS
   notarisation).
4. Verify that the SVRL output can be mapped to `Finding { code, bt, path }`
   without losing the rule identifier.

## Decision criteria

- **Go (A)**: identical findings to C on the corpus, < 150 ms per invoice,
  packaging documented for all three OSes.
- **Go with isolation (A behind a feature)**: identical findings, but packaging
  is heavy → keep it out of the default build and ship a "full validation"
  build for release.
- **Fallback**: pure-Rust rules for everything blocking, with the official
  artefacts used as a CI oracle; accept that some obscure rules are only caught
  at export time by an external validator.

## Notes

- XML parsers must disable DTD and external entity resolution; the artefacts
  themselves are trusted, the invoices are not.
- Artefact versions change; the harness must report the version used in every
  finding set, and a fixture must pin expected results per version.
- A JVM dependency in the shipped product is not an option.
