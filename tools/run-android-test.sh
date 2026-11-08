#!/usr/bin/env sh
# Runs one instrumented test class from the debug build on one device.
#
# The app and its test APK are installed first, so the run always matches the
# tree the caller just built. The runner prints one line per test and exits
# non-zero when one fails, which `just` passes on.
#
# Usage: run-android-test.sh <device-serial|-> <test-class>
set -eu

serial=${1:--}
test_class=${2:-}
shift 2
package=com.github.sdgoij.bonk
# CI hands over the APKs it built; a local run uses the Gradle outputs.
apk=${BONK_APK:-mobile/androidApp/build/outputs/apk/debug/androidApp-debug.apk}
test_apk=${BONK_TEST_APK:-mobile/androidApp/build/outputs/apk/androidTest/debug/androidApp-debug-androidTest.apk}

if [ -z "$test_class" ]; then
    echo "error: name the test class to run" >&2
    exit 2
fi

for file in "$apk" "$test_apk"; do
    if [ ! -f "$file" ]; then
        echo "error: $file is missing; build it first" >&2
        exit 1
    fi
done

# MSYS rewrites leading-slash arguments into Windows paths; adb needs them as written.
adb_of() {
    if [ -z "$serial" ] || [ "$serial" = "-" ]; then
        env MSYS_NO_PATHCONV=1 adb "$@"
    else
        env MSYS_NO_PATHCONV=1 adb -s "$serial" "$@"
    fi
}

if ! adb_of get-state >/dev/null 2>&1; then
    if [ -n "$serial" ] && [ "$serial" != "-" ]; then
        echo "error: no device is attached with serial $serial" >&2
    else
        echo "error: no single device is available; pass a serial from the list below" >&2
    fi
    echo "attached devices:" >&2
    env MSYS_NO_PATHCONV=1 adb devices -l >&2
    exit 1
fi

if [ "${BONK_SKIP_INSTALL:-0}" != "1" ]; then
    adb_of install -r "$apk" >/dev/null
    adb_of install -r "$test_apk" >/dev/null
fi

# `am instrument` exits 0 even when a test fails, so the output carries the verdict.
result=$(adb_of shell am instrument -w -e class "$test_class" \
    "$@" "$package.test/androidx.test.runner.AndroidJUnitRunner")
printf '%s\n' "$result"

# `OK (0 tests)` means the class was not found, which is a failure of the run.
if ! printf '%s\n' "$result" | grep -qE '^OK \([1-9][0-9]* test'; then
    echo "error: $test_class did not run, or did not pass, on the device" >&2
    exit 1
fi
