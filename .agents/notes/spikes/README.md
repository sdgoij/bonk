# M0 spikes

Each spike time-boxes an unknown that would otherwise be discovered mid-project.
A spike closes with a written conclusion and, if needed, an ADR change.

| Spike | Question | Status |
|-------|----------|--------|
| [sqlcipher](sqlcipher.md) | Can we build and ship bundled SQLCipher on all targets? | Closed: builds with Strawberry Perl; encryption verified |
| [mobile](mobile.md) | Can SQLCipher cross-build for Android and keep the archive interoperable? | Closed: x86_64 canary passes on API 37; arm64 builds; host↔device archives interoperate |
| [mobile-ffi](mobile-ffi.md) | Can UniFFI + Compose Multiplatform + JNA survive R8 on Android? | Closed: debug and minified release run on API 37; release APK 2.3 MB; Kotlin and Swift bindings generate |
| [mobile-keystore](mobile-keystore.md) | Can Android Keystore hold the device secret behind auth and surface invalidation? | Closed: put/get/forget + invalidation pass on debug and R8 release; PIN path, physical fingerprint remains |
| [mobile-argon2](mobile-argon2.md) | Is the 64 MiB Argon2id KDF fast enough on a low-end device? | Closed: 428 ms cold / 143–299 ms warm on a 2-vCPU AVD; no OOM; physical low-end remains |
| [mobile-determinism](mobile-determinism.md) | Do UBL and PDF bytes match between the host and Android? | Closed: identical SHA-256 for both artefacts on host and x86_64 API 37; arm64 build not yet run |
| [pdfa3](pdfa3.md) | Can Typst output be turned into conformant PDF/A-3 with an embedded CII file? | Open, scheduled before M4 |
| [validation](validation.md) | Can we execute the official EN 16931 Schematron/XSLT artefacts natively? | Closed: artefacts are XSLT 2.0; the authority path runs an external validator (ADR-006 amended) |
| [pdf-rendering](pdf-rendering.md) | Is Typst the right PDF backend, or is a PDF writer plus our own layout smaller? | Closed: `krilla` plus our layout shipped (2026-10-30); binary 62 MB → 32 MB |

Spikes are written down rather than remembered, because "we tried that" without
the error message is how the same afternoon gets spent twice.
