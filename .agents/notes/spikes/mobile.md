# Spike: SQLCipher on Android

**Date:** 2026-11-07
**Environment:** Windows 11, Rust 1.98.0, NDK r30, `cargo-ndk` 4.1.2,
`rusqlite` 0.41, `bundled-sqlcipher-vendored-openssl`, SQLCipher 4.19.0,
OpenSSL 3.6.3, Cygwin Perl 5.42.2 with MakeMaker 7.76 from Strawberry Perl
5.42.3
**Status:** Closed — x86_64 canary passes on an API 37 emulator; arm64 builds;
archives round-trip between host and device.

## Question

Can the desktop SQLCipher stack cross-build for `aarch64-linux-android` and
`x86_64-linux-android`, run on an emulator, keep the archive encrypted, and
produce files that the desktop client can open?

This is S1 in `mobile-design.md` §13. It is the portability risk that could
change the mobile plan.

## Answer

Yes. The existing `sqlcipher` feature cross-builds without changing the
production crates:

- `x86_64-linux-android` builds and runs on the `Medium_Phone` AVD (API 37,
  x86_64, 16 KB page size).
- `aarch64-linux-android` builds; no arm64 AVD is installed, so it was not
  executed.
- The canary is absent from `.db`, `-wal`, and `-shm` on the device.
- A host-created archive opens on the device, and a device-created archive opens
  on the host.
- Both binaries have `LOAD` alignment `0x4000`; NDK r30's default is already
  16 KB-safe.

No production code changed. The spike adds one example,
`crates/store/examples/android_canary.rs`.

## What was run

Host (the existing desktop stack):

```sh
sh tools/cargo-strawberry.sh build -p store --example android_canary
./target/debug/examples/android_canary.exe create target/s1-host
./target/debug/examples/android_canary.exe open   target/s1-host
```

Android x86_64:

```sh
export MSYS_NO_PATHCONV=1
export ANDROID_HOME=C:/Users/T/AppData/Local/Android/Sdk
export ANDROID_NDK_HOME=C:/Users/T/AppData/Local/Android/android-ndk-r30
export PERL5LIB=/c/Users/T/Desktop/bonk/target/perl-lib
export PATH=/usr/bin:$PATH
cargo ndk --platform 26 -t x86_64 build -p store --example android_canary

adb push target/x86_64-linux-android/debug/examples/android_canary /data/local/tmp/
adb shell chmod 755 /data/local/tmp/android_canary
adb shell /data/local/tmp/android_canary create /data/local/tmp/s1-android
```

Android arm64:

```sh
cargo ndk --platform 26 -t arm64-v8a build -p store --example android_canary
```

Cross-platform archive check:

```sh
# host -> device
adb push target/s1-host /data/local/tmp/s1-host
adb shell /data/local/tmp/android_canary open /data/local/tmp/s1-host

# device -> host
adb pull /data/local/tmp/s1-android target/s1-android-pulled
./target/debug/examples/android_canary.exe open target/s1-android-pulled
```

## Results

| Check | Result |
|---|---|
| Host canary create/open | Pass; 516 096-byte database, canary absent from raw bytes |
| x86_64 cross-build | Pass; 3 min 53 s cold, ~1 s incremental |
| x86_64 emulator run | Pass; API 37, x86_64, `getconf PAGESIZE` = 16384 |
| arm64 cross-build | Pass; ELF 64-bit ARM aarch64, Android 26 |
| Debug binary size | 42 MB per ABI, unstripped (release size still to be measured) |
| 16 KB alignment | Pass; every `LOAD` segment is `0x4000` aligned on both ABIs |
| Host archive opens on Android | Pass; seller and currency round-trip |
| Android archive opens on host | Pass; seller and currency round-trip |
| Canary absent on device | Pass; scanned `.db`, `-wal`, `-shm` |

The emulator image is the 16 KB `google_apis_playstore_ps16k` image, so the
x86_64 run also proves the SQLCipher build works on a 16 KB-page device.

## Emulator instance used

The only instance started for this spike:

| AVD | Serial | ABI | API | Page size | PID | State |
|---|---|---|---|---|---|---|
| `Medium_Phone` | `emulator-5554` | x86_64 | 37 | 16384 | 52828 | stopped 2026-11-07 |

The AVD was not created or wiped. Only `emulator-5554` was killed; no other
emulator process was touched. Future runs should check `adb devices` first, use
an explicit port, and record the same details here before stopping the instance.

## The Windows Perl trap

`openssl-src` runs OpenSSL's `Configure`, which is Perl. On Windows:

- **Strawberry Perl** has MakeMaker but is `MSWin32`; OpenSSL refuses it:
  "This perl implementation doesn't produce Unix like paths".
- **Git for Windows' Cygwin Perl** produces Unix paths but ships without
  `ExtUtils::MakeMaker`, `Locale::Maketext::Simple`, and `Pod::Usage`.

The spike staged the missing pure-Perl modules from Strawberry into
`target/perl-lib` and pointed Cygwin Perl at them:

```sh
mkdir -p target/perl-lib
cp -r /c/Strawberry/perl/lib/ExtUtils target/perl-lib/
cp -r /c/Strawberry/perl/lib/Locale  target/perl-lib/
cp -r /c/Strawberry/perl/lib/Pod     target/perl-lib/

export PERL5LIB=/c/Users/T/Desktop/bonk/target/perl-lib
export PATH=/usr/bin:$PATH
```

Two MSYS details matter:

- `PERL5LIB` must reach Perl as `/c/...`. Without `MSYS_NO_PATHCONV=1`, MSYS
  rewrites it to `C:/...`, and Perl splits it on `:` into `C` and
  `/Users/...`.
- `MSYS_NO_PATHCONV=1` also stops `adb push`/`adb shell` from rewriting
  `/data/local/tmp/...` into `C:/Program Files/Git/...`. Set the Android paths
  in Windows form (`C:/...`) when it is on.

This is a local-host workaround, not a product dependency. On Linux the Perl is
already Unix-like and has MakeMaker, so a CI cross-build should not need any of
it. The mobile pipeline should run the cross-build on Ubuntu, not Windows.

## Findings that affect the plan

1. **Vendored OpenSSL is viable.** No switch to a prebuilt SQLCipher, and no
   application-level encryption, is needed. The existing `sqlcipher` feature is
   enough.
2. **The libtomcrypt provider is not reachable through `libsqlite3-sys`
   0.39.1.** `sqlcipher-src` 419.0.6 ships libtomcrypt, but `libsqlite3-sys`
   does not compile those sources; without vendored OpenSSL it still links
   `crypto`. Do not plan on the no-OpenSSL path without upgrading or patching
   that crate.
3. **16 KB pages are already handled by NDK r30.** The proposal's suggested
   `-Wl,-z,max-page-size=16384` is not needed by default. Keep the CI `readelf`
   check and add the flag only if a toolchain change regresses it.
4. **`keyring` still enters the Android graph** because `store`'s default
   features include it. It did not break this example, but M11.1 should gate it
   behind `keychain` so mobile never compiles the desktop credential store.
5. **Windows is a workable but awkward cross-build host.** Use Linux CI for the
   authoritative Android build; keep Windows for local convenience.
6. **Debug size is not release size.** 42 MB is unstripped debug. Measure the
   release `.so` in S2/the size-budget job before trusting the APK budget.

## Decision

S1 passes. The mobile plan keeps `rusqlite` + SQLCipher and the existing
encrypted archive format. The next spike is S2 (UniFFI + Compose Multiplatform);
M11.1 should also add the `keychain` feature gate and the Linux CI cross-build.
