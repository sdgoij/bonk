#!/usr/bin/env sh
# Fetches the EN 16931 codelist Schematron into the ignored target/ directory.
#
# The file is the extraction source for `cargo run -p xtask -- codelists`.
# Its SHA-256 is pinned in `xtask/src/codelists.rs`, so re-running the
# extraction after an upstream change reports a hash mismatch instead of
# silently rewriting the bundles.
set -eu

cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

SOURCE=target/validator/EN16931-UBL-codes.sch
URL=https://raw.githubusercontent.com/ConnectingEurope/eInvoicing-EN16931/master/ubl/schematron/codelist/EN16931-UBL-codes.sch

if [ -f "$SOURCE" ]; then
    echo "already present: $SOURCE"
else
    echo "downloading the codelist Schematron..."
    mkdir -p "$(dirname -- "$SOURCE")"
    curl -L --fail --max-time 120 -o "$SOURCE" "$URL"
fi

echo "next: cargo run -p xtask -- codelists"
