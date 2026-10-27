#!/bin/sh
# Build/test with the bundled SQLCipher stack on Windows.
#
# OpenSSL's Configure script is Perl. The Perl shipped with Git for Windows is
# stripped and cannot build it (no ExtUtils::MakeMaker), so this wrapper puts
# Strawberry Perl first on PATH. Install it from https://strawberryperl.com/
# and adjust the path below if it lives elsewhere.
set -eu

STRAWBERRY_PERL_BIN="${STRAWBERRY_PERL_BIN:-/c/Strawberry/perl/bin}"
if [ ! -x "$STRAWBERRY_PERL_BIN/perl.exe" ]; then
    echo "cargo-strawberry: no perl.exe under $STRAWBERRY_PERL_BIN" >&2
    echo "install Strawberry Perl or set STRAWBERRY_PERL_BIN" >&2
    exit 1
fi

PATH="$STRAWBERRY_PERL_BIN:$PATH"
export PATH
exec cargo "$@"
