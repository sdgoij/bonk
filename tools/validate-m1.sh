#!/usr/bin/env sh
# M1 external validation gate.
#
# Generates 20 invoices through the CLI and validates every UBL file with
# Mustangproject. The validator jar is downloaded into the ignored target/
# directory, so this script is not part of the offline test suite.
set -eu

cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

MUSTANG_VERSION=2.17.0
MUSTANG_JAR=${MUSTANG_JAR:-target/validator/Mustang-CLI-$MUSTANG_VERSION.jar}
JAVA_BIN=${JAVA:-java}
WORK=target/m1-validation
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
        --street "Main 1" --postal "1000 AA" --peppol 0106:12345678 \
        --vat NL123456782B01 --kvk 12345678 |
        field seller
)

c1=$("$BONK_BIN" customer-add --db "$DB" --keystore "$KEY" --passphrase "$PASS" --name "Alpha Customer BV" --city Rotterdam --street "Alpha 1" --postal "3000 AA" --peppol 0106:10000001 | field customer)
c2=$("$BONK_BIN" customer-add --db "$DB" --keystore "$KEY" --passphrase "$PASS" --name "Beta Customer BV" --city Utrecht --street "Beta 2" --postal "3500 AA" --peppol 0106:10000002 | field customer)
c3=$("$BONK_BIN" customer-add --db "$DB" --keystore "$KEY" --passphrase "$PASS" --name "Gamma Customer BV" --city Eindhoven --street "Gamma 3" --postal "5600 AA" --peppol 0106:10000003 | field customer)
c4=$("$BONK_BIN" customer-add --db "$DB" --keystore "$KEY" --passphrase "$PASS" --name "Delta Customer BV" --city Groningen --street "Delta 4" --postal "9700 AA" --peppol 0106:10000004 | field customer)

n=0
while IFS='|' read -r date due item qty price rate note buyer_ref; do
    [ -n "$date" ] || continue
    n=$((n + 1))

    case $((n % 4)) in
        1) customer=$c1 ;;
        2) customer=$c2 ;;
        3) customer=$c3 ;;
        0) customer=$c4 ;;
    esac

    set -- draft-new \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --seller "$seller" --customer "$customer" \
        --date "$date" --due "$due" \
        --line "$item|$qty|$price|$rate"
    [ -z "$note" ] || set -- "$@" --note "$note"
    [ -z "$buyer_ref" ] || set -- "$@" --buyer-ref "$buyer_ref"

    draft=$("$BONK_BIN" "$@" | field draft)
    if [ -z "$draft" ]; then
        echo "error: could not create draft $n" >&2
        exit 1
    fi
    "$BONK_BIN" issue "$draft" \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --out "$OUT" >/dev/null
done <<'SPECS'
2026-01-05|2026-02-04|Consulting|1|100.00|21|January retainer|PO-1001
2026-01-19|2026-02-18|Support hours|3|75.50|21||PO-1002
2026-02-02|2026-03-04|Implementation|1|1250.00|21|Phase one|PO-1003
2026-02-16|2026-03-18|Design & review|2|99.99|21||PO-1004
2026-03-02|2026-04-01|Hosting|12|9.95|21|Monthly|PO-1005
2026-03-20|2026-04-19|Consulting|0.5|400.00|21|Half day|PO-1006
2026-04-06|2026-05-06|Training|7|125.00|21||PO-1007
2026-04-21|2026-05-21|Support hours|10|75.50|21|Priority|PO-1008
2026-05-04|2026-06-03|Audit|1|2500.00|21|Annual|PO-1009
2026-05-18|2026-06-17|Migration|4|350.00|21||PO-1010
2026-06-01|2026-07-01|Consulting|6|100.00|21|June|PO-1011
2026-06-15|2026-07-15|Design & review|12.5|99.99|21||PO-1012
2026-07-06|2026-08-05|Support hours|8|75.50|21|Summer cover|PO-1013
2026-07-20|2026-08-19|Workshop|2|600.00|21||PO-1014
2026-08-03|2026-09-02|Hosting|15|9.95|21|Quarterly|PO-1015
2026-08-17|2026-09-16|Implementation|1|999.00|21|Phase two|PO-1016
2026-09-07|2026-10-07|Consulting|20|100.00|21||PO-1017
2026-09-21|2026-10-21|Training|2.5|125.00|21|Team session|PO-1018
2026-10-05|2026-11-04|Support hours|33|75.50|21||PO-1019
2026-11-02|2026-12-02|Year-end review|5|180.00|21|Close|PO-1020
SPECS

# A credit note exercises the CreditNote binding, including BG-3.
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
    --out "$OUT" >/dev/null

documents=$(find "$OUT" -maxdepth 1 -name '*.xml' | wc -l | tr -d ' ')
if [ "$documents" -ne 21 ]; then
    echo "error: expected 21 documents, found $documents" >&2
    exit 1
fi

valid=0
for xml in "$OUT"/*.xml; do
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

echo "Mustang EN 16931 / Peppol BIS 3.0: $valid/$documents documents valid (20 invoices, 1 credit note)"
