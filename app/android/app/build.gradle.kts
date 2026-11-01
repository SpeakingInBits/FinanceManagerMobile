// Packages the Rust library (built by cargo-ndk) into an APK / Play Store app bundle. The app has
// no Java/Kotlin code: Android's NativeActivity loads libfinance_app.so directly.
plugins {
    id("com.android.application")
}

val rustAbis = listOf("arm64-v8a", "x86_64")
val rustLibDir = layout.buildDirectory.dir("rustJniLibs")

android {
    namespace = "com.speakinginbits.financetracker"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.speakinginbits.financetracker"
        // API 26 (Android 8.0) is the minimum Slint's Android backend supports.
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += rustAbis }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    sourceSets["main"].jniLibs.srcDir(rustLibDir)
}

// Always an optimized Rust build: debug Rust is far slower and the APK is for running on devices.
val cargoBuild by tasks.registering(Exec::class) {
    description = "Builds libfinance_app.so for each ABI with cargo-ndk."
    workingDir = rootDir.parentFile.parentFile
    val args = mutableListOf("cargo", "ndk")
    rustAbis.forEach { args += listOf("-t", it) }
    args += listOf("-P", "26", "-o", rustLibDir.get().asFile.absolutePath, "build", "--release", "-p", "finance-app", "--lib")
    commandLine(args)
}

tasks.named("preBuild") { dependsOn(cargoBuild) }
