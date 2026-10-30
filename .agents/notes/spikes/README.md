# M0 spikes

Each spike time-boxes an unknown that would otherwise be discovered mid-project.
A spike closes with a written conclusion and, if needed, an ADR change.

| Spike | Question | Status |
|-------|----------|--------|
| [sqlcipher](sqlcipher.md) | Can we build and ship bundled SQLCipher on all targets? | Closed: builds with Strawberry Perl; encryption verified |
| [pdfa3](pdfa3.md) | Can Typst output be turned into conformant PDF/A-3 with an embedded CII file? | Open, scheduled before M4 |
| [validation](validation.md) | Can we execute the official EN 16931 Schematron/XSLT artefacts natively? | Closed: artefacts are XSLT 2.0; the authority path runs an external validator (ADR-006 amended) |
| [pdf-rendering](pdf-rendering.md) | Is Typst the right PDF backend, or is a PDF writer plus our own layout smaller? | Closed: `krilla` plus our layout shipped (2026-10-30); binary 62 MB → 32 MB |

Spikes are written down rather than remembered, because "we tried that" without
the error message is how the same afternoon gets spent twice.
