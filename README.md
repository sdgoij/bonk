# Bonk (working name)

Local-first e-invoicing client for the EU market, with the accounting layer
scheduled next. Nothing leaves the machine unless the user explicitly sends it.

**Status:** M1 complete. M2 is underway: the `conformance` crate has
profiles, a BT/BG registry, a semantic invoice view, and the core BR-CO and
VAT-category rules. The shared issue path refuses blocking conformance findings
before anything is stored, the GUI can focus the editable field behind a finding,
and non-blocking warnings can be suppressed only with a recorded reason.
Blocking findings cannot be suppressed. The UBL writer emits exemption reasons
and delivery data for the standard, zero-rated, exempt, reverse-charge,
intra-community, export, and outside-scope categories. The domain core, encrypted
keystore, migrations, CLI, GUI workspace, deterministic UBL writer, and visual PDF
via the Typst CLI all work end to end. A 20-invoice corpus and a 7-category VAT
matrix pass Mustangproject against EN 16931 and Peppol BIS 3.0 with zero
failures.

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

## Repository setup

Install the pre-commit gate once per clone:

```sh
git config core.hooksPath .githooks
```

The hook runs `cargo fmt --all --check` and
`clippy --workspace --all-targets -- -D warnings` through
`tools/cargo-strawberry.sh`, and refuses the commit when either fails. Set
`BONK_PRECOMMIT_TESTS=1` to include `cargo test --workspace`, and use
`git commit --no-verify` to skip the gate once. It checks the working tree, not
the index, so a partially staged change needs its own care.

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

### Schema rollback

Reversible migrations can be rolled back explicitly:

```sh
bonk migrate-down --to 2 --yes
```

Rollback preflights the whole chain first. If any migration above the target has
no down migration, nothing is applied. Migrations 0001 and 0002 are intentionally
irreversible: they create the issued-document, artefact, and audit-evidence
schema. Migration 0003 is reversible because it owns only the suppression table.

Run the older binary after a rollback. A newer binary applies all compiled
migrations when it opens the database, so starting it again would reapply the
forward change.

### First invoice from the CLI

```sh
bonk init
bonk seller-add --name "Your BV" --city Amsterdam --street "Main 1" --postal "1000 AA" \
    --peppol 0106:12345678 --vat NL123456782B01 --kvk 12345678
bonk customer-add --name "Customer BV" --city Rotterdam --street "Buyer 1" \
    --postal "3000 AA" --peppol 0106:87654321
bonk draft-new --seller <seller-id> --customer <customer-id> \
    --due 2026-11-26 --buyer-ref PO-1 \
    --line "Consulting|2|50.00|21"
bonk issue <draft-id> --out ./out
bonk credit-note 2026-0001
bonk issue <credit-note-draft-id> --out ./out
bonk import ./received/*.xml
bonk received
bonk match 2026-0001 "Customer BV"
bonk search customer --direction inbound
bonk export --format csv --out invoices.csv
bonk approve 2026-0001
bonk mark-paid 2026-0001
bonk verify
bonk backup --to ./backups --keep 10
bonk verify-backup ./backups/backup-20261028T120000Z.db
```

Peppol BIS 3.0 requires an endpoint identifier for both parties and a buyer or
order reference, and a Dutch seller also needs a street, a post code, and a KvK
number. Omitting them is a blocking finding, not a warning, because the recipient
cannot route the document without them.

`import` records invoices received from suppliers in the same table as the ones
you issue, distinguished by direction. Both UBL 2.1 and UN/CEFACT CII (the syntax
inside Factur-X) are read; the syntax only changes the element names, not the
record. A Factur-X PDF is read as a container: the embedded document is
extracted and cross-checked against its XMP metadata, and the PDF is kept as
received. The issuer is matched against your customers by VAT identifier, then
Peppol identifier, then legal name and city; an unmatched supplier is kept and
flagged rather than invented. Importing the same file (or the same supplier
number) twice writes nothing and reports a duplicate.

`received` is the payable register: number, supplier, dates, amount, and status.
When the import-time match picked the wrong party, `match <document> <customer>`
records the right one by identifier or display name. The override is its own row
with an author and a timestamp, so the register shows the party on file while the
document keeps the name the supplier sent.

`approve` and `mark-paid` move a document forward, and each step is appended as
evidence — the document row itself never changes, because what you decided and
when is part of the record.

`search` looks through numbers, party names, references, and notes, and can be
narrowed by direction, an issue-date window, or status. Words are combined (all
must appear) and each may be partial, so `cust 2026` finds what you expect while
typing. Dates and amounts are filters, not text.

`export` writes the same rows as CSV (for a spreadsheet or an accountant) or
JSON (for tooling), with the same filters and an optional query. Amounts are
decimal strings in both formats, and an empty export is a header or an empty
array rather than an error. Standard output is the default; `--out` writes a
file.

`backup` takes a verified encrypted copy (`VACUUM INTO`, so it inherits the key)
and prunes older copies beyond `--keep`; nothing is pruned until the new copy has
passed an integrity check and the audit chain. `verify-backup` checks a copy
without opening the live database. Restoring is a file copy onto the clean
machine plus the same keystore — stop the application first, replace the database
file, then run `bonk verify`.

`verify` prints the audit chain head — sequence number, hash, and time. Two
copies of the same archive must agree on that triple; `verify-backup` prints the
same line for a copy, so the comparison is a diff rather than a guess.

`passphrase-change` rewraps the data key with a new passphrase. The database is
not touched, the new keystore is only written after the sealed bytes have been
unsealed in memory and confirmed to hold the same key, and the change is
recorded in the audit chain. Backups made before the change still need the old
passphrase. The new passphrase is prompted for twice; `BONK_NEW_PASSPHRASE` or
`--new-passphrase` exist for scripts and tests.

`recovery-add` seals the same data key under a second, one-time secret and prints
it: 32 base32 symbols in eight groups, from an alphabet without `I`, `L`, `O`, or
`U`, so a transcribed key is a typo rather than a different key. The passphrase
is required first — otherwise anyone holding a copy of the keystore could add a
route of their own. The key is shown once and never stored; running the command
again replaces it and the old key stops working.

`passphrase-reset` is the other half: it opens the archive with the recovery key
instead of the passphrase, installs a new passphrase, and **rotates the recovery
key**, printing the new one. Rotation is deliberate — the key that was just typed
may now be in a shell history or a screenshot, and leaving it valid would quietly
keep a second route open. The used key stops working.

`evidence <document> --out <dir>` writes a bundle an auditor or counterparty can
check without the archive: the document's stored record, its artefacts (and, for
an imported document, the bytes as received), and the whole audit chain, with
every file listed by SHA-256 in `manifest.json`. `evidence-verify <dir>` needs no
keystore and no database: it hashes every file, rejects anything the manifest
never listed, refuses paths that leave the bundle, and recomputes the chain,
requiring it to end at the recorded head. A number that names both an issued and
a received document is refused rather than guessed at — pass the identifier.
The chain travels whole, so a bundle grows with the archive; the design note says
what that buys and what it does not prove.

`support-bundle --out <dir>` writes what a maintainer needs and nothing about the
business: counts, versions, sizes, integrity results, per-kind audit counts, and
the chain head. No names, amounts, numbers, identifiers, or paths — it is
assembled from typed facts rather than filtered from logs, so there is no free
text to redact. The command prints exactly what it writes, so nothing is sent
that has not been read.

`credit-note` copies an issued invoice into a credit-note draft: same customer,
currency, and lines, with BG-3 naming the invoice it corrects. Credit notes use
their own number series (`2026-C0001`) so they never consume an invoice number.

Line format: `name|quantity|unit_price|rate[|category[|unit[|exemption_code[|exemption_reason]]]]`. For example:

```sh
--line "Reverse charge|1|100.00|0|AE||VATEX-EU-AE|Reverse charge"
--line "Exempt supply|1|100.00|0|E|||Exempt supply"
```

Intra-community supply also needs `--delivery YYYY-MM-DD`; the deliver-to country
is taken from the customer address. `issue` writes `<number>.xml` and, when Typst
is installed, `<number>.pdf`; both are stored as artefacts. `verify` recomputes
the audit chain, every artefact hash, and every stored payload — imported files
included — and reports unreferenced blobs without failing on them.

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
directory, generates 20 varied invoices plus a credit note in
`target/m1-validation/`, and validates every document. Last result: **21/21
documents valid** against EN 16931 and Peppol BIS 3.0. No Docker required. The
validator jar is not committed.

For VAT-category or exemption-reason changes, also run the category matrix:

```sh
sh tools/validate-vat-matrix.sh
```

It validates standard, zero-rated, exempt, reverse-charge, intra-community,
export, and outside-scope invoices. Last result: **7/7 invoices valid**.

### Official validator (M2.3)

Any directory of UBL documents can be checked against the official artefacts:

```sh
MUSTANG_JAR=target/validator/Mustang-CLI-2.17.0.jar \
    cargo run -p xtask -- conformance target/m1-validation/out
```

It prints one line per document, lists the rule ids that failed with their BT
reference and message, and exits non-zero on any invalid document. The oracle
needs Java 17+; it is a developer and release tool and never part of the shipped
binary (`official-validation` feature).

### Code lists

Validation uses vendored EN 16931 code lists (currency, country, VAT category,
VATEX, units, EAS, document type) embedded at compile time. They are generated
from the official codelist Schematron rather than maintained by hand:

```sh
sh tools/fetch-codelists.sh
cargo run -p xtask -- codelists --check
```

The bundle, its manifest, and the update procedure are documented in
`.agents/notes/codelist-updates.md` (decision: ADR-018).

### Desktop

`bonk gui` (or no subcommand) opens the same workflow: unlock, create the seller,
add customers, edit lines with live totals and findings, save drafts, and issue.
Both front ends call the same renderers: UBL is byte-identical whichever one
issues the document, and the Issued screen can save the Typst PDF.

The workspace also has a find panel: type a number, party, reference, or note and
press Enter (Ctrl+F jumps to the field), optionally narrowed by side and status.
Leaving the query blank lists everything the filters allow. Results are read-only
for now — opening an archived document in the editor is not built yet.

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
