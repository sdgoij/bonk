#!/usr/bin/env sh
# Rebuilds the bundled fonts.
#
# The faces are checked in brotli-compressed, so a fresh clone builds offline and
# the binary carries about two thirds of the raw size. Run this only when the
# family or a weight changes: it re-downloads from a pinned commit, verifies the
# bytes against the hashes below, and re-compresses. Needs curl and the brotli
# CLI (Debian/Homebrew `brotli`, MSYS2 `brotli`).
set -eu

cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

# The commit the checked-in assets came from. A branch would move; a commit does
# not, and the hashes below are what actually decides whether a rebuild is
# trustworthy.
NOTO_COMMIT=f8d157532fbfaeda587e826d4cd5b21a49186f7c
BASE="https://github.com/notofonts/noto-cjk/raw/$NOTO_COMMIT/Serif/OTF/SimplifiedChinese"
OUT=crates/render/fonts
WORK=target/fonts

if ! command -v brotli >/dev/null 2>&1; then
    echo "error: the brotli CLI is required to re-compress a face" >&2
    exit 1
fi

mkdir -p "$WORK" "$OUT"

fetch() {
    name=$1
    expected=$2
    raw=$WORK/$name.otf
    if [ ! -f "$raw" ]; then
        echo "downloading $name..."
        curl -L --fail --max-time 900 -o "$raw" "$BASE/$name.otf"
    fi
    actual=$(sha256sum "$raw" | cut -d' ' -f1)
    if [ "$actual" != "$expected" ]; then
        echo "error: $name.otf is not the pinned font (sha256 $actual)" >&2
        exit 1
    fi
    echo "compressing $name..."
    brotli -q 11 -f -o "$OUT/$name.otf.br" "$raw"
    echo "  $OUT/$name.otf.br $(wc -c <"$OUT/$name.otf.br" | tr -d ' ') bytes"
}

fetch NotoSerifCJKsc-Regular 2a2eae2628df83556c54018c41e20fa532c1b862c5256ae8b3f23feb918d12ca
fetch NotoSerifCJKsc-Bold 8af07d4b6c2e82bcc72a30e066eaf295f11b9424f4aad2eaa9fe0e9c3b38fc73

curl -L --fail --max-time 300 -o "$OUT/LICENSE" \
    "https://raw.githubusercontent.com/notofonts/noto-cjk/$NOTO_COMMIT/Serif/LICENSE"

echo "done; run the render tests to confirm the faces inflate and still cover the scripts"
