import org.gradle.api.tasks.Exec

plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.android.kotlin.multiplatform.library) apply false
    alias(libs.plugins.kotlin.multiplatform) apply false
    alias(libs.plugins.compose.multiplatform) apply false
    alias(libs.plugins.compose.compiler) apply false
}

// Native build and binding generation.
//
// `-Pnative` makes both modules' `preBuild` depend on `uniffiBindings`, so one
// command regenerates the bindings and the per-ABI libraries. Without the flag
// the build uses what is already in `androidApp/src/main/jniLibs`, which lets a
// developer without the NDK compile Kotlin.
val repoRoot = rootProject.projectDir.parentFile
val cargoCommand = if (System.getProperty("os.name").startsWith("Windows", ignoreCase = true)) {
    listOf("sh", "tools/cargo-strawberry.sh")
} else {
    listOf("cargo")
}

val cargoNdk = tasks.register<Exec>("cargoNdk") {
    workingDir = repoRoot
    if (System.getProperty("os.name").startsWith("Windows", ignoreCase = true)) {
        commandLine(
            listOf("sh", "tools/cargo-android.sh") + listOf(
                "--platform", "26",
                "-t", "arm64-v8a", "-t", "x86_64",
                "-o", "mobile/androidApp/src/main/jniLibs",
                "build", "--release", "-p", "ffi",
            ),
        )
    } else {
        commandLine(
            listOf("cargo", "ndk") + listOf(
                "--platform", "26",
                "-t", "arm64-v8a", "-t", "x86_64",
                "-o", "mobile/androidApp/src/main/jniLibs",
                "build", "--release", "-p", "ffi",
            ),
        )
    }
}

val uniffiBindings = tasks.register<Exec>("uniffiBindings") {
    dependsOn(cargoNdk)
    workingDir = repoRoot
    commandLine(
        cargoCommand + listOf(
            "run", "-p", "ffi", "--features", "cli", "--bin", "uniffi-bindgen", "--",
            "generate", "--library", "target/aarch64-linux-android/release/libbonkc.so",
            "--language", "kotlin", "--out-dir", "mobile/shared/build/generated/uniffi",
        ),
    )
}

val stripNdk = tasks.register<Exec>("stripNdk") {
    dependsOn(uniffiBindings)
    workingDir = repoRoot
    commandLine("sh", "tools/strip-android.sh")
}

if (providers.gradleProperty("native").isPresent) {
    // The application module owns the native libraries, so its preBuild must run
    // the NDK build before packaging. The shared module compiles the generated
    // Kotlin, so its Android compilation depends on the generator too.
    project(":androidApp").tasks.matching {
        it.name.startsWith("merge") && it.name.endsWith("JniLibFolders")
    }.configureEach {
        dependsOn(stripNdk)
    }
    project(":shared").tasks.matching { it.name == "compileAndroidMain" }.configureEach {
        dependsOn(uniffiBindings)
    }
}
