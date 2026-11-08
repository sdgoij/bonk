#!/usr/bin/env sh
# Cold start of the release build to the lock screen, as the platform reports it.
#
# `am start -W -S` starts a fresh process and reports the time until the first
# frame. The design's 2.0 s budget is for a mid-range phone; this script only
# refuses a start that took five seconds, so an emulator run catches a
# regression without pretending to be the device measurement.
#
# Usage: measure-cold-start.sh <device-serial|->
set -eu

serial=${1:--}
package=com.github.sdgoij.bonk
# CI hands over the APK it built; a local run uses the Gradle output.
apk=${BONK_APK:-mobile/androidApp/build/outputs/apk/release/androidApp-release.apk}

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
if [ ! -f "$apk" ]; then
    echo "error: $apk is missing; run ./gradlew :androidApp:assembleRelease first" >&2
    exit 1
fi

adb_of install -r --no-streaming "$apk" >/dev/null
adb_of shell am force-stop "$package"
adb_of logcat -c

output=$(adb_of shell am start -W -S -n "$package/.MainActivity")
printf '%s\n' "$output"

total=$(printf '%s\n' "$output" | sed -n 's/^TotalTime: //p')
if [ -z "$total" ]; then
    echo "error: the platform did not report a TotalTime" >&2
    exit 1
fi
echo "cold start: ${total} ms"
test "$total" -lt 5000
