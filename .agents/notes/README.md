# Documentation

Working name: **Bonk** (placeholder — naming is unresolved; nothing in the
architecture may depend on the product name).

| Document | Purpose |
|----------|---------|
| [technical-design.md](technical-design.md) | v1.0 architecture, data model, compliance boundaries, security, testing. The reference for *why* the system is shaped this way. |
| [accounting-design.md](accounting-design.md) | v1.1 accounting layer: general ledger, VAT return preparation, bank reconciliation. |
| [implementation-plan.md](implementation-plan.md) | Milestones, effort estimates, release trains, quality gates, first-two-weeks task list. |
| [adr/](adr/README.md) | Architecture decision records 001–017. |
| [spikes/](spikes/README.md) | Time-boxed investigations and their findings. |

The original brief lives at `../../spec.md`. The design document supersedes it
where the two disagree; §2 of the design document lists every correction and the
reasoning behind it.

Document status: drafts, 2026-10-27. Changes to a recorded decision require a
new or amended ADR (technical-design §17, accounting-design §14).
