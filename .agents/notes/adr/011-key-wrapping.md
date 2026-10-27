# ADR-011: DEK/KEK key wrapping with Argon2id; SQLCipher raw key

**Status:** Accepted, 2026-10-27

## Context

The database must be encrypted at rest, and users must be able to change their
passphrase without re-encrypting gigabytes. SQLCipher's own passphrase KDF is a
black box with defaults we do not control, and a passphrase change means a full
`PRAGMA rekey` over the file.

## Decision

- Generate a random 32-byte data encryption key (DEK).
- Derive a key encryption key (KEK) from the passphrase with Argon2id
  (64 MiB, 3 iterations, 1 lane, 16-byte salt).
- Wrap the DEK with AES-256-GCM, authenticating the header (magic, version, KDF
  parameters, salt) as additional data.
- Store the wrapped DEK in `keystore.bin`; open SQLCipher with the raw DEK
  (`PRAGMA key = "x'…'"`), skipping SQLCipher's KDF.
- A passphrase change rewraps 32 bytes. The file is not touched.

## Consequences

- KDF parameters are explicit, versioned, and testable.
- Changing a passphrase is instant; a forgotten passphrase is still
  unrecoverable, which the UI must say out loud.
- The keystore file is a single point of failure and must be backed up with the
  database.
- Memory hygiene: keys live in `Zeroizing` buffers and are never formatted.

## Alternatives

- **SQLCipher passphrase directly**: opaque KDF, slow passphrase changes.
- **OS keychain only**: portable but not sufficient at rest; used, if at all, as
  an additional wrap.
