#!/bin/sh
# Cross-build for Android with the Unix-path Perl that OpenSSL's Configure needs.
#
# The Strawberry Perl used for host builds identifies as MSWin32 and refuses to
# produce the Unix paths OpenSSL wants when targeting Android. The Perl shipped
# with Git for Windows is Unix-like but lacks ExtUtils::MakeMaker, so the required
# pure-Perl modules are staged from Strawberry into the ignored target directory.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
root=$(cygpath -u "$root")
strawberry_lib="${STRAWBERRY_PERL_LIB:-/c/Strawberry/perl/lib}"
target_lib="$root/target/perl-lib"

if [ ! -d "$target_lib/ExtUtils" ]; then
    mkdir -p "$target_lib"
    for module in ExtUtils Locale Pod; do
        cp -R "$strawberry_lib/$module" "$target_lib/"
    done
fi

# Git Bash's perl lives under a path with a space. OpenSSL's Makefile embeds
# $(PERL) unquoted, so use the 8.3 short form with forward slashes.
perl_exe=$(cygpath -d /usr/bin/perl | tr '\\' '/')
export PERL="$perl_exe"
# Git Bash rewrites POSIX paths in the environment when it launches a native
# executable; Cygwin Perl then sees a drive-letter path and splits it at the
# colon. Keep this one in POSIX form.
export MSYS2_ENV_CONV_EXCL="PERL5LIB"
export PERL5LIB="$target_lib"
PATH="/usr/bin:$PATH"
export PATH

exec cargo ndk "$@"
