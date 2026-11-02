#!/usr/bin/env sh
# M1 external validation gate.
#
# Generates the conformance corpus through the CLI and validates every UBL file with
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
# An EU buyer outside the Netherlands, for intra-community supply: it needs a
# VAT identifier and a country of its own.
c3=$("$BONK_BIN" customer-add --db "$DB" --keystore "$KEY" --passphrase "$PASS" --name "Gamma Customer BV" --city Antwerpen --country BE --street "Gamma 3" --postal "2000" --vat BE0417497106 --peppol 0208:0417497106 | field customer)
c4=$("$BONK_BIN" customer-add --db "$DB" --keystore "$KEY" --passphrase "$PASS" --name "Delta Kunde GmbH" --city Koeln --country DE --street "Delta 4" --postal "50667" --vat DE123456789 --peppol 9930:DE123456789 | field customer)
# A buyer outside the union, for export.
c5=$("$BONK_BIN" customer-add --db "$DB" --keystore "$KEY" --passphrase "$PASS" --name "Epsilon AS" --city Bergen --country NO --street "Epsilon 5" --postal "5003" --peppol 0192:123456789 | field customer)

n=0

# Issues one draft from `draft-new` arguments, so the multi-line invoices below do
# not repeat the bookkeeping.
issue_draft() {
    draft=$("$BONK_BIN" "$@" | field draft)
    if [ -z "$draft" ]; then
        echo "error: could not create a draft" >&2
        exit 1
    fi
    "$BONK_BIN" issue "$draft" \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --out "$OUT" >/dev/null
}

# buyer|date|due|delivery|item|qty|price|rate|category|unit|code|reason|note|buyer_ref|order_ref
#
# The buyer and the category are explicit even where they could be inferred: a
# corpus that only ever says "S" cannot exercise the rules for the other eight
# codes, and intra-community supply and export need a buyer country the modulo
# assignment above could not guarantee.
while IFS='|' read -r who date due delivery item qty price rate category unit code reason note buyer_ref order_ref; do
    [ -n "$date" ] || continue
    n=$((n + 1))

    case "$who" in
        1) customer=$c1 ;;
        2) customer=$c2 ;;
        3) customer=$c3 ;;
        4) customer=$c4 ;;
        5) customer=$c5 ;;
        *) echo "error: unknown buyer $who on row $n" >&2; exit 1 ;;
    esac

    set -- draft-new \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --seller "$seller" --customer "$customer" \
        --date "$date"
    [ -z "$due" ] || set -- "$@" --due "$due"
    [ -z "$delivery" ] || set -- "$@" --delivery "$delivery"
    set -- "$@" --line "$item|$qty|$price|$rate|$category|$unit|$code|$reason"
    [ -z "$note" ] || set -- "$@" --note "$note"
    [ -z "$buyer_ref" ] || set -- "$@" --buyer-ref "$buyer_ref"
    [ -z "$order_ref" ] || set -- "$@" --order-ref "$order_ref"

    issue_draft "$@"
done <<'SPECS'
1|2026-01-05|2026-02-04||Consulting|1|100.00|21|S|HUR|||January retainer|PO-1001|
1|2026-01-19|2026-02-18||Support hours|3|75.50|21|S|HUR|||Support|PO-1002|
2|2026-02-02|2026-03-04|2026-02-02|Implementation|1|1250.00|21|S||||Phase one|PO-1003|SO-1
2|2026-02-16|2026-03-18||Design & review|2|99.99|21|S|||||PO-1004|
1|2026-03-02|2026-04-01||Hosting|12|9.95|21|S|MON|||Monthly|PO-1005|
1|2026-03-20|2026-04-19||Consulting|0.5|400.00|21|S|HUR|||Half day|PO-1006|
2|2026-04-06|2026-05-06|2026-04-06|Training|7|125.00|21|S|||||PO-1007|
2|2026-04-21|2026-05-21||Support hours|10|75.50|21|S|HUR|||Priority|PO-1008|
1|2026-05-04|2026-06-03||Audit|1|2500.00|21|S||||Annual|PO-1009|SO-2
1|2026-05-18|2026-06-17||Migration|4|350.00|21|S|||||PO-1010|
2|2026-06-01|2026-07-01|2026-06-01|Consulting|6|100.00|21|S|HUR|||June|PO-1011|
2|2026-06-15|2026-07-15||Design & review|12.5|99.99|21|S|||||PO-1012|
1|2026-07-06|2026-08-05||Support hours|8|75.50|21|S|HUR|||Summer cover|PO-1013|
1|2026-07-20|2026-08-19|2026-07-20|Workshop|2|600.00|21|S|||||PO-1014|
2|2026-08-03|2026-09-02||Hosting|15|9.95|21|S|MON|||Quarterly|PO-1015|
2|2026-08-17|2026-09-16||Implementation|1|999.00|21|S||||Phase two|PO-1016|
1|2026-09-07|2026-10-07|2026-09-07|Consulting|20|100.00|21|S|HUR||||PO-1017|
1|2026-09-21|2026-10-21||Training|2.5|125.00|21|S||||Team session|PO-1018|
2|2026-10-05|2026-11-04||Support hours|33|75.50|21|S|HUR||||PO-1019|
2|2026-11-02|2026-12-02|2026-11-02|Year-end review|5|180.00|21|S||||Close|PO-1020|
1|2026-09-10|2026-10-10||Consulting|12|100.00|21|S|HUR|||September|PO-1021|
2|2026-09-24|2026-10-24||Support hours|4|75.50|21|S|HUR|||October|PO-1022|
1|2026-01-09|2026-02-08||Handbook print|10|25.00|9|S||||Books|PO-1101|
2|2026-01-23|2026-02-22||Course material|5|40.00|9|S|||||PO-1102|
1|2026-02-06|2026-03-08|2026-02-06|Catering|20|12.50|9|S||||Lunch|PO-1103|
2|2026-02-20|2026-03-22||Magazine subscription|1|89.00|9|S|||||PO-1104|
1|2026-03-06|2026-04-05||E-book licence|3|19.95|9|S|||||PO-1105|
2|2026-03-24|2026-04-23|2026-03-24|Printed manual|15|15.00|9|S|||||PO-1106|
1|2026-04-10|2026-05-10||Training material|4|35.00|9|S|||||PO-1107|
2|2026-04-24|2026-05-24||Bookshelf set|1|149.00|9|S|||||PO-1108|
1|2026-05-08|2026-06-07||Interim dividend fee|1|500.00|0|Z|||||PO-1201|
2|2026-05-22|2026-06-21||Settlement fee|1|75.00|0|Z|||||PO-1202|
1|2026-06-05|2026-07-05|2026-06-05|Zero-rated supply|2|300.00|0|Z|||||PO-1203|
2|2026-06-19|2026-07-19||Zero-rated service|1|220.00|0|Z|||||PO-1204|
1|2026-07-03|2026-08-02||Administration fee|1|60.00|0|Z|||||PO-1205|
2|2026-07-17|2026-08-16|2026-07-17|Zero-rated delivery|3|95.00|0|Z|||||PO-1206|
1|2026-08-07|2026-09-06||Zero-rated consultancy|4|110.00|0|Z|||||PO-1207|
2|2026-08-21|2026-09-20||Zero-rated hosting|1|45.00|0|Z|||||PO-1208|
1|2026-09-04|2026-10-04||Medical device advice|1|450.00|0|E|||Exempt supply (health care)||PO-1301|
2|2026-09-18|2026-10-18|2026-09-18|Education services|8|150.00|0|E|||Exempt supply (education)||PO-1302|
1|2026-10-02|2026-11-01||Insurance broker fee|1|200.00|0|E|||Exempt supply (insurance)||PO-1303|
2|2026-10-16|2026-11-15||Financial advice|3|175.00|0|E|||Exempt supply (finance)||PO-1304|
1|2026-10-30|2026-11-29|2026-10-30|Care services|6|90.00|0|E|||Exempt supply (care)||PO-1305|
2|2026-11-13|2026-12-13||Membership fee|1|300.00|0|E|||Exempt supply (non-profit)||PO-1306|
3|2026-01-15|2026-02-14||Subcontracting|1|1200.00|0|AE|||Reverse charge||PO-1401|
4|2026-01-29|2026-02-28|2026-01-29|Cross-border consulting|2|650.00|0|AE|||Reverse charge||PO-1402|
3|2026-02-12|2026-03-14||Installation work|1|3400.00|0|AE|||Reverse charge||PO-1403|
4|2026-02-26|2026-03-28||Technical review|5|210.00|0|AE|||Reverse charge||PO-1404|
3|2026-03-12|2026-04-11|2026-03-12|Assembly on site|1|2100.00|0|AE|||Reverse charge||PO-1405|
4|2026-03-26|2026-04-25||Engineering support|8|160.00|0|AE|||Reverse charge||PO-1406|
3|2026-04-09|2026-05-09|2026-04-09|Goods delivered|10|250.00|0|K|||Intra-community supply||PO-1501|
4|2026-04-23|2026-05-23|2026-04-23|Machinery parts|2|1800.00|0|K|||Intra-community supply||PO-1502|
3|2026-05-07|2026-06-06|2026-05-07|Components|25|40.00|0|K|||Intra-community supply||PO-1503|
4|2026-05-21|2026-06-20|2026-05-21|Equipment|1|5200.00|0|K|||Intra-community supply||PO-1504|
3|2026-06-04|2026-07-04|2026-06-04|Consumables|50|12.50|0|K|||Intra-community supply||PO-1505|
4|2026-06-18|2026-07-18|2026-06-18|Spare parts|12|95.00|0|K|||Intra-community supply||PO-1506|
5|2026-07-09|2026-08-08|2026-07-09|Goods exported|5|400.00|0|G|||Export outside the EU||PO-1601|
5|2026-07-23|2026-08-22|2026-07-23|Export consultancy|3|500.00|0|G|||Export outside the EU||PO-1602|
5|2026-08-06|2026-09-05|2026-08-06|Export equipment|1|2600.00|0|G|||Export outside the EU||PO-1603|
5|2026-08-20|2026-09-19|2026-08-20|Export software licence|2|750.00|0|G|||Export outside the EU||PO-1604|
SPECS

# Three invoices carry several lines, so the VAT breakdown has more than one group
# and the totals have to add up across categories.
issue_draft draft-new --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
    --seller "$seller" --customer "$c1" --date 2026-12-01 --due 2026-12-31 \
    --line "Consulting|8|100.00|21|S|HUR" \
    --line "Handbook print|5|25.00|9|S" \
    --buyer-ref PO-1701 --note "Two rates"

issue_draft draft-new --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
    --seller "$seller" --customer "$c2" --date 2026-12-02 --due 2027-01-01 \
    --line "Support hours|6|75.50|21|S|HUR" \
    --line "Zero-rated service|1|220.00|0|Z" \
    --buyer-ref PO-1702

issue_draft draft-new --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
    --seller "$seller" --customer "$c4" --date 2026-12-03 --due 2027-01-02 \
    --delivery 2026-12-03 \
    --line "Engineering support|4|160.00|0|AE|||Reverse charge" \
    --line "Goods delivered|2|300.00|0|K|||Intra-community supply" \
    --line "Consulting|3|100.00|21|S|HUR" \
    --buyer-ref PO-1703 --note "Mixed treatment"

# Credit notes exercise the CreditNote binding, including BG-3.
for invoice in 2026-0001 2026-0002 2026-0003; do
    credit=$(
        "$BONK_BIN" credit-note "$invoice" \
            --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
            --date 2026-12-15 |
            field "credit note"
    )
    if [ -z "$credit" ]; then
        echo "error: could not create a credit note for $invoice" >&2
        exit 1
    fi
    "$BONK_BIN" issue "$credit" \
        --db "$DB" --keystore "$KEY" --passphrase "$PASS" \
        --out "$OUT" >/dev/null
done

documents=$(find "$OUT" -maxdepth 1 -name '*.xml' | wc -l | tr -d ' ')
if [ "$documents" -ne 66 ]; then
    echo "error: expected 66 documents, found $documents" >&2
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

echo "Mustang EN 16931 / Peppol BIS 3.0: $valid/$documents documents valid (63 invoices, 3 credit notes)"
