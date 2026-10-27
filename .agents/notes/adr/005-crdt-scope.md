# ADR-005: CRDTs for drafts and catalogues only; facts merge by union

**Status:** Accepted, 2026-10-27

## Context

The brief asked for a schema designed around CRDTs so multi-device sync could
arrive later. CRDTs are the wrong tool for legal documents: merge is defined as
"pick a winner", and for an issued invoice there is no legitimate winner between
two different versions.

## Decision

Split the data by nature:

- **Immutable facts** — issued invoices, artefacts, journal entries, audit
  events: replicated as an append-only set, merged by union. Same id with a
  different content hash is an integrity alarm, never a merge.
- **Mutable state** — drafts, customers, catalogue, settings: per-field
  last-writer-wins registers ordered by a hybrid logical clock, plus
  observed-remove sets for collections.
- **Sequences** — leased blocks per device, so numbering never collides.

## Consequences

- Sync is simpler and auditable: conflicts only exist where a human can resolve
  them.
- Two devices can still create two different drafts; that is visible and
  mergeable.
- The v1.1 ledger inherits the rule: postings are facts, period locks are
  facts.

## Alternatives

- **Whole-document CRDT**: silent corruption of issued documents.
- **No sync primitives until v2**: rewrites the schema and every repository.
