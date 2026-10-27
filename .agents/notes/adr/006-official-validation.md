# ADR-006: Official artefacts are the authority, executed by an external validator

**Status:** Accepted, 2026-10-27. Amended 2026-10-27: the native XSLT binding was
replaced by an external oracle after the validation spike
([notes](../spikes/validation.md)).

## Context

EN 16931 conformance is defined by Schematron/XSLT artefacts published by the
standard bodies and OpenPeppol. Hand-reimplementing them is how a product ends
up confidently rejecting valid invoices.

The original decision assumed a thin native binding to libxslt. The spike showed
that assumption is wrong: the published stylesheet is XSLT 2.0 (`xsl:function`,
`for-each-group`, `upper-case`), and libxslt implements XSLT 1.0 only. There is
no embeddable XSLT 2.0 engine in Rust or C that we would ship.

## Decision

Two validation paths, with different jobs:

1. **Live feedback** — a Rust implementation of the high-frequency business
   rules, with messages that point at a field (BT reference). Fast, approximate,
   never the authority.
2. **Authority** — the official artefacts, executed as-is by an external
   validator, in `conformance-official` behind the `official-validation` feature.
   `report` parses the validator's report (rule id, BT reference, XPath,
   assertion text) into typed failures; `oracle` runs the process.

The oracle is a developer and release tool. It never ships: the shipped product
has no JVM dependency and needs no network. `tools/validate-m1.sh` and
`cargo run -p xtask -- conformance <dir>` are how the authority is exercised.

## Consequences

- Correctness is anchored to the source of truth, and artefact updates are
  reviewable diffs in the upstream repositories.
- The authority path needs Java 17+ on the developer and CI machine. That is a
  documented gate dependency, not a runtime requirement.
- Two paths mean two code bases to test; the corpus is shared, and the oracle is
  what a divergence is measured against.
- A gap found by the oracle is a normal outcome, not a failure: BR-CO-25 was
  found this way and implemented the same day.

## Alternatives

- **Pure Rust only**: fast to ship, unverifiable against the official suites.
- **Pure-Rust XPath 2.0 / Schematron engine**: a second product to maintain; its
  bugs would look exactly like compliance.
- **Bundle a JVM**: unacceptable install size and startup cost.
- **Call a hosted validation API**: violates the offline promise.
