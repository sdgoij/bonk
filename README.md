# Bonk (working name)

Local-first e-invoicing client for the EU market, with the accounting layer
scheduled next. Nothing leaves the machine unless the user explicitly sends it.

**Status:** M1 complete. M2 is underway: the `conformance` crate has
profiles, a BT/BG registry, a semantic invoice view, and the core BR-CO and
VAT-category rules. The shared issue path refuses blocking conformance findings
before anything is stored, and the UBL writer emits exemption reasons for the
standard, zero-rated, exempt, reverse-charge, export, and outside-scope
categories. The domain core, encrypted keystore, migrations, CLI, GUI workspace,
deterministic UBL writer, and visual PDF via the Typst CLI all work end to end.
A 20-invoice corpus and a 6-category VAT matrix pass Mustangproject against
EN 16931 and Peppol BIS 3.0 with zero failures.

Design documents live in [`.agents/notes/`](.agents/notes/README.md):

- [`technical-design.md`](.agents/notes/technical-design.md) — v1.0 architecture
- [`accounting-design.md`](.agents/notes/accounting-design.md) — v1.1 ledger
- [`implementation-plan.md`](.agents/notes/implementation-plan.md) — milestones

## Build

Requires Rust 1.98 (pinned in `rust-toolchain.toml`).

The default build enables SQLCipher through
`rusqlite/bundled-sqlcipher-vendored-openssl`, which compiles OpenSSL from
source. OpenSSL's `Configure` script is Perl and needs `ExtUtils::MakeMaker`;
the Perl bundled with Git for Windows is stripped and cannot build it.

**Windows:** install [Strawberry Perl](https://strawberryperl.com/) and use the
wrapper, which puts it first on `PATH`:

```sh
sh tools/cargo-strawberry.sh test --workspace
```

Set `STRAWBERRY_PERL_BIN` if it is not installed at `C:\Strawberry`. A cold
build takes about six minutes (OpenSSL); later builds are incremental.

**macOS / Linux:** the system Perl is sufficient.

```sh
cargo test --workspace
```

`.zed/settings.json` disables all language servers for Perl in this workspace;
the repository contains no Perl files.

### Development fallback: unencrypted

Hosts without a usable OpenSSL/Perl toolchain can still run the whole stack:

```sh
cargo test --workspace --no-default-features --features plain
```

This build stores the database **unencrypted**. It exists only for development:
the GUI shows a permanent red warning, and the CLI prints
`encryption: none (test build)`. Never ship or use it for real data. See
[`.agents/notes/spikes/sqlcipher.md`](.agents/notes/spikes/sqlcipher.md).

## Run

```sh
# macOS / Linux
cargo run -p cli -- init --db ./bonk.db --keystore ./keystore.bin
cargo run -p cli -- verify --db ./bonk.db --keystore ./keystore.bin
cargo run -p cli -- gui

# Windows
sh tools/cargo-strawberry.sh run -p cli -- init --db ./bonk.db --keystore ./keystore.bin
sh tools/cargo-strawberry.sh run -p cli -- gui
```

`BONK_PASSPHRASE` is read when `--passphrase` is omitted; otherwise the CLI
prompts. Passing a passphrase on the command line is for tests only (shell
history).

### First invoice from the CLI

```sh
bonk init
bonk seller-add --name "Your BV" --city Amsterdam --vat NL123456782B01
bonk customer-add --name "Customer BV" --city Rotterdam
bonk draft-new --seller <seller-id> --customer <customer-id> \
    --line "Consulting|2|50.00|21"
bonk issue <draft-id> --out ./out
bonk verify
```

Line format: `name|quantity|unit_price|rate[|category[|unit[|exemption_code[|exemption_reason]]]]`. For example:

```sh
--line "Reverse charge|1|100.00|0|AE||VATEX-EU-AE|Reverse charge"
--line "Exempt supply|1|100.00|0|E|||Exempt supply"
```

`issue` writes `<number>.xml` and, when Typst is installed, `<number>.pdf`; both
are stored as artefacts. `verify` recomputes the audit chain and every artefact
hash.

PDF rendering shells out to `typst` 0.14 (override with `BONK_TYPST`). If Typst
is unavailable, issuing still succeeds and prints a warning; the UBL artefact is
unaffected.

### External validation (M1)

With Java 17+ installed, build the CLI and run the external gate:

```sh
sh tools/cargo-strawberry.sh build -p cli
sh tools/validate-m1.sh
```

The script downloads Mustangproject 2.17.0 into the ignored `target/validator/`
directory, generates 20 varied invoices in `target/m1-validation/`, and validates
every UBL file. Last result: **20/20 invoices valid** against EN 16931 and
Peppol BIS 3.0. No Docker required. The validator jar is not committed.

For VAT-category or exemption-reason changes, also run the category matrix:

```sh
sh tools/validate-vat-matrix.sh
```

It validates standard, zero-rated, exempt, reverse-charge, export, and
outside-scope invoices. Last result: **6/6 invoices valid**.

### Desktop

`bonk gui` (or no subcommand) opens the same workflow: unlock, create the seller,
add customers, edit lines with live totals and findings, save drafts, and issue.
Both front ends call the same renderers: UBL is byte-identical whichever one
issues the document, and the Issued screen can save the Typst PDF.

## Layout

| Crate | Role | Milestone |
|-------|------|-----------|
| `crux` | Money, quantities, VAT, dates, IDs, invoice state machine | M0 |
| `store` | Argon2id keystore, SQLCipher, migrations | M0 |
| `app` | egui application library | M0 shell |
| `cli` | The `bonk` binary: GUI + headless commands | M0 |
| `conformance` | EN 16931 rules, codelists, profiles | M2 |
| `conformance-official` | Vendored Schematron/XSLT, native binding | M2 |
| `syntax` | UBL 2.1 / CII D16B writers and readers | M1/M3 |
| `render` | Typst PDF (M1); PDF/A-3 and Factur-X (M4) | M1/M4 |
| `import` | Sniffing, parsing, dedupe | M3 |
| `bridge` | Connector ports and plugin host | M7 |
| `sync` | HLC and CRDT merge | M8 |
| `testkit` | Fixtures and builders | M0 |
| `xtask` | Developer automation | M0 |

## Non-goals

No hosted backend, no accounts, no payroll, no inventory, no fixed assets, no
Peppol Access Point, no tax-filing intermediary. The full boundary is in
technical design §1.3.
