# Updating the vendored EN 16931 code lists

The lists live in `crates/conformance/codelists/<bundle>/` and are generated
from the official codelist Schematron. See
[ADR-018](adr/018-generated-codelists.md) for why they are generated rather
than maintained by hand.

Extraction is the only supported way to change a bundle. Editing a `.txt` file
by hand leaves it disagreeing with the manifest; the integrity test fails on the
next `cargo test`.

## Normal update

1. Fetch the upstream source into the ignored `target/` directory:

   ```sh
   sh tools/fetch-codelists.sh
   ```

2. Try the extraction against the pinned source:

   ```sh
   cargo run -p xtask -- codelists --check
   ```

   A clean tree prints `codelists: 9 files match the pinned source`. If the
   download differs from the pinned SHA-256, extraction stops and prints both
   hashes; continue with the procedure below only after reviewing the difference.

3. On an accepted upstream change:

   - Pick a new bundle label (date of the upstream release, `YYYY-MM`) and set
     `BUNDLE` in `xtask/src/codelists.rs`.
   - Update `SOURCE_SHA256` and `RETRIEVED` in the same file.
   - Update `BUNDLE_VERSION` and the `include_str!` paths in
     `crates/conformance/src/codelist.rs`.
   - Run `cargo run -p xtask -- codelists` to write the new bundle directory.
   - Delete the previous bundle directory in the same change; keeping two
     bundles would mean two answers to "is this code valid".
   - Run `cargo run -p xtask -- codelists --check` again.

4. Review the diff as data: which codes were added, removed, or renamed. A
   removal can turn a previously issued invoice into a failing one, so check
   whether a conformance rule or a test asserted the removed code.

5. Run the gates:

   ```sh
   cargo fmt --all --check
   sh tools/cargo-strawberry.sh clippy --workspace --all-targets -- -D warnings
   sh tools/cargo-strawberry.sh test --workspace
   sh tools/validate-m1.sh
   sh tools/validate-vat-matrix.sh
   ```

## Rehearsal (simulated release)

Run this once per bundle cycle, and after changing the extraction code, to prove
the pipeline still catches a real difference.

1. Copy the pinned source and inject a synthetic code into one list, for example
   add `ZZQ` to the `contains(' ... ')` literal of `BR-CL-23` (units).
2. Run `cargo run -p xtask -- codelists --source <copy> --accept-new-source`.
3. Confirm `git --no-pager diff --stat crates/conformance/codelists` shows only
   the expected file and `manifest.json`, and that `git --no-pager diff` on that
   file contains exactly the injected code.
4. Restore the bundle:

   ```sh
   git checkout -- crates/conformance/codelists
   ```

5. Confirm `cargo run -p xtask -- codelists --check` reports a clean tree again.

The rehearsal is recorded in `implementation-plan.md` under M2.2 together with
its date, so the next reviewer knows the pipeline was exercised rather than
assumed.
