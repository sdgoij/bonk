# Spike: Argon2id timing on Android

**Date:** 2026-11-07
**Environment:** Windows 11, JDK 17.0.11, Gradle 9.9.0, AGP 9.5.1,
Kotlin 2.4.21, Compose Multiplatform 1.12.2, UniFFI 0.32.2, JNA 5.19.1,
NDK r30, AVD `Medium_Phone` (x86_64, API 37, 2 vCPU, `-memory 2048`)
**Status:** Closed — 64 MiB / t=3 / p=1 completes in well under 3 s, with no OOM.

## Question

Does the keystore's production Argon2id cost (64 MiB, 3 iterations, one lane)
finish in under ~3 s on a 2–3 GB device without an OOM, and is a progress UI
needed?

This is S4 in `mobile-design.md` §13.

## Answer

Yes. The app's **Bench Argon2 (64 MiB)** button reports, across two runs:

| Run | First (cold) | Second (warm) | Peak RSS |
|---|---|---|---|
| 1 | 428 ms | 299 ms | 272 224 KiB |
| 2 | 151 ms | 143 ms | 272 224 KiB |

The budget is 3 000 ms, so even the cold run is more than 6× inside it. The
64 MiB KDF does not produce an OOM; the peak RSS includes the Compose/JNA
baseline, with roughly 64 MiB of it being Argon2.

## Measurement path

`bench_argon2` in `crates/ffi` mirrors `store::keystore::derive_kek` exactly:

- Argon2id, version `0x13`;
- `m_cost = 65 536 KiB` (64 MiB), `t_cost = 3`, `p_cost = 1`;
- 32-byte output, 16-byte salt;
- two runs: the first includes cold page faults, the second is warm;
- `VmHWM` is read from `/proc/self/status` after the runs.

The Android actual formats the report as:

```
first=<ms> ms, second=<ms> ms, maxRSS=<KiB> KiB
```

The call runs on `Dispatchers.IO`; the UI remains paintable and shows
`running 64 MiB Argon2id...` while it works, so the progress affordance is
already in place.

## Results

The emulator was started with `-cores 2` and `-memory 2048`. The guest reported
2 CPUs and `MemTotal` ~3.9 GiB; the memory flag did not reduce the guest to
2 GiB. The measurement still demonstrates that the 64 MiB allocation succeeds
and is far from the 3 s ceiling, but it is not a substitute for a physical
2–3 GB device.

| Check | Result |
|---|---|
| Cold KDF | 428 ms |
| Warm KDF | 143–299 ms |
| Peak RSS | 272 224 KiB |
| OOM / crash | None |
| Progress UI | Feasible; call is off the UI thread |

## Caveats

- The AVD is an x86_64 host CPU, not a low-end ARM phone. Emulator timings are
  optimistic; a physical 2–3 GB device test remains in the device matrix.
- `-memory 2048` did not reduce the guest's `MemTotal` below ~3.9 GiB on this
  image. The 64 MiB cost is small relative to either size, but the 2 GB
  allocation-pressure test still needs a real device or a differently
  configured AVD.
- The first run includes cold page faults; the second is the steady-state
  figure. Both are reported, because unlock after a cold start is the case the
  user feels.
- The 272 MiB peak is process RSS, not KDF allocation. It includes Compose, JNA,
  and the app baseline.

## Decision

Keep `KdfParams::DEFAULT` at 64 MiB / t=3 / p=1. Do not lower it for mobile.
Show progress while the KDF runs, because a slower physical device may spend
1–3 s there even though this emulator does not. Add a physical low-end ARM
timing to the device matrix before the beta.

## Emulator instance used

| AVD | Serial | ABI | API | Cores | PID | State |
|---|---|---|---|---|---|---|
| `Medium_Phone` | `emulator-5554` | x86_64 | 37 | 2 | 18828 | stopped 2026-11-07 |

The app was uninstalled and the screenshots were removed. `adb emu kill` left
emulator process `28564` behind; it was terminated explicitly.
