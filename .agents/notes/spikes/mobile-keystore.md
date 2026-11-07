# Spike: Android Keystore device secret store

**Date:** 2026-11-07
**Environment:** Windows 11, JDK 17.0.11, Gradle 9.9.0, AGP 9.5.1,
Kotlin 2.4.21, Compose Multiplatform 1.12.2, UniFFI 0.32.2, JNA 5.19.1,
`androidx.biometric` 1.1.0, `androidx.fragment` 1.9.1, NDK r30, AVD
`Medium_Phone` (x86_64, API 37, 16 KB pages)
**Status:** Closed — Keystore wrap/unwrap, Rust-driven callback, invalidation
detection, and passphrase-fallback signal verified on debug and R8 release.

## Question

Can the Android Keystore hold a 32-byte device secret behind authentication,
be driven from Rust through the UniFFI `DeviceSecretStore` callback, and report
invalidation so the caller can fall back to the archive passphrase?

This is S3 in `mobile-design.md` §13.

## Answer

Yes. The app's S3 button reports:

```
put=ok get=ok invalidated=detected delete=true after=none
```

The same result appears on the debug build and on the minified/R8 release. The
emulator used the device-credential path (a PIN) rather than a fingerprint; the
Keystore code path is the same, and a physical biometric test remains for the
device matrix.

## Implementation

Rust (`crates/ffi`):

- `DeviceSecretStore` is a UniFFI callback interface:
  `put(account, secret)`, `get(account) -> Option<Vec<u8>>`,
  `delete(account) -> bool`;
- `AppError { Store { detail }, Invalidated, AuthFailed { detail } }`;
- exported `write_device_secret`, `read_device_secret`,
  `forget_device_secret`, and `probe_device_store`;
- `probe_device_store` performs put → get → compare → delete → get and reports
  the result. The unit test uses an in-memory implementation.

Android (`mobile/composeApp`):

- `KeystoreDeviceStore` implements the generated `DeviceSecretStore` interface.
- Key: AES-256-GCM, alias `bonk.device.s3`,
  `setUserAuthenticationRequired(true)`,
  `setInvalidatedByBiometricEnrollment(true)`,
  `setUserAuthenticationParameters(0, AUTH_BIOMETRIC_STRONG |
  AUTH_DEVICE_CREDENTIAL)` on API 30+, `setUnlockedDeviceRequired(true)` on API
  28+, and `setUserAuthenticationValidityDurationSeconds(-1)` for API 26–29.
- Storage: `noBackupFilesDir/device/<account>.bin`, containing `iv ||
  ciphertext`; the 32-byte secret is never written in plaintext.
- Authentication: `BiometricPrompt` with a `CryptoObject`; the prompt allows
  biometrics or the device credential. The Rust callback runs on an IO thread
  and blocks on a `CountDownLatch` while the prompt is shown on the main thread.
- Invalidation: `invalidateForTest()` deletes the Keystore alias and leaves the
  wrapped file. The next `get` finds no key and throws
  `AppException.Invalidated`; the harness catches it and reports
  `invalidated=detected`.
- `KeystoreHost.runS3Test()` calls the Rust functions in sequence:
  write → read → compare → invalidate → read (expect `Invalidated`) → forget →
  read (expect `None`).

## Test flow

```sh
adb shell locksettings set-pin 1234
adb install -r mobile/composeApp/build/outputs/apk/debug/composeApp-debug.apk
adb shell am start -n com.github.sdgoij.bonk/.MainActivity
# tap "Run S3 Keystore test", enter 1234 on the system keypad
```

The device-credential screen is secure, so `screencap` returns black; the PIN
keypad was inspected with `uiautomator dump` and tapped at the digit bounds.
`input text` does not reach the custom Compose keypad. The debug build accepted
one PIN entry; the release build showed a second prompt before finishing, which
is consistent with per-use authentication.

The release build was also installed and tested:

```sh
./gradlew :composeApp:assembleRelease   # 3.0 MB, R8 + resource shrinking
adb install -r composeApp/build/outputs/apk/release/composeApp-release.apk
```

## Measurements

| Build | Result |
|---|---|
| Debug APK | Installed; S3 result shown |
| Release APK, R8 | **3.0 MB**; S3 result shown |
| Rust unit tests | 5 passed (hello, bytes, counter, probe, short-secret rejection) |
| Keystore round-trip | `put=ok get=ok` |
| Invalidation | `invalidated=detected` |
| Forget | `delete=true`, then `after=none` |
| Release delta from S2 | ~0.7 MB for biometric/fragment/coroutines |

## Findings

1. **The Keystore and callback work.** The platform secret can be wrapped,
   read back, and deleted entirely from Rust through the UniFFI callback.
2. **Device credential was the testable path.** A headless emulator has no
   enrolled fingerprint; allowing `DEVICE_CREDENTIAL` made the auth path
   runnable. Production must decide whether that is an acceptable second route
   or whether to require `BIOMETRIC_STRONG` and fall back to the passphrase.
   ADR-025 owns that decision.
3. **Invalidation is surfaced correctly.** Deleting the key while leaving the
   ciphertext makes the next read return `Invalidated`, which is the signal the
   archive UI needs to show the passphrase form. A real biometric-enrolment
   change was not performed; that remains a physical-device test.
4. **UniFFI 0.32.2 has a Kotlin error-codegen collision.** A variant field named
   `message` generates both a constructor property and a `Throwable.message`
   override, which does not compile. Use `detail` (or any other name) for error
   payloads. The plan's `AppError` sketch now uses `detail`.
5. **The device-credential window is a security trade-off.** While the key is
   authorized, a PIN can unwrap the secret. The passphrase remains the fallback
   and the archive file is still SQLCipher-encrypted; document the route and
   decide the policy before release.
6. **The release measurement is still comfortable.** 3.0 MB with biometric and
   fragment dependencies is far below the 30 MB budget.

## Decision

S3 passes for the device store. Keep `KeystoreDeviceStore`, the callback, and
the invalidation signal. M11.1 adapts the store to the archive's
`DeviceStore`/`Application` layer, decides the biometric-vs-device-credential
policy in ADR-025, and adds the physical fingerprint and enrolment-change tests.
The iOS Keychain path stays optional and follows the same callback contract.

## Emulator instance used

| AVD | Serial | ABI | API | Page size | PID | State |
|---|---|---|---|---|---|---|
| `Medium_Phone` | `emulator-5554` | x86_64 | 37 | 16384 | 34092 | stopped 2026-11-07 |

The test PIN was cleared, the app was uninstalled, and the screenshot/keypad
files were removed. `adb emu kill` left emulator process `10488` behind; it was
terminated explicitly.
