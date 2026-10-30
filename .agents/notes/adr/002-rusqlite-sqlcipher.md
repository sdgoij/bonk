# ADR-002: `rusqlite` with bundled SQLCipher, one writer

**Status:** Accepted, 2026-10-27

## Context

The database holds invoices, bank statements, and eventually a ledger. It must
be encrypted at rest, queryable with indexes and full-text search, and it runs
on a laptop with one human user.

## Decision

Use SQLite through `rusqlite` (synchronous), compiled with
`bundled-sqlcipher-vendored-openssl`. Open with a raw 256-bit key, `WAL`,
`synchronous = FULL`, `foreign_keys = ON`, `secure_delete = ON`,
`temp_store = MEMORY`, one writer connection and at most a couple of readers.

## Consequences

- SQL semantics, transactions, FTS5, and `VACUUM INTO` backups are available
  without a server.
- Building requires a working C toolchain and OpenSSL; on Windows that means a
  full Perl (see the SQLCipher spike note), and on Linux a Perl that provides
  `Time::Piece` (a separate package on distributions that split core modules).
  The `plain` feature exists only for development on hosts that lack it.
- One writer caps concurrency, which matches the scale target.

## Alternatives

- **`sqlx` (async)**: no benefit for a desktop single-writer workload, and
  SQLCipher support is painful with offline builds.
- **`redb`/`sled`/LMDB**: no SQL, no FTS, and we would hand-roll reports.
- **Application-level field encryption over plain SQLite**: leaks structure and
  query patterns, and is easy to get wrong.
