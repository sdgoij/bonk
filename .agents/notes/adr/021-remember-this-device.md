# ADR-021: "Remember this device" keeps the data key in the OS credential store

**Status:** Accepted, 2026-10-28

## Context

A passphrase on every launch is the right default for an archive that holds a
company's invoices, and it is also the reason people leave the application closed.
The plan left a way out: O3, an optional OS-keychain wrap, for "remember me".

Two questions had to be answered: what is stored, and by whom.

## Decision

`store::device` keeps **the data key itself** in the platform credential store —
Windows Credential Manager, the macOS Keychain, the freedesktop Secret Service —
through `keyring` 4 (the v1-compatible API, MIT OR Apache-2.0, one dependency on
each platform).

Alternatives that were rejected:

- **Storing the passphrase** would put a secret the user may reuse elsewhere into a
  place they did not expect it, and would make the keystore's Argon2id irrelevant
  on this machine.
- **A third wrap inside the keystore**, with the wrapping key in the keychain, was
  the tidier-looking design: the keystore stays the one artefact. It requires a
  third file format version with flags for which wraps are present, a migration,
  and a rule for what happens when the keychain entry and the file disagree. It
  buys nothing over storing the key: whoever can read the entry can also read the
  keystore beside it, so the indirection protects against a threat that does not
  exist here.

The entry is named `bonk` / `archive-<hash of the absolute keystore path>`, so the
credential store's UI does not list the user's folders. The cost is that moving
the archive orphans the entry; `forget` then reports that it had nothing to
remove, and the stale entry holds a key to a database that is no longer at that
path.

Enabling it requires the passphrase again, and both enabling and disabling are
appended to the audit chain (`device_trusted`, `device_forgotten`): a second way
into the archive is evidence about the archive, like a recovery key.

The store is a port (`DeviceStore`) with an in-memory implementation for tests,
because no CI runner has a credential store. Linux without a Secret Service — a
container, a headless box — reports "no credential store" rather than failing
obscurely, and the window hides the button in that case only after it is pressed.

## Consequences

- While it is on, the archive is protected by the operating system's login, not by
  the passphrase. Anyone at an unlocked session can read it, which is exactly what
  the feature promises and what the threat model now says in R9.
- The passphrase still protects a copy taken to another machine; the entry does not
  travel.
- Changing the passphrase does not touch the entry, because the data key does not
  change. Changing the *archive path* does, in the sense that the entry is no
  longer found.
- `keyring` brings `windows-native-keyring-store` on Windows, `apple-native-keyring-store`
  on macOS, and `zbus-secret-service-keyring-store` (and therefore `zbus`) on Linux.
  None of it is on the hot path; all of it is opt-in at runtime.
- `bonk forget --device` works without the passphrase: revoking the convenience
  must not require the credential it replaced.

## Alternatives

- **Windows Hello / Touch ID as the condition**: the platform stores above are
  what those APIs are built on; asking for a biometric prompt per unlock is a
  separate feature with its own platform code on three platforms.
- **Trusting the archive for N minutes after unlock**: keeps the passphrase in
  play but still requires typing it once per session; it solves a different
  problem (annoyance within a day) and the idle lock already covers it.
