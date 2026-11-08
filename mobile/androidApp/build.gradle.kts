plugins {
    alias(libs.plugins.android.application)
}

// The application module is the Android shell: identity, manifest, native
// libraries, and R8. All Kotlin and Compose code lives in `:shared`, which keeps
// the KMP plugin out of an application module (AGP 9 rejects that combination).
android {
    namespace = "com.github.sdgoij.bonk"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.github.sdgoij.bonk"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        // The instrumented test asserts the golden artefact hashes on a device.
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // The .so files are produced by `cargoNdk` into the default
    // `src/main/jniLibs` directory.

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            // Spike only: sign with the debug key so the R8 build can be installed.
            signingConfig = signingConfigs.getByName("debug")
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro",
            )
        }
    }
}

dependencies {
    implementation(project(":shared"))

    androidTestImplementation(libs.androidx.test.junit)
    androidTestImplementation(libs.androidx.test.runner)
}
