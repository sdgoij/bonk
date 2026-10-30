# Bonk developer tasks.
#
# One place for the commands in AGENTS.md. Every recipe that calls Cargo goes
# through tools/cargo-strawberry.sh on Windows, because the SQLCipher build
# compiles OpenSSL and OpenSSL's Configure script is Perl.
#
#   just            list what is here
#   just gate       everything CI runs, before committing
#   just build      the shippable release binary

set shell := ["sh", "-cu"]
set windows-shell := ["sh", "-cu"]

cargo := if os_family() == "windows" { "sh tools/cargo-strawberry.sh" } else { "cargo" }

# List the recipes.
default:
    @just --list

# Build the shippable binary: release profile, SQLCipher enabled.
build:
    {{cargo}} build --release -p cli

# Build the development binary without optimisation.
build-debug:
    {{cargo}} build -p cli

# Open the window against the demo archive (passphrase: bonk-demo).
run:
    {{cargo}} run -p cli -- gui

# Regenerate the demo archive: seller, customers, and a few hundred documents.
demo:
    {{cargo}} run -p cli -- demo-seed --force

# Check formatting and whitespace. This is the whole gate for docs-only changes.
fmt:
    {{cargo}} fmt --all --check
    git --no-pager diff --check

# Rewrite formatting in place.
fmt-fix:
    {{cargo}} fmt --all

# Lints, with warnings treated as errors.
clippy:
    {{cargo}} clippy --workspace --all-targets -- -D warnings

# Licences, advisories, and banned crates. Needs `cargo-deny` installed:
#   cargo install cargo-deny --locked
# The advisory database is fetched on the first run and cached after that.
deny:
    {{cargo}} deny check

# The offline test suite, encrypted build.
test:
    {{cargo}} test --workspace

# The same suite against the unencrypted development fallback.
test-plain:
    {{cargo}} test --workspace --no-default-features --features plain

# Test one crate: `just test-crate store`.
test-crate crate:
    {{cargo}} test -p {{crate}}

# Every gate from AGENTS.md, in the order CI runs them.
gate: fmt clippy deny test test-plain

# Enable the pre-commit hook once per clone.
hooks:
    git config core.hooksPath .githooks

# UBL serialisation, validation, and PDF rendering (needs Java 17+ and network).
validate-m1:
    sh tools/validate-m1.sh

# VAT-category, exemption-reason, and breakdown matrix (needs Java 17+).
validate-vat:
    sh tools/validate-vat-matrix.sh

# Developer automation: `just xtask codelists`, `just xtask conformance <dir>`.
xtask *args:
    {{cargo}} run -p xtask -- {{args}}

# Remove build artefacts.
clean:
    {{cargo}} clean
