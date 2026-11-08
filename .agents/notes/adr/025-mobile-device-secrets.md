# ADR-025: Mobile device secrets stay in the platform store and unlock biometrically

**Status:** Accepted, 2026-11-08

## Context

The mobile client may keep the archive key so the passphrase is not typed on
every launch. On the desktop the same convenience is ADR-021: the key goes into
the operating system's credential store and is protected by the login. A phone
has no login in that sense. It has a lockscreen, a device credential, and
usually a biometric.

The first implementation generated an Android Keystore key with
`setUserAuthenticationRequired(true)` and
`setInvalidatedByBiometricEnrollment(true)`, and allowed
`AUTH_BIOMETRIC_STRONG or AUTH_DEVICE_CREDENTIAL` so a headless emulator with
only a PIN could be driven. An emulator run with a simulated fingerprint on
2026-11-08 showed two problems.

- Adding a fingerprint did not invalidate the key. A key that also allows the
  device credential is not invalidated by a biometric enrolment change, so the
  flag was inert exactly when it mattered.
- `device_ready` unwrapped the key to answer "is a device key stored?", which
  showed a biometric prompt on every cold start before the user chose that
  route.

The same port also has to serve iOS later, and the desktop `keyring` stack must
not be pulled into the mobile binary.

## Decision

Keep the key in the platform store behind a `DeviceSecretStore` callback
(Android Keystore today, Keychain later), and make the Android key
biometric-only: `AUTH_BIOMETRIC_STRONG` with no device-credential fallback. The
passphrase is the fallback, not the PIN.

The port answers three questions separately: `put`, `get`, and `has`. `has` asks
whether an entry exists without reading it, so a readiness check never prompts.
The Rust `DeviceStore` trait gives `has` a default that reads, which is correct
for stores that answer without authentication, and the Android implementation
overrides it.

When the key is invalidated, the app deletes the wrapped copy, hides the device
route, and writes a `device-revoked` flag beside the archive. The flag is
consumed after the next successful passphrase unlock, when the open archive can
append `device_forgotten` to the audit chain.

A fresh Keystore key removes wrapped copies it can no longer open, and the alias
is versioned (`bonk.device.biometric`) so a key made under an older policy is not
reused. `keyring` stays a desktop dependency. The FFI error field is `detail`
rather than `message` because the Kotlin generator emits a broken `message`
member, and the archive lock is named `lock` because `close` collides with the
generated `AutoCloseable`.

## Consequences

- Adding or removing a fingerprint or face invalidates the device key, and the
  next device unlock asks for the passphrase. The device PIN no longer opens the
  archive, which is the point: a PIN holder could otherwise enrol a new
  biometric and keep the route.
- "Remember this device" is offered only when the device has an enrolled strong
  biometric. On a device without one the option is hidden and the passphrase is
  the only route.
- The device key protects a powered-off, stolen phone. It does not protect an
  unlocked running app, which holds the DEK in memory; that residual is
  unchanged from the desktop (R1, R10).
- The revocation event arrives one unlock late because the audit chain needs the
  open archive. The window is stated in the threat model and the design notes.
- Upgrading from the device-credential key leaves an unused alias in the Keystore
  and drops any remembered device, so the user unlocks once with the passphrase
  and re-enables the feature.
- iOS must use `.biometryCurrentSet` to get the same invalidation property. A
  Keychain item that allows the passcode has the same defect as
  `AUTH_DEVICE_CREDENTIAL`.

## Alternatives

- **Allow the device credential on the key.** Simpler for devices without
  biometrics and for headless emulators, but the enrolment-change invalidation
  does not fire, so a new fingerprint silently keeps access. Rejected.
- **Keep the key in the app's files under a key derived from the device
  credential.** Reimplements what the Keystore already does and gives up
  hardware-backed key storage. Rejected.
- **Store the passphrase instead of the data key.** Puts a reusable secret in a
  place the user did not expect, and makes the keystore's Argon2id irrelevant.
  Rejected for the same reason as ADR-021.
- **Unwrap on readiness to detect an invalidated key.** It prompts on every
  launch and still cannot distinguish "invalidated" from "not authenticated".
  Rejected in favour of a non-reading `has` plus the deferred revocation.
- **A separate key per archive.** The account name already names the archive; a
  single key with one alias is enough, and a second key adds migration state for
  no gain.
