// Packages the Rust library (built by cargo-ndk) into an APK / Play Store app bundle. The app has
// no Java/Kotlin code: Android's NativeActivity loads libfinance_app.so directly.
import javax.inject.Inject
import org.gradle.process.ExecOperations

plugins {
    id("com.android.application")
}

val rustAbis = listOf("arm64-v8a", "x86_64")
val minSdkVersion = 26
val compileSdkVersion = 36

android {
    namespace = "com.speakinginbits.financetracker"
    compileSdk = compileSdkVersion

    defaultConfig {
        applicationId = "com.speakinginbits.financetracker"
        // API 26 (Android 8.0) is the minimum Slint's Android backend supports.
        minSdk = minSdkVersion
        targetSdk = compileSdkVersion
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += rustAbis }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
}

/** Builds libfinance_app.so for each ABI with cargo-ndk, into [outputDir]. */
abstract class CargoNdkBuild : DefaultTask() {
    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @get:Input
    abstract val abis: ListProperty<String>

    @get:Input
    abstract val minSdk: Property<Int>

    @get:Input
    abstract val androidJar: Property<String>

    @get:Internal
    abstract val workspaceDir: DirectoryProperty

    @get:Inject
    abstract val execOps: ExecOperations

    init {
        // Cargo tracks the Rust sources itself and is fast when nothing changed.
        outputs.upToDateWhen { false }
    }

    @TaskAction
    fun build() {
        val args = mutableListOf("cargo", "ndk")
        abis.get().forEach { args += listOf("-t", it) }
        // Always an optimized build: debug Rust is far slower and the APK runs on devices.
        args += listOf(
            "-P", minSdk.get().toString(),
            "-o", outputDir.get().asFile.absolutePath,
            "build", "--release", "-p", "finance-app", "--lib",
        )
        execOps.exec {
            workingDir(workspaceDir.get().asFile)
            // cargo-ndk sets ANDROID_PLATFORM to the min SDK, which would make Slint's build
            // script look for that platform's android.jar; point it at the compile SDK's jar.
            environment("ANDROID_JAR", androidJar.get())
            commandLine(args)
        }
    }
}

val cargoBuild = tasks.register<CargoNdkBuild>("cargoBuild") {
    abis.set(rustAbis)
    minSdk.set(minSdkVersion)
    workspaceDir.set(rootProject.layout.projectDirectory.dir("../.."))
    androidJar.set(
        androidComponents.sdkComponents.sdkDirectory.map {
            it.file("platforms/android-$compileSdkVersion/android.jar").asFile.absolutePath
        },
    )
}

androidComponents {
    onVariants { variant ->
        variant.sources.jniLibs?.addGeneratedSourceDirectory(cargoBuild, CargoNdkBuild::outputDir)
    }
}
