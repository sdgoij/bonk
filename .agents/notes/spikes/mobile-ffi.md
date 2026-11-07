# Spike: UniFFI + Compose Multiplatform on Android

**Date:** 2026-11-07
**Environment:** Windows 11, JDK 17.0.11, Gradle 9.9.0, AGP 9.5.1,
Kotlin 2.4.21, Compose Multiplatform 1.12.2, UniFFI 0.32.2, JNA 5.19.1,
Android SDK 37 (`android-37.0`), NDK r30, AVD `Medium_Phone` (x86_64, API 37,
16 KB pages)
**Status:** Closed — debug and R8/minified release both run on the emulator;
arm64 builds; Kotlin and Swift bindings generate from the same Rust surface.

## Question

Can a Rust crate expose a UniFFI object and function to a Compose Multiplatform
Android app, survive JNA and R8, and stay inside a reasonable APK budget? Does
the maintained UniFFI Kotlin Multiplatform generator fit the shared module, or
do we need the JVM-Kotlin + Swift fallback?

This is S2 in `mobile-design.md` §13.

## Answer

Yes. The minimal `crates/ffi` crate generates Kotlin and Swift bindings from one
Rust definition. The Android app calls a plain function, a stateful object, and
a byte buffer; debug and minified release both display:

```
Hello from Rust, Android! counter=3 bytes=3
```

The maintained `uniffi-bindgen-kotlin-multiplatform` crate is stale (0.1.0,
2023) and is not compatible with UniFFI 0.32.2. S2 therefore used standard
Kotlin/JVM bindings with JNA on Android. Swift bindings generate cleanly for the
future iOS target. The KMP `expect/actual` boundary keeps the FFI call out of
`commonMain`.

## Versions pinned by the spike

| Component | Version |
|---|---|
| Gradle | 9.9.0 |
| AGP | 9.5.1 |
| Kotlin | 2.4.21 |
| Compose Multiplatform | 1.12.2 |
| UniFFI | 0.32.2 |
| JNA | 5.19.1 |
| activity-compose | 1.13.0 |
| compileSdk / targetSdk | 37 / 35 |
| minSdk | 26 |

Compose 1.12 requires `compileSdk >= 37`; the installed platform is
`android-37.0` and AGP accepts it. `targetSdk` stays at 35 for the spike.

## What was built

Rust (`crates/ffi`):

- `hello(name) -> String`;
- `Counter` with a constructor and `increment() -> u64`;
- `uniffi::setup_scaffolding!()` and a `uniffi-bindgen` binary behind a `cli`
  feature;
- `[lib] name = "bonkc"`, `crate-type = ["lib", "cdylib", "staticlib"]`.

Kotlin/Compose (`mobile/`):

- `commonMain`: `expect fun greet(name: String): String` and the Compose UI;
- `androidMain`: the `actual` calls `hello` and `Counter` from
  `uniffi.bonkc`, plus `MainActivity` and the manifest;
- generated Kotlin is added to `androidMain` from
  `mobile/composeApp/build/generated/uniffi` (not committed);
- `libbonkc.so` is copied into `src/androidMain/jniLibs/<abi>/` (not committed).

Commands used:

```sh
# Rust tests
cargo test -p ffi

# Kotlin bindings from the host cdylib
cargo build -p ffi
cargo run -p ffi --features cli --bin uniffi-bindgen -- \
  generate target/debug/bonkc.dll --language kotlin --out-dir target/uniffi-kotlin

# Swift bindings (iOS path)
cargo run -p ffi --features cli --bin uniffi-bindgen -- \
  generate target/debug/bonkc.dll --language swift --out-dir target/uniffi-swift

# Android libraries
cargo ndk --platform 26 -t x86_64 -t arm64-v8a build --release -p ffi
# copy and strip with NDK llvm-strip --strip-unneeded

# App
cd mobile
./gradlew :composeApp:assembleDebug
./gradlew :composeApp:assembleRelease
adb install -r composeApp/build/outputs/apk/release/composeApp-release.apk
```

## Measurements

| Build | Result |
|---|---|
| Rust unit tests | 3 passed |
| Kotlin bindings | `uniffi.bonkc.bonkc.kt` generated |
| Swift bindings | `bonkc.swift`, `bonkcFFI.h`, `bonkcFFI.modulemap` generated |
| Debug APK (x86_64 + arm64, all JNA ABIs initially) | 19 MB, then 18 MB after `abiFilters` |
| Release APK, R8 + resource shrinking | **2.3 MB** |
| Release APK without JNA/Rust (stub `actual`, temporary build) | **1.1 MB** |
| JNA + UniFFI + Rust delta | ~1.2 MB total |
| JNA native libs (x86_64 + arm64) | 303 432 bytes |
| `libbonkc.so` release, both ABIs | ~531 KB + ~564 KB unstripped; 848 336 bytes after `llvm-strip --strip-unneeded` |
| Debug `libbonkc.so` | 6.4 MB |
| Release runtime | Minified APK installed, launched, rendered the Rust greeting |

The release budget is 30 MB; at 2.3 MB, JNA is not worth replacing with a
hand-written JNI shim. The no-JNA figure is a real build (stub `actual`, no
UniFFI, no JNA, no native library), not an estimate.

## Emulator instance used

| AVD | Serial | ABI | API | Page size | PID | State |
|---|---|---|---|---|---|---|
| `Medium_Phone` | `emulator-5554` | x86_64 | 37 | 16384 | 54672 | stopped 2026-11-07 |

Only this instance was started; no other emulator was touched. The app was
uninstalled and the screenshot files were removed before shutdown. `adb emu
kill` detached the device but left the emulator process (`47704`) behind; it was
terminated explicitly.

## Findings that affect the plan

1. **Use standard Kotlin/JVM bindings, not the KMP generator.** The
   `uniffi_bindgen_kotlin_multiplatform` crate is 0.1.0 from 2023 and does not
   track UniFFI 0.32.2. Keep the KMP `expect/actual` boundary and put the
   generated Kotlin in `androidMain`; the Swift bindings serve `iosMain` later.
2. **AGP 9.5 rejects the Kotlin Multiplatform plugin in an application module.**
   S2 used the documented temporary bypass:
   `android.builtInKotlin=false` and `android.newDsl=false`. Production must
   split the project: a KMP/shared library module (new
   `com.android.kotlin.multiplatform.library` plugin) plus a thin
   `com.android.application` module. Do not carry the bypass into M11.1.
3. **JNA's size is acceptable.** The total release APK is 2.3 MB with both ABIs;
   the no-JNA comparison is 1.1 MB. A JNI shim is not justified by size alone.
4. **R8 needs keep rules.** `-keep class uniffi.**`, `-keep class
   com.sun.jna.**`, and the JNA `Structure` subclass rule are sufficient; the
   minified build calls Rust successfully.
5. **Strip the Rust library for release.** A debug `libbonkc.so` is 6.4 MB;
   the release build is ~531–564 KB and ~367–391 KB after `llvm-strip`.
   AGP's `stripReleaseDebugSymbols` could not strip `libbonkc.so` in this setup,
   so packaging needs an explicit strip step or `strip = "symbols"` in the Rust
   release profile.
6. **Compose Multiplatform works on Android**, and the same `commonMain` UI can
   be shared with iOS. The CMP dependency accessors are deprecated in 1.12
   ("specify dependency directly"), but the direct `material3` coordinate did
   not resolve at the same version; keep the accessors for now and revisit when
   the plugin's artifact versions are aligned.
7. **The binding must live in `androidMain`.** The generated Kotlin uses JNA and
   `java.nio`; it cannot go into `commonMain`. This is exactly what the
   `expect/actual` wrapper is for.
8. **The manifest starts from a safe baseline:** `allowBackup=false`, no
   `INTERNET` permission, no exported components beyond the launcher activity.
9. **Do not export a method named `close`.** UniFFI 0.32.2 gives every object a
   Kotlin `AutoCloseable.close()`; a Rust `close()` generates a duplicate
   declaration and the bindings do not compile. The archive method is `lock()`.
   This is the same collision class as the error field that had to be renamed
   from `message` to `detail`.
10. **M11.1 confirmed the two-module split.** AGP 9.5 rejects both
    `com.android.application` and `com.android.library` alongside the KMP
    plugin; `com.android.kotlin.multiplatform.library` is required. The
    application module has no Kotlin, and `MainActivity` stays in `shared` with
    the manifest in `androidApp` naming it. The archive probe creates, locks,
    and reopens a SQLCipher archive on the emulator.

## Decision

S2 passes. Keep UniFFI 0.32.2, generated Kotlin/JVM bindings, JNA, and Compose
Multiplatform. M11.1 adopts the two-module structure required by AGP 9 (shared
KMP library + Android application), adds the Kotlin/Swift generation and native
library copy to Gradle/CI, and keeps the `expect/actual` boundary so iOS can
supply a Swift-backed actual.
