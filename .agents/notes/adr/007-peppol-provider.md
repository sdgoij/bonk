# ADR-007: Peppol through access-point provider APIs, not AS4

**Status:** Accepted, 2026-10-27

## Context

Peppol looks like a protocol (AS4 over HTTPS with WS-Security, signing,
encryption, MDN receipts, SMP/SML discovery). Joining it is a certification and
operations programme: AP PKI certificates, conformance testing, always-on SMP
availability, and commercial obligations.

## Decision

Define a `PeppolAccessPoint` port with `discover`, `send`, `status`, and
`poll_inbound`. Ship adapters for commercial access points that expose a REST
API. Never implement AS4 ourselves.

Every send is outbox-backed and idempotent, and every provider receipt is stored
as an immutable artefact.

## Consequences

- We can ship invoicing before Peppol (v1.0), and Peppol before we are a network
  operator.
- Provider choice is commercial, not architectural; multiple providers can
  coexist.
- Users depend on a provider account for Peppol sending, which the product must
  state plainly.

## Alternatives

- **Own access point**: a separate company-sized project with certification and
  uptime obligations.
- **Email-only delivery**: does not satisfy counterparties that mandate Peppol.
