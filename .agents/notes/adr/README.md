# Architecture Decision Records

Short records of decisions that are expensive to reverse. The full reasoning
lives in `../technical-design.md` and `../accounting-design.md`; each file here
captures the decision, its context, and what it costs.

| ADR | Decision |
|-----|----------|
| [001](001-hexagonal-core.md) | Hexagonal core with a crate-enforced dependency rule |
| [002](002-rusqlite-sqlcipher.md) | `rusqlite` with bundled SQLCipher, one writer |
| [003](003-money-micro-units.md) | Money as integer micro-units; no floats |
| [004](004-immutable-issued-invoices.md) | Issued invoices are write-once; corrections are credit notes |
| [005](005-crdt-scope.md) | CRDTs for drafts and catalogues only; facts merge by union |
| [006](006-official-validation.md) | Official validation artefacts behind an isolated native binding |
| [007](007-peppol-provider.md) | Peppol through access-point provider APIs, not AS4 |
| [008](008-plugin-runtime.md) | Sidecar plugins for network; WASM for pure transforms |
| [009](009-pdf-stack.md) | Typst layout with PDF/A-3 post-processing |
| [010](010-egui.md) | egui accepted with a documented accessibility limitation |
| [011](011-key-wrapping.md) | DEK/KEK key wrapping with Argon2id; SQLCipher raw key |
| [012](012-audit-chain.md) | Hash-chained audit log with optional AdES anchor |
| [018](018-generated-codelists.md) | Code lists generated from the official Schematron, not hand-maintained |
| [019](019-pdf-extraction.md) | Factur-X extraction with `lopdf`; the visual layer is never read |

Adding or changing a decision means adding an ADR; editing history is not how
this directory works.
