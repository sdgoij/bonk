# Mobile S2 spike

This is the S2 proof, not the production layout. It uses a single `composeApp`
module and the AGP 9 compatibility flags `android.builtInKotlin=false` and
`android.newDsl=false`. M11.1 splits it into `shared` (KMP library) +
`androidApp` (application); see `.agents/notes/spikes/mobile-ffi.md` and
`.agents/notes/mobile-design.md` §6.1.

## Prerequisites

- JDK 17
- Android SDK with `platforms/android-37.0`
- NDK r30 (`ANDROID_NDK_HOME`)
- `cargo-ndk`
- `ANDROID_HOME` pointing at the SDK

## Generate the bindings and native libraries

The Gradle build can regenerate both. With `ANDROID_NDK_HOME` set and
`cargo-ndk` installed:

```sh
cd mobile
./gradlew :composeApp:assembleDebug -Pnative
```

`-Pnative` makes `preBuild` depend on `cargoNdk` and `uniffiBindings`. The
bindings are generated from the arm64 library that `cargoNdk` produced, so no
separate host build is needed. On Windows the tasks go through
`tools/cargo-strawberry.sh`, which is what OpenSSL's build needs.

Without `-Pnative` the build uses the libraries already in
`src/androidMain/jniLibs`.

The equivalent manual steps, from the repository root, are:

```sh
cargo build -p ffi
cargo run -p ffi --features cli --bin uniffi-bindgen -- \
  generate target/debug/bonkc.dll --language kotlin --out-dir target/uniffi-kotlin

mkdir -p mobile/composeApp/build/generated/uniffi
cp -r target/uniffi-kotlin/uniffi mobile/composeApp/build/generated/uniffi/

cargo ndk --platform 26 -t x86_64 -t arm64-v8a build --release -p ffi
mkdir -p mobile/composeApp/src/androidMain/jniLibs/x86_64
mkdir -p mobile/composeApp/src/androidMain/jniLibs/arm64-v8a
cp target/x86_64-linux-android/release/libbonkc.so \
  mobile/composeApp/src/androidMain/jniLibs/x86_64/
cp target/aarch64-linux-android/release/libbonkc.so \
  mobile/composeApp/src/androidMain/jniLibs/arm64-v8a/
```

On Linux/macOS the host cdylib is `target/debug/libbonkc.so` or
`target/debug/libbonkc.dylib`; use that path for `uniffi-bindgen`.

Strip the Android libraries before a size measurement (the exact `llvm-strip`
path depends on the NDK install):

```sh
llvm-strip --strip-unneeded mobile/composeApp/src/androidMain/jniLibs/*/libbonkc.so
```

## Build

```sh
cd mobile
./gradlew :composeApp:assembleDebug
./gradlew :composeApp:assembleRelease
```

The release build is signed with the debug key in this spike so it can be
installed. The generated bindings and `jniLibs` are gitignored.

## S4 Argon2 benchmark

The "Bench Argon2 (64 MiB)" button times the exact keystore KDF parameters
(Argon2id, 64 MiB, t=3, p=1) twice and reports cold/warm milliseconds plus the
process peak RSS. The call runs on `Dispatchers.IO`, so the progress text stays
visible while it works. On the 2-vCPU AVD the result was 428 ms cold,
143–299 ms warm, with a 272 224 KiB peak.

## S5 Artefact determinism

The "Hash UBL + PDF" button calls `artefact_hashes()` from `crates/ffi`, which
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
the same commit.

## S3 Keystore probe

The "Run S3 Keystore test" button drives put/get/invalidate/forget through the
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
