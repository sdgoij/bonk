#!/usr/bin/env sh
# M4.6 external PDF/A gate.
#
# Issues a document at every Factur-X level and validates each hybrid with
# veraPDF, the reference PDF/A validator. veraPDF is a Java application, so it is
# downloaded into the ignored target/ directory and this script is not part of the
# offline test suite.
set -eu

cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

VERAPDF_DIR=${VERAPDF_DIR:-target/validator/verapdf}
JAVA_BIN=${JAVA:-java}
WORK=target/pdfa-validation
DB=$WORK/bonk.db
KEY=$WORK/keystore.bin
OUT=$WORK/out
PASS=pdfa-validation-gate

# The version this gate was last checked against. The download URL carries no
# version, so a fresh machine gets whatever upstream ships that day; this pin is
# what turns "it still passes" into "the validator we know passes". Bump it
# deliberately, after reading the report from the newer one.
VERAPDF_VERSION=${VERAPDF_VERSION:-1.30.4}

# Validating one file: this script again with a flag. `sh` cannot export a
# function, and the worker needs the environment the gate just set up, so
# re-entering is the portable way to give `xargs -P` something to run.
if [ "${1:-}" = "--check-one" ]; then
    pdf=$2
    report=$WORK/$(basename "$(dirname "$pdf")")-$(basename "$pdf").verapdf.log
    if "$JAVA_BIN" -jar "$VERAPDF_CLI" --format text "$pdf" >"$report" 2>&1 &&
        grep -q '^PASS .* 3b$' "$report"; then
        echo "PASS $(basename "$(dirname "$pdf")")/$(basename "$pdf") PDF/A-3B"
        exit 0
    fi
    echo "FAIL: $pdf" >&2
    cat "$report" >&2
    exit 1
fi

if [ -n "${BONK_BIN:-}" ]; then
    :
elif [ -x target/debug/bonk.exe ]; then
    BONK_BIN=target/debug/bonk.exe
elif [ -x target/debug/bonk ]; then
    BONK_BIN=target/debug/bonk
else
    echo "error: build the CLI first (cargo build -p cli)" >&2
    exit 1
fi

if ! command -v "$JAVA_BIN" >/dev/null 2>&1; then
    echo "error: Java 17+ is required to run veraPDF" >&2
    exit 1
fi

verapdf_cli() {
    ls "$VERAPDF_DIR"/bin/cli-*.jar 2>/dev/null | head -n 1
}

# The installer ships an IzPack jar that unpacks itself; `auto-install.xml` is the
# unattended answer file it reads, with the path filled in because the installer
# resolves it against the working directory java runs in.
install_verapdf() {
    echo "downloading veraPDF..."
    mkdir -p target/validator
    zip=target/validator/verapdf-installer.zip
    curl -L --fail --max-time 900 -o "$zip" "https://software.verapdf.org/releases/verapdf-installer.zip"

    rm -rf target/validator/verapdf-unpacked
    unzip -q -o "$zip" -d target/validator/verapdf-unpacked
    installer=$(find target/validator/verapdf-unpacked -name 'verapdf-izpack-installer-*.jar' | head -n 1)
    if [ -z "$installer" ]; then
        echo "error: the veraPDF download does not contain an installer" >&2
        exit 1
    fi

    root=$(pwd -W 2>/dev/null || pwd)
    answers=target/validator/verapdf-auto-install.xml
    cat >"$answers" <<EOF
<AutomatedInstallation langpack="eng">
    <com.izforge.izpack.panels.htmlhello.HTMLHelloPanel id="welcome"/>
    <com.izforge.izpack.panels.target.TargetPanel id="install_dir">
        <installpath>$root/$VERAPDF_DIR</installpath>
    </com.izforge.izpack.panels.target.TargetPanel>
    <com.izforge.izpack.panels.packs.PacksPanel id="sdk_pack_select">
        <pack index="0" name="veraPDF GUI" selected="false"/>
        <pack index="1" name="veraPDF CLI" selected="true"/>
        <pack index="2" name="veraPDF Documentation" selected="false"/>
        <pack index="3" name="veraPDF Sample Plugins" selected="false"/>
    </com.izforge.izpack.panels.packs.PacksPanel>
    <com.izforge.izpack.panels.install.InstallPanel id="install"/>
    <com.izforge.izpack.panels.finish.FinishPanel id="finish"/>
</AutomatedInstallation>
EOF

    rm -rf "$VERAPDF_DIR"
    "$JAVA_BIN" -jar "$installer" "$answers" >target/validator/verapdf-install.log 2>&1 || {
        echo "error: veraPDF could not be installed; see target/validator/verapdf-install.log" >&2
        exit 1
    }
}

if [ -z "$(verapdf_cli)" ]; then
    install_verapdf
fi
CLI=$(verapdf_cli)
if [ -z "$CLI" ]; then
    echo "error: veraPDF installed without a CLI jar" >&2
    exit 1
fi
VERAPDF_CLI=$CLI

version=$("$JAVA_BIN" -jar "$CLI" --version 2>/dev/null | head -n 1 | tr -d '\r')
echo "$version"
case "$version" in
    *"$VERAPDF_VERSION"*) ;;
    *)
        echo "error: expected veraPDF $VERAPDF_VERSION, got '$version'" >&2
        echo "       The download URL has no version in it, so upstream has moved." >&2
        echo "       Read the report from the new one, then bump VERAPDF_VERSION here" >&2
        echo "       and the cache key in .github/workflows/ci.yml." >&2
        exit 1
        ;;
esac

field() {
    sed -n "s/^$1: //p" | tr -d '\r'
}

rm -rf "$WORK"
mkdir -p "$OUT" "$WORK/templates"
# The repo ships one alternate layout; the archive finds it beside the database, so
# the gate covers every level against two templates rather than one.
cp templates/compact.json "$WORK/templates/compact.json"

"$BONK_BIN" init --db "$DB" --keystore "$KEY" --passphrase "$PASS" >/dev/null
seller=$(
    "$BONK_BIN" seller-add \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --name "Validation Seller BV" --city Amsterdam \
        --street "Main 1" --postal "1000 AA" --peppol 0106:12345678 \
        --vat NL123456782B01 --kvk 12345678 |
        field seller
)
customer=$(
    "$BONK_BIN" customer-add \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --name "Alpha Customer BV" --city Rotterdam \
        --street "Alpha 1" --postal "3000 AA" --peppol 0106:10000001 |
        field customer
)

# One invoice per level per template, so a level that stops conforming fails on
# its own name rather than somewhere inside a corpus.
issue_level() {
    level=$1
    template=$2
    set -- draft-new \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --seller "$seller" --customer "$customer" \
        --date 2026-10-27 --due 2026-11-26 --buyer-ref "PO-$level-$template" \
        --line "Consulting|2|50.00|21" \
        --note "Level $level, template ${template:-default}"
    [ -z "$template" ] || set -- "$@" --template "$template"
    draft=$("$BONK_BIN" "$@" | field draft)
    if [ -z "$draft" ]; then
        echo "error: could not create the $level draft" >&2
        exit 1
    fi
    out=$OUT/$level-${template:-default}
    mkdir -p "$out"
    "$BONK_BIN" issue "$draft" \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --profile "$level" --out "$out" >"$out/issue.log"
    # A layout that was not found falls back to the built-in one, and the gate would
    # then be checking one template twice. The CLI names the layout it really drew.
    if [ -n "$template" ] && ! grep -q "^layout: Compact " "$out/issue.log"; then
        echo "error: $level/$template was not drawn with the compact template" >&2
        cat "$out/issue.log" >&2
        exit 1
    fi
}

for level in minimum basic-wl basic en16931 extended; do
    issue_level "$level" ""
    issue_level "$level" compact
done

# A credit note carries BG-3 and a different document type, and a three-line
# invoice makes the VAT breakdown do some work.
credit=$(
    "$BONK_BIN" credit-note 2026-0001 \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --date 2026-12-15 |
        field "credit note"
)
if [ -z "$credit" ]; then
    echo "error: could not create a credit note" >&2
    exit 1
fi
"$BONK_BIN" issue "$credit" \
    --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
    --out "$OUT/other" >/dev/null

mixed=$(
    "$BONK_BIN" draft-new \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --seller "$seller" --customer "$customer" \
        --date 2026-12-01 --due 2026-12-31 --buyer-ref PO-MIXED \
        --line "Consulting|8|100.00|21|S|HUR" \
        --line "Handbook print|5|25.00|9|S" |
        field draft
)
"$BONK_BIN" issue "$mixed" \
    --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
    --out "$OUT/other" >/dev/null

total=0
pdfs=$WORK/pdfs.txt
: >"$pdfs"
for pdf in "$OUT"/*/*.factur-x.pdf; do
    printf '%s\n' "$pdf" >>"$pdfs"
    total=$((total + 1))
done

if [ "$total" -ne 12 ]; then
    echo "error: expected 12 hybrids, found $total" >&2
    exit 1
fi

export JAVA_BIN VERAPDF_CLI WORK
# Four at a time: one JVM per file, and these runners give four cores. The
# validations do not touch each other, so the wall time becomes one file's rather
# than twelve, and a failure still prints the report that explains it.
started=$(date +%s)
if ! xargs -P 4 -n 1 sh "$0" --check-one <"$pdfs"; then
    echo "error: at least one hybrid is not PDF/A-3B; the reports are in $WORK" >&2
    exit 1
fi
elapsed=$(( $(date +%s) - started ))

echo "veraPDF: $total hybrids valid as PDF/A-3B in ${elapsed}s (5 levels × 2 templates, 1 credit note, 1 mixed-rate invoice)"
