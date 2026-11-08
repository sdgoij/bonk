#!/bin/sh
# Fail when a shipped 64-bit library is not 16 KB page aligned.
#
# Android 15 devices may use 16 KB pages, and Play requires the native libraries
# of apps that target it to be compatible. The alignment comes from the linker,
# so the check belongs next to the cross-build rather than in a review.
set -eu

[ "$#" -gt 0 ] || { echo "usage: tools/check-page-size.sh libbonkc.so..." >&2; exit 2; }
ndk=${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the NDK root}
case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) host=windows-x86_64; exe=.exe ;;
    Darwin*) host=darwin-x86_64; exe= ;;
    *) host=linux-x86_64; exe= ;;
esac
readelf="$ndk/toolchains/llvm/prebuilt/$host/bin/llvm-readelf$exe"
[ -x "$readelf" ] || { echo "no llvm-readelf at $readelf" >&2; exit 1; }

fail=0
for lib in "$@"; do
    [ -f "$lib" ] || { echo "no library at $lib" >&2; exit 1; }
    alignments=$("$readelf" -l "$lib" | awk '$1 == "LOAD" { print $NF }' | sort -u)
    for value in $alignments; do
        if [ "$value" != "0x4000" ]; then
            echo "$lib: a LOAD segment is aligned to $value, not 0x4000" >&2
            fail=1
        fi
    done
    echo "$lib: LOAD align $(echo $alignments)"
done
exit $fail
