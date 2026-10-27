# Spike: bundled SQLCipher on Windows

**Date:** 2026-10-27
**Environment:** Windows, Rust 1.98.0, MSVC 14.44, `rusqlite` 0.41,
`bundled-sqlcipher-vendored-openssl`, OpenSSL 3.6.3, Strawberry Perl 5.42.3
**Status:** Closed — build works with Strawberry Perl; encryption verified.

## Question

`rusqlite`'s `bundled-sqlcipher-vendored-openssl` feature compiles SQLCipher and
OpenSSL from source. Does that work on a normal Windows development machine, and
what does it cost?

## Answer

Yes, with a full Perl. Cold build takes about 6 minutes; incremental builds are
seconds.

```sh
sh tools/cargo-strawberry.sh test --workspace
```

The wrapper only prepends `/c/Strawberry/perl/bin` to `PATH` and execs cargo.
`STRAWBERRY_PERL_BIN` overrides the location. CI on `windows-latest` needs
`ilammy/msvc-dev-cmd` for `nmake`.

## Failure mode that had to be solved first

Git for Windows' Perl is stripped and cannot build OpenSSL. Two distinct errors,
the second of which is fatal:

```
Can't locate Locale/Maketext/Simple.pm in @INC
  ... at /usr/share/perl5/core_perl/Params/Check.pm line 6.
```

A two-function shim (`loc`, `gettext`) gets `Configure` past this, and then:

```
Configuring OpenSSL version 3.6.3 for target VC-WIN64A
Can't locate ExtUtils/MakeMaker.pm in @INC
Failure! build file wasn't produced.
```

`MakeMaker` generates the build file and cannot reasonably be shimmed. Do not
spend time on shims: install Strawberry Perl.

A second trap for anyone scripting this: MSYS Perl splits `PERL5LIB` on `:`, so
a Windows path (`C:/…`) is truncated after the drive letter. Environment values
must reach Perl in POSIX form (`/c/…`). Cargo passes environment variables
through unchanged, while the MSYS shell rewrites them when spawning native
programs, so setting `PERL5LIB` from a shell script does not survive.

## Verified with the real cipher build

16 store tests, all green:

- keystore round-trip, wrong passphrase, header tampering, truncation,
  passphrase rotation without re-encrypting the database;
- migrations applied once, idempotent reopen, `PRAGMA integrity_check`;
- `open` never creates a missing database file;
- **`the_file_does_not_contain_plaintext`** — a canary string inserted into the
  database does not appear in `.db`, `-wal`, or `-shm`;
- **`a_wrong_key_cannot_open_the_file`** — SQLCipher logs an HMAC failure and the
  open is rejected;
- **`backup_inherits_the_key_and_the_data`** — `VACUUM INTO` produces an
  encrypted copy that reopens with the same key, is rejected by another key,
  and contains no plaintext; an existing backup is never overwritten.

Full workspace (96 tests) also passes with the encrypted default, including the
CLI integration test that asserts `encryption: sqlcipher`.

## Still open (not part of the build question)

- Crash-during-write torture (kill the process mid-commit, reopen, verify).
- Restore verification against the audit chain — lands with M6, when the chain
  exists.
- Release gate that fails if a shipped binary reports `encryption: none`.

## Decision

ADR-002 and ADR-011 stand unchanged. The `plain` cargo feature stays for hosts
that cannot build OpenSSL, but it is loudly labelled in the UI and CLI and is
excluded from releases.
