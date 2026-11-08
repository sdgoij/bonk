#!/usr/bin/env sh
# Puts a seeded archive into a debuggable Android build.
#
# Android keeps app data private, so the only way in is `run-as`, which the
# platform grants only to a debuggable package. The archive is staged in
# /data/local/tmp first because the app's uid cannot read the host filesystem.
# Whatever archive the device already holds is copied to files/archive.bak
# before it is replaced.
#
# Usage: push-archive.sh <device-serial|-> <directory with bonk.db and keystore.bin> <passphrase>
set -eu

serial=${1:--}
source_dir=${2:-}
passphrase=${3:-}

# The app id is part of the project, not of the device.
package=com.github.sdgoij.bonk
staging=/data/local/tmp/bonk-demo

if [ -z "$source_dir" ] || [ ! -f "$source_dir/bonk.db" ] || [ ! -f "$source_dir/keystore.bin" ]; then
    echo "error: give a directory holding bonk.db and keystore.bin" >&2
    exit 2
fi

# MSYS rewrites leading-slash arguments into Windows paths; adb needs them as written.
adb_of() {
    if [ -z "$serial" ] || [ "$serial" = "-" ]; then
        env MSYS_NO_PATHCONV=1 adb "$@"
    else
        env MSYS_NO_PATHCONV=1 adb -s "$serial" "$@"
    fi
}

if ! adb_of get-state >/dev/null 2>&1; then
    echo "error: no device responded; start an emulator or pass its serial" >&2
    exit 1
fi

if ! adb_of shell pm list packages "$package" | grep -q "$package"; then
    echo "error: $package is not installed; build and install a debug apk first (just mobile-android)" >&2
    exit 1
fi

# Android encrypts app data until the device is unlocked, and a freshly
# installed package has no data directory until it has been started once.
if ! adb_of shell run-as "$package" ls files >/dev/null 2>&1; then
    adb_of shell am start -n "$package/.MainActivity" >/dev/null 2>&1 || true
    sleep 2
fi
if ! adb_of shell run-as "$package" ls files >/dev/null 2>&1; then
    echo "error: the app's storage is not readable; unlock the device and run again" >&2
    exit 1
fi

# Keep the archive that is there, so seeding twice does not lose a test run.
# The remote command is quoted for the device shell: adb joins its arguments
# with spaces and does not re-quote them.
adb_of shell "run-as $package sh -c 'mkdir -p files/archive files/archive.bak; \
    [ -f files/archive/bonk.db ] && cp files/archive/bonk.db files/archive.bak/bonk.db; \
    [ -f files/archive/keystore.bin ] && cp files/archive/keystore.bin files/archive.bak/keystore.bin; true'"

# The app must not hold the database open while it is replaced.
adb_of shell am force-stop "$package"

adb_of shell rm -rf "$staging"
adb_of shell mkdir -p "$staging"
adb_of push "$source_dir/bonk.db" "$staging/bonk.db" >/dev/null
adb_of push "$source_dir/keystore.bin" "$staging/keystore.bin" >/dev/null
adb_of shell chmod 644 "$staging/bonk.db" "$staging/keystore.bin"

# A stale write-ahead log would be replayed over the new database.
adb_of shell "run-as $package sh -c 'rm -f files/archive/bonk.db-wal files/archive/bonk.db-shm; \
    cp $staging/bonk.db $staging/keystore.bin files/archive/'"

adb_of shell rm -rf "$staging"

echo "archive replaced for $package"
echo "previous archive kept in files/archive.bak (readable with adb shell run-as $package)"
echo "passphrase: $passphrase"
echo "open the app on the device and unlock."
