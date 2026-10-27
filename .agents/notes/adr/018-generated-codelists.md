# ADR-018: Code lists are generated from the official Schematron, not hand-maintained

**Status:** Accepted, 2026-10-27

## Context

EN 16931 code lists (currency, country, VAT category, VATEX, units, EAS,
document type) change between releases. The rule engine needs them for offline
validation, so they ship inside the binary. Hand-copying a 2 000-entry unit list
produces two failure modes: a missing entry rejects a valid invoice (a false
positive, which is how users learn to ignore warnings), and a stale entry
accepts an invalid one (which is discovered by the recipient, not by us).

The official codelist Schematron already contains each list as a
space-delimited literal inside the assertion that uses it. That file is the
authority; a human transcription is only a lossy copy of it.

## Decision

Code lists are **extracted mechanically** from `EN16931-UBL-codes.sch` by
`cargo run -p xtask -- codelists` and committed as data under
`crates/conformance/codelists/<bundle>/`, one file per list plus a
`manifest.json`.

- The extraction source is pinned twice: by SHA-256 in `xtask/src/codelists.rs`
  and by the bundle label. Extraction refuses a different file unless
  `--accept-new-source` is passed, so an upstream change is a deliberate act,
  not a silent rewrite.
- The manifest records the upstream URL, the retrieval date, the source hash,
  and a SHA-256 plus entry count per file.
- `conformance` embeds the files with `include_str!` and parses each list once,
  on first use. Nothing is downloaded at runtime or at build time.
- A test recomputes every file hash and entry count against the manifest, so a
  corrupted or stale bundle fails in CI. `cargo run -p xtask -- codelists
  --check` reproduces that check without a build of the test suite.

## Consequences

- A release is a reviewable diff of plain text plus a manifest, not an
  unverifiable claim that "the codelists were updated".
- Extraction requires the pinned upstream file on disk
  (`tools/fetch-codelists.sh`). Reproducing a bundle therefore needs network
  once, but building and validating never does.
- The bundle is only as fresh as its label. We must schedule a check before each
  release rather than assume it stays current; the procedure is in
  [`../codelist-updates.md`](../codelist-updates.md).
- Lists we do not yet validate (payment means, ISO 6523 ICD, charge reasons)
  are not vendored. They are added on demand, with the same mechanism, when the
  rule that needs them is implemented.

## Alternatives

- **Hand-maintained `&[&str]` tables in Rust**: simplest to write, but a full
  unit list is unreadable in review and impossible to diff against upstream.
- **Download at build time**: breaks offline builds, makes the artefact depend
  on a moving branch, and hides the update in a build log.
- **Download at first run**: violates the local-first guarantee and makes
  validation results depend on when the user last had network.
- **Depend on a codelist crate**: no maintained crate tracks EN 16931 releases
  at a fidelity we can defend; we would still need to verify it against the
  Schematron.
