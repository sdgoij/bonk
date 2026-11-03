#!/usr/bin/env sh
# Expected-findings gate.
#
# The corpus gate proves that valid documents pass the official validator. This
# one proves the other half: a document that breaks one rule is reported as
# breaking that rule and no other, so a field that quietly stops being written is
# caught by the official artefacts rather than by our engine agreeing with
# itself.
#
# It also runs our own rule engine over the same files and compares the two
# findings rule by rule. A rule only the authority reports is a gap — work the
# engine has not done yet — and is printed; a rule only we report would block an
# invoice the authority accepts, and fails the gate.
#
# The fixtures are mutations of one issued invoice, so they cannot drift away
# from what the writer actually produces. Run tools/validate-m1.sh once first: it
# issues the invoice and fetches the validator.
set -eu

cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

MUSTANG_VERSION=2.17.0
MUSTANG_JAR=${MUSTANG_JAR:-target/validator/Mustang-CLI-$MUSTANG_VERSION.jar}
JAVA_BIN=${JAVA:-java}
SOURCE=${1:-target/m1-validation/out/2026-0001.xml}
WORK=target/m1-validation/findings
BASE=$WORK/base.xml

if [ ! -f "$SOURCE" ]; then
    echo "error: $SOURCE is missing; run tools/validate-m1.sh first" >&2
    exit 1
fi
if [ ! -f "$MUSTANG_JAR" ]; then
    echo "error: $MUSTANG_JAR is missing; run tools/validate-m1.sh once to fetch it" >&2
    exit 1
fi
if ! command -v "$JAVA_BIN" >/dev/null 2>&1; then
    echo "error: Java 17+ is required to run Mustang" >&2
    exit 1
fi

rm -rf "$WORK"
mkdir -p "$WORK"
cp "$SOURCE" "$BASE"

# The rule identifiers the validator fired, in sorted order and deduplicated.
# The validator mentions each identifier more than once — in the log line and in
# the report XML — so the set is what is compared, not the text.
rules_of() {
    grep -o 'BR-[A-Z0-9][A-Z0-9-]*' | sort -u | tr '\n' ' ' | sed 's/ $//'
}

fail=0
check() {
    name=$1
    expected=$2
    output=$("$JAVA_BIN" -jar "$MUSTANG_JAR" \
        --action validate --source "$WORK/$name.xml" \
        --no-notices --disable-file-logging 2>&1) || true
    actual=$(printf '%s\n' "$output" | rules_of)
    if [ "$actual" = "$expected" ]; then
        printf 'ok    %-12s %s\n' "$name" "$actual"
    else
        printf 'FAIL  %-12s expected "%s", got "%s"\n' "$name" "$expected" "$actual" >&2
        fail=1
    fi
}

# 1. The amount due is positive, so BR-CO-25 wants a due date or payment terms.
perl -ne 'print unless /<cbc:DueDate>/' "$BASE" > "$WORK/no-due-date.xml"
check no-due-date "BR-CO-25"

# 2. A line net that no longer equals quantity times price: the line arithmetic
#    (BR-CO-13) and the category rule for the standard rate (BR-S-08).
perl -pe 's{(<cbc:LineExtensionAmount currencyID="EUR">)100\.00}{${1}90.00}' "$BASE" > "$WORK/line-net.xml"
check line-net "BR-CO-13 BR-S-08"

# 3. The invoice tax total no longer matches the breakdown (BR-CO-14, BR-CO-15).
perl -pe 'if (!$done && m{<cbc:TaxAmount currencyID="EUR">21\.00}) { s{>21\.00}{>20.00}; $done = 1 }' "$BASE" > "$WORK/tax-total.xml"
check tax-total "BR-CO-14 BR-CO-15"

if [ "$fail" -ne 0 ]; then
    exit 1
fi

echo "expected findings: 3/3 documents report exactly the rules they were built to break"

# The same files, read by our engine. This is the only place both verdicts are
# produced for one document, so it is where a rule we apply and the authority
# does not — a finding that would block a valid invoice — is caught.
MUSTANG_JAR="$MUSTANG_JAR" JAVA="$JAVA_BIN" \
    cargo run -q -p xtask -- compare "$WORK"
