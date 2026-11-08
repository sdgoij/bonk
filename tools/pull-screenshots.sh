#!/usr/bin/env sh
# Copies the screenshots the test run drew from the device into the assets the
# comparison reads. Used after a run with `-e updateScreenshots 1`.
#
# Usage: pull-screenshots.sh <device-serial|->
set -eu

serial=${1:--}
package=com.github.sdgoij.bonk
destination=mobile/androidApp/src/androidTest/assets/screenshots

adb_of() {
    if [ -z "$serial" ] || [ "$serial" = "-" ]; then
        env MSYS_NO_PATHCONV=1 adb "$@"
    else
        env MSYS_NO_PATHCONV=1 adb -s "$serial" "$@"
    fi
}

if ! adb_of get-state >/dev/null 2>&1; then
    echo "error: no device is attached with serial $serial" >&2
    exit 1
fi

mkdir -p "$destination"
adb_of pull "/sdcard/Android/data/$package/files/screenshots/." "$destination"
echo "goldens: $destination"
