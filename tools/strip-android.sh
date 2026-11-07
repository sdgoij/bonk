#!/bin/sh
# Strip the Android libraries cargo-ndk produced. AGP's own strip cannot handle
# the Rust .so here, so the release APK would otherwise carry symbols and debug
# sections from every dependency.
set -eu

ndk=${ANDROID_NDK_HOME:?set ANDROID_NDK_HOME to the NDK root}
case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) host=windows-x86_64; exe=.exe ;;
    Darwin*) host=darwin-x86_64; exe= ;;
    *) host=linux-x86_64; exe= ;;
esac

strip="$ndk/toolchains/llvm/prebuilt/$host/bin/llvm-strip$exe"
for abi in arm64-v8a x86_64; do
    "$strip" --strip-unneeded "mobile/androidApp/src/main/jniLibs/$abi/libbonkc.so"
done
