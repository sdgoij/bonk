# Fuzz targets

libFuzzer harnesses for the three parsers that consume untrusted input:

- `ubl` — the UBL 2.1 reader (`syntax::read::read_invoice`)
- `cii` — the CII D16B reader (`syntax::read::read_document`)
- `pdf` — the Factur-X PDF extractor (`import::extract_invoice`)

Each target answers or refuses, and must never panic or hang. The property tests
in `syntax` and `import` are the offline gate; these targets are the deeper,
time-boxed run, and they need a nightly toolchain plus `cargo-fuzz`.

## Run

```sh
cargo install cargo-fuzz          # once
cargo +nightly fuzz run ubl        # runs until interrupted
cargo +nightly fuzz run ubl -- -max_total_time=60
just fuzz ubl 60                  # the same, via the Justfile
just fuzz-all 60                  # all three, a minute each
```

The corpora are seeded with one real document per format (`fuzz/corpus/<target>/`),
so mutation starts near the shape each parser understands. New interesting inputs
land in the same directory and are worth committing when they improve coverage.

This is a separate Cargo workspace: the shipped binary must never depend on a
fuzz harness, and the targets need `-Z sanitizer` flags the stable build does
not carry.
