# Mobile (M11.0 spikes and M11.1 foundation)

The project is split into two Gradle modules, which is what AGP 9 requires when
Kotlin Multiplatform and Compose are involved:

- `shared` — the KMP library: `commonMain` Compose UI, `androidMain` platform
  `expect`/`actual` implementations, and the generated UniFFI bindings.
- `androidApp` — the application shell: application id, manifest, native
  libraries, and R8. It has no Kotlin of its own; `MainActivity` lives in the
  shared library and the manifest names it.

This replaces the S2 single-module `composeApp`. The AGP 9 compatibility flags
(`android.builtInKotlin=false`, `android.newDsl=false`) are gone. The shared
module uses the `com.android.kotlin.multiplatform.library` plugin, because AGP 9
rejects the legacy `com.android.library` and `com.android.application` plugins
alongside the KMP plugin.

## Prerequisites

- JDK 17
- Android SDK with `platforms/android-37.0`
- NDK r30 (`ANDROID_NDK_HOME`)
- `cargo-ndk`
- `ANDROID_HOME` pointing at the SDK
- On Windows, Git Bash's Perl and Strawberry Perl's pure-Perl modules, which
  `tools/cargo-android.sh` stages automatically.

## Build

```sh
cd mobile
./gradlew :androidApp:assembleDebug
./gradlew :androidApp:assembleRelease
```

The release build is signed with the debug key in this spike so it can be
installed. The generated bindings and `jniLibs` are gitignored.

## Regenerate the bindings and native libraries

With `ANDROID_NDK_HOME` set and `cargo-ndk` installed:

```sh
cd mobile
./gradlew :androidApp:assembleDebug -Pnative
```

`-Pnative` wires the root tasks into the build:

- `cargoNdk` builds `libbonkc.so` for `arm64-v8a` and `x86_64` into
  `androidApp/src/main/jniLibs`. On Windows it runs through
  `tools/cargo-android.sh`, which gives OpenSSL the Unix-path Perl it needs.
- `uniffiBindings` generates the Kotlin bindings from the arm64 library into
  `shared/build/generated/uniffi`.
- `stripNdk` runs `llvm-strip` on both libraries; AGP's own strip cannot handle
  the Rust `.so`.

Without `-Pnative` the build uses the libraries already in
`androidApp/src/main/jniLibs` and the bindings already in
`shared/build/generated/uniffi`.

The equivalent manual steps, from the repository root, are:

```sh
cargo build -p ffi
cargo run -p ffi --features cli --bin uniffi-bindgen -- \
  generate target/debug/bonkc.dll --language kotlin --out-dir target/uniffi-kotlin

mkdir -p mobile/shared/build/generated/uniffi
cp -r target/uniffi-kotlin/uniffi mobile/shared/build/generated/uniffi/

cargo ndk --platform 26 -t x86_64 -t arm64-v8a \
  -o mobile/androidApp/src/main/jniLibs build --release -p ffi
```

On Linux/macOS the host cdylib is `target/debug/libbonkc.so` or
`target/debug/libbonkc.dylib`; use that path for `uniffi-bindgen`.

## Lock and create flow

On first launch the app offers **Create your archive**; the passphrase is typed
twice. Creating it opens the archive and shows seller, draft, and schema counts.
**Lock** drops the key and returns to the unlock screen. Relaunching checks for
the two files and offers the unlock screen rather than create again.

The Android actual maps UniFFI error variants to user text, because a fieldless
error variant generates an empty Kotlin `message`: a wrong passphrase becomes
"Wrong passphrase. Try again.", and an existing archive becomes "An archive
already exists on this device." The passphrase stays in Compose state only and
is cleared after use. The archive handle lives in `MobileArchiveHost` and is
closed when the activity is destroyed, so a configuration change locks it.

The content is capped at 560 dp and centered on wide screens, and fills the
phone width otherwise. It was checked on the AVD at 1080×2400 (phone) and
1600×2560 at 320 dpi (800×1280 dp).

The S3/S4/S5 probes moved into a collapsed **Diagnostics** section below the
unlocked card.

## S4 Argon2 benchmark

The **Bench Argon2 (64 MiB)** diagnostic times the exact keystore KDF parameters
(Argon2id, 64 MiB, t=3, p=1) twice and reports cold/warm milliseconds plus the
process peak RSS. The call runs on `Dispatchers.IO`, so the progress text stays
visible while it works. On the 2-vCPU AVD the result was 428 ms cold,
143–299 ms warm, with a 272 224 KiB peak.

## S5 artefact determinism

The **Hash UBL + PDF** diagnostic calls `artefact_hashes()` from `crates/ffi`, which
builds a fixed sample invoice through `calculate_totals`, `write_invoice`, and
the PDF renderer, then hashes both artefacts:

```
ubl_sha256=7d7f70eb279208887daae6a0099bf1c61359a709f7fc0e6fb565ea7e9c5f7020
pdf_sha256=b81c0e35d9c11e1fa4c421fba60af47748a6bf949576dba38d2a66038d63e512
ubl_len=4305
pdf_len=25846
```

The host prints the same lines:

```sh
cargo run -p ffi --example artefact_probe
```

Both must match the Android output exactly; see
`.agents/notes/spikes/mobile-determinism.md`. The values are also pinned in a
`crates/ffi` unit test, so changing the renderer or fonts updates the test in
the same commit. The rebuilt M11.1 library (with `application` and SQLCipher
linked in) produced the same four values on the emulator.

## S3 Keystore probe

The **Run S3 Keystore test** diagnostic drives put/get/invalidate/forget through the
Rust `DeviceSecretStore` callback. The prompt allows biometrics or the device
credential, so a headless emulator needs a PIN:

```sh
adb shell locksettings set-pin 1234
```

Tap the button, then enter the PIN on the system keypad. The credential screen
is secure, so `screencap` is black; `uiautomator dump` exposes the keypad bounds
if the test is automated. The expected result is:

```
put=ok get=ok invalidated=detected delete=true after=none
```
