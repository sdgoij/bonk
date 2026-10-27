# ADR-004: Issued invoices are write-once; corrections are credit notes

**Status:** Accepted, 2026-10-27

## Context

A user will notice a typo after sending an invoice. The tempting fix is to make
the record editable. That destroys the only thing the product sells: proof of
what was sent, and when.

## Decision

`Draft → Validated → Issued` is one-way. At `Issued` the document is numbered,
party data is snapshotted, artefacts are hashed, and the rows are protected by
`BEFORE UPDATE`/`BEFORE DELETE` triggers. Corrections are credit notes (`381`)
and, where required, a replacement invoice.

The state machine in `crux::invoice` refuses illegal transitions; the triggers
are a second line of defence against future code.

## Consequences

- Every issued document can be reproduced and explained years later.
- Users must learn the credit-note flow; the UI has to make that obvious rather
  than hiding it.
- Storage grows monotonically (retention deletion is an explicit, audited
  action, not an edit).

## Alternatives

- **Editable issued invoices**: simple, and indefensible in an audit.
- **Soft delete / status flags**: makes "issued" ambiguous; a deleted invoice
  may still have been delivered.
