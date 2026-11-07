# Spike: cross-target UBL and PDF determinism

**Date:** 2026-11-07
**Environment:** Windows 11 host (x86_64), Rust 1.98.0, UniFFI 0.32.2,
render/`krilla` 0.8.2 + `harfrust` 0.15, NDK r30; AVD `Medium_Phone`
(x86_64, API 37)
**Status:** Closed — host and Android produce byte-identical UBL and PDF.

## Question

Does the artefact pipeline produce the same bytes on the host and on Android for
the same fixed invoice, or does the mobile ABI change the font/shaper path, the
clock, or the decimal formatting?

This is S5 in `mobile-design.md` §13.

## Answer

Yes. The same sample invoice serialises to the same UBL XML and the same PDF on
both targets:

| Artefact | Host | Android (x86_64, API 37) | Equal |
|---|---|---|---|
| UBL SHA-256 | `7d7f70eb279208887daae6a0099bf1c61359a709f7fc0e6fb565ea7e9c5f7020` | same | yes |
| UBL bytes | 4 305 | 4 305 | yes |
| PDF SHA-256 | `b81c0e35d9c11e1fa4c421fba60af47748a6bf949576dba38d2a66038d63e512` | same | yes |
| PDF bytes | 25 846 | 25 846 | yes |

The PDF hash is the stronger result: UBL is string assembly over integer money,
whereas the PDF exercises `harfrust` shaping, the bundled DejaVu fonts, `krilla`
serialisation, and the layout engine. All of those agree across the two targets.

Re-verified on 2026-11-07 after M11.1: the rebuilt library, now with
`application` and SQLCipher linked in, returned the same four values on the
emulator. Adding the archive layer did not move the renderer or the writer.

## Measurement path

`crates/ffi/src/lib.rs` exports `artefact_hashes()`, which builds one fixed
invoice and hashes both artefacts:

- seller `Bonk Demo BV`, customer `Voorbeeld Klant BV`, both NL;
- invoice `2026-0001`, issue `2026-11-07`, due `2026-12-07`, EUR;
- two lines: 3 × 150.00 pieces and 12.5 × 95.00 hours, 21 % VAT;
- totals from `crux::calculate_totals`, not hand-written constants;
- UBL from `syntax::write_invoice`;
- PDF from `render::Renderer::new().render(...)` with `Language::English`.

The data construction mirrors `app::pdf_data` (ISO dates, `to_fixed_string(2)`
totals, trimmed quantities and prices). M11.1 moves that function into
`crates/application`; this probe then delegates to it.

Host:

```sh
cargo run -p ffi --example artefact_probe
```

Android: the **Hash UBL + PDF** button calls the same function through the
UniFFI Kotlin bindings and prints the four values.

`crates/ffi/src/lib.rs` also pins the values in a unit test, so an accidental
change to the writer, the layout, the fonts, or the shaper fails `just test`
rather than silently changing every archived artefact.

## Why the clock and the IDs do not leak in

- `LocalDate` values are fixed in the sample; the PDF creation date is derived
  from the issue date, not from the wall clock.
- `LineId::new()` is random (UUID v7), but a line id is not serialised into
  either artefact.
- Money and quantities are integers; workspace lints deny float arithmetic.
- Repeated calls in one process return the same hashes, which the test asserts.

## Caveats

- Only the x86_64 emulator was executed. The `arm64-v8a` library builds and is
  packaged, but this host has no arm64 AVD; an arm64 device must be added to the
  matrix before the beta.
- The host is Windows. Linux and macOS CI runs are still to be added; the test
  is target-independent Rust and should stay green there.
- The probe stops short of `application::issue` and `store::issue_draft`; it
  proves the two writers are portable, not that the archive transaction is. That
  is M11.4's golden-artefact test.
- The pinned digests are expected to change when the renderer, the bundled
  fonts, or the UBL writer change. Update them in the same commit and record why.

## Decision

S5 passes. Keep `artefact_hashes()` and the golden test. In M11.1, replace the
probe's local data construction with the extracted `application` service; in
M11.4, run the same comparison through a real archive issue on an arm64 device
and in CI. Treat any future mismatch as a shaper, font, or clock defect before
suspecting money arithmetic.

## Emulator instance used

| AVD | Serial | ABI | API | PID | State |
|---|---|---|---|---|---|
| `Medium_Phone` | `emulator-5556` | x86_64 | 37 | 11536 | stopped 2026-11-07 |
| `Medium_Phone` | `emulator-5556` | x86_64 | 37 | 56220 | stopped 2026-11-07 (M11.1 re-check) |

`adb -s emulator-5556 emu kill` was followed by an explicit check; the launcher
PID was gone and the helper processes left by `emu kill` exited on their own.
No emulator remained in `adb devices`.
