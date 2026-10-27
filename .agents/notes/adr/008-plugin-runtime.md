# ADR-008: Sidecar plugins for network; WASM for pure transforms

**Status:** Accepted, 2026-10-27

## Context

Integrations should be additive and must never be able to corrupt the archive or
exfiltrate data silently. Third-party code also cannot be trusted with the
process's memory, secrets, and sockets.

## Decision

- **First-party connectors** are Rust implementations of the connector traits
  and ship with the application.
- **Third-party connectors** run as out-of-process sidecars speaking JSON-RPC
  over stdio, with a signed manifest, pinned hash, declared capabilities, and an
  egress host allow-list enforced by the host.
- **Pure transforms** (mapping, templating, validation extensions) may run in
  Wasmtime, which gets no filesystem or network access.

WASM is not used for network connectors: WASI does not provide meaningful egress
control, and secrets passed into a sandbox are still exposed to the host process
that runs it.

## Consequences

- A crashing or malicious plugin cannot take the database down; it can only fail
  its own job.
- Packaging a sidecar is more work for plugin authors than linking a library.
- The outbox and receipt model applies uniformly to both mechanisms.

## Alternatives

- **All in-process**: smallest blast radius wins over convenience here.
- **All WASM**: tempting, but network and secret handling become host functions,
  which reintroduces the trust problem with extra indirection.
