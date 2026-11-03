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

The corpora start from one real document per format, kept in
`fuzz/seeds/<target>/`, so mutation begins near the shape each parser
understands. `just fuzz` copies the seeds into `fuzz/corpus/<target>/`, which is
ignored: a long run writes thousands of generated inputs there, and only one that
improves coverage is worth keeping, by moving it into `seeds/<target>/`.

This is a separate Cargo workspace: the shipped binary must never depend on a
fuzz harness, and the targets need `-Z sanitizer` flags the stable build does
not carry.
