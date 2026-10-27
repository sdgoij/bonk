# ADR-012: Hash-chained audit log with optional AdES anchor

**Status:** Accepted, 2026-10-27

## Context

The product's claim is provable custody. Artefact hashes prove a file is the one
recorded, but not that the record itself was not rewritten. An append-only chain
makes tampering detectable without a central authority.

## Decision

Every state change appends an audit event:

```
hash_n = SHA-256(hash_{n-1} || canonical(event_n))
```

`canonical` is stable JSON: sorted keys, RFC 3339 UTC, no floats, nulls omitted.
Event kinds cover creation, validation, issue, artefact rendering, sending,
receipts, payments, credits, key rotation, exports, and retention deletion.

The chain head is shown in the UI and included in evidence bundles. A verifier
recomputes the chain offline (`bonk verify`). Optional AdES signing of the head
is supported for users who want a stronger anchor.

## Consequences

- Silent edits to historical events become detectable.
- Local attackers who hold the key can still rewrite the whole chain; only
  external anchoring closes that gap, and the product says so rather than
  overclaiming.
- Every write path must emit an event, which is tested per feature.

## Alternatives

- **No audit log**: fails the evidence goal.
- **Blockchain/timestamping authority in v1**: cost and complexity without a
  customer asking for it.
