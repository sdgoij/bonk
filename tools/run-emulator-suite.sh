#!/usr/bin/env sh
# Runs the instrumented classes on the emulator against the APKs CI downloaded,
# then measures a cold start of the release build.
#
# The emulator action runs each line of its script in a separate shell, so the
# APK paths cannot be exported in one line and used in the next; this script is
# one shell from start to finish.
#
# Usage: run-emulator-suite.sh <apk-directory>
set -eu

directory=${1:?name the directory the APKs were downloaded into}

find_apk() {
    found=$(find "$directory" -name "$1" -print -quit)
    if [ -z "$found" ] || [ ! -f "$found" ]; then
        echo "error: no $1 under $directory" >&2
        find "$directory" -type f >&2
        exit 1
    fi
    printf '%s' "$found"
}

apk=$(find_apk androidApp-debug.apk)
test_apk=$(find_apk androidApp-debug-androidTest.apk)
release_apk=$(find_apk androidApp-release.apk)

# Installed once; the runner script would otherwise repeat it per class.
adb install -r "$apk" >/dev/null
adb install -r "$test_apk" >/dev/null

export BONK_APK="$apk"
export BONK_TEST_APK="$test_apk"
export BONK_SKIP_INSTALL=1

sh tools/run-android-test.sh - com.github.sdgoij.bonk.ArtefactDeterminismTest
sh tools/run-android-test.sh - com.github.sdgoij.bonk.ShellScreenshotTest
sh tools/run-android-test.sh - com.github.sdgoij.bonk.BudgetTest
BONK_APK="$release_apk" sh tools/measure-cold-start.sh -
