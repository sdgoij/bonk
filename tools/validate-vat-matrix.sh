#!/usr/bin/env sh
# M2 VAT matrix validation gate.
#
# Generates one invoice for each VAT category the current domain can represent
# and validates every UBL file with Mustangproject. Intra-community supply needs
# delivery fields and is added when those land.
set -eu

cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

MUSTANG_VERSION=2.17.0
MUSTANG_JAR=${MUSTANG_JAR:-target/validator/Mustang-CLI-$MUSTANG_VERSION.jar}
JAVA_BIN=${JAVA:-java}
WORK=target/vat-matrix
DB=$WORK/bonk.db
KEY=$WORK/keystore.bin
OUT=$WORK/out
PASS=validation

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
    echo "error: Java 17+ is required to run Mustang" >&2
    exit 1
fi

if [ ! -f "$MUSTANG_JAR" ]; then
    echo "downloading Mustang $MUSTANG_VERSION..."
    mkdir -p "$(dirname -- "$MUSTANG_JAR")"
    curl -L --fail --max-time 300 \
        -o "$MUSTANG_JAR" \
        "https://repo1.maven.org/maven2/org/mustangproject/Mustang-CLI/$MUSTANG_VERSION/Mustang-CLI-$MUSTANG_VERSION.jar"
fi

field() {
    sed -n "s/^$1: //p" | tr -d '\r'
}

rm -rf "$WORK"
mkdir -p "$OUT"

"$BONK_BIN" init --db "$DB" --keystore "$KEY" --passphrase "$PASS" >/dev/null
seller=$(
    "$BONK_BIN" seller-add \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --name "Validation Seller BV" --city Amsterdam \
        --vat NL123456782B01 --kvk 12345678 |
        field seller
)
plain_customer=$(
    "$BONK_BIN" customer-add \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --name "Domestic Customer BV" --city Rotterdam |
        field customer
)
vat_customer=$(
    "$BONK_BIN" customer-add \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --name "Business Customer BV" --city Utrecht \
        --vat NL111111110B01 |
        field customer
)

make_invoice() {
    label=$1
    customer=$2
    line_spec=$3

    set -- draft-new \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --seller "$seller" --customer "$customer" \
        --date 2026-10-27 --due 2026-11-26 \
        --line "$line_spec"
    draft=$("$BONK_BIN" "$@" | field draft)
    if [ -z "$draft" ]; then
        echo "error: could not create $label draft" >&2
        exit 1
    fi
    "$BONK_BIN" issue "$draft" \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --out "$OUT" >/dev/null
}

make_invoice "standard" "$plain_customer" "Consulting|2|50.00|21"
make_invoice "zero-rated" "$plain_customer" "Zero rated|1|100.00|0|Z"
make_invoice "exempt" "$plain_customer" "Exempt supply|1|100.00|0|E|||Exempt supply"
make_invoice "reverse-charge" "$vat_customer" "Reverse charge|1|100.00|0|AE||VATEX-EU-AE|Reverse charge"
make_invoice "export" "$plain_customer" "Export outside the EU|1|100.00|0|G||VATEX-EU-G|Export outside the EU"

# Outside-scope invoices must not carry seller or buyer VAT identifiers, so this
# scenario needs its own database.
O_WORK=$WORK/outside-scope
O_DB=$O_WORK/bonk.db
O_KEY=$O_WORK/keystore.bin
O_OUT=$O_WORK/out
mkdir -p "$O_WORK" "$O_OUT"
"$BONK_BIN" init --db "$O_DB" --keystore "$O_KEY" --passphrase "$PASS" >/dev/null
o_seller=$(
    "$BONK_BIN" seller-add \
        --db "$O_DB" --keystore "$O_KEY" --passphrase "$PASS" \
        --name "Outside Scope Seller BV" --city Amsterdam --kvk 87654321 |
        field seller
)
o_customer=$(
    "$BONK_BIN" customer-add \
        --db "$O_DB" --keystore "$O_KEY" --passphrase "$PASS" \
        --name "Outside Scope Customer BV" --city Rotterdam |
        field customer
)
o_draft=$(
    "$BONK_BIN" draft-new \
        --db "$O_DB" --keystore "$O_KEY" --passphrase "$PASS" \
        --seller "$o_seller" --customer "$o_customer" \
        --date 2026-10-27 --due 2026-11-26 \
        --line "Outside scope|1|100.00|0|O||VATEX-EU-O|Not subject to VAT" |
        field draft
)
"$BONK_BIN" issue "$o_draft" \
    --db "$O_DB" --keystore "$O_KEY" --passphrase "$PASS" \
    --out "$O_OUT" >/dev/null

invoices=$(find "$OUT" "$O_OUT" -maxdepth 1 -name '*.xml' | wc -l | tr -d ' ')
if [ "$invoices" -ne 6 ]; then
    echo "error: expected 6 invoices, found $invoices" >&2
    exit 1
fi

valid=0
for xml in "$OUT"/*.xml "$O_OUT"/*.xml; do
    log=${xml%.xml}.mustang.log
    if "$JAVA_BIN" -jar "$MUSTANG_JAR" \
        --action validate --source "$xml" \
        --no-notices --disable-file-logging >"$log" 2>&1 &&
        grep -q 'status="valid"' "$log"; then
        valid=$((valid + 1))
    else
        echo "INVALID: $xml" >&2
        tail -n 30 "$log" >&2
        exit 1
    fi
done

echo "Mustang EN 16931 / Peppol BIS 3.0 VAT matrix: $valid/$invoices invoices valid"
