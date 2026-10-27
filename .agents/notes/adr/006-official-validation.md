# ADR-006: Official validation artefacts behind an isolated native binding

**Status:** Accepted, 2026-10-27 (implementation pending the M0 spike)

## Context

EN 16931 conformance is defined by Schematron/XSLT artefacts published by the
standard bodies and OpenPeppol. Hand-reimplementing them is how a product ends
up confidently rejecting valid invoices.

## Decision

Two validation paths, with different jobs:

1. **Live feedback** — a Rust implementation of the high-frequency business
   rules, with messages that point at a field (BT reference). Fast, approximate,
   never the authority.
2. **Authority** — the official artefacts, executed as-is, in a separate crate
   (`conformance-official`) behind the `official-validation` feature. Artifacts
   are vendored with a manifest of source URL, version, and SHA-256.

A Java oracle (Mustang/KoSIT) is used in CI only, to differential-test our
results. It never ships.

## Consequences

- Correctness is anchored to the source of truth, and artefact updates are
  reviewable diffs.
- The native XSLT dependency complicates packaging on Windows and macOS; that is
  the price of not inventing rule semantics.
- Two paths mean two code bases to test; the corpus is shared and the oracle
  compares them.

## Alternatives

- **Pure Rust only**: fast to ship, unverifiable against the official suites.
- **Bundle a JVM**: unacceptable install size and startup cost.
- **Call a hosted validation API**: violates the offline promise.
