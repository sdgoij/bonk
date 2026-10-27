# ADR-001: Hexagonal core with a crate-enforced dependency rule

**Status:** Accepted, 2026-10-27

## Context

The product promises that creation, validation, export, and search work with no
network. A promise like that decays unless the compiler enforces it: one
`reqwest` call in a domain function and the guarantee is gone, usually without
anyone noticing until an offline test fails in an airport.

## Decision

The domain lives in a crate (`crux`) that has no I/O dependencies: no
`rusqlite`, no `reqwest`, no `std::fs`, no `async`. Application services compose
domain rules with ports. Adapters implement the ports and are the only crates
allowed to touch the outside world.

The dependency direction is a compile-time property, not a review convention.
`cargo tree` is the architecture diagram.

## Consequences

- Offline behaviour is testable by construction.
- Passing an invoice model to a connector requires an explicit DTO conversion,
  which is where a network authorisation check can live.
- More small crates and more explicit mapping code than a layered monolith.

## Alternatives

- **Layers by technical concern only** (controllers/services/repositories):
  nothing prevents a repository call from a domain entity.
- **Single crate with modules**: the rule becomes a code-review promise.
