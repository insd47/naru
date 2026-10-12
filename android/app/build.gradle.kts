plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "dev.insd.naru"
    // Material 3 Expressive (material3 1.5) requires compiling against API 37 or later.
    compileSdk {
        version = release(37)
    }

    defaultConfig {
        applicationId = "dev.insd.naru"
        // MediaCodecInfo.isHardwareAccelerated exists from API 29.
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"

        // The Rust protocol crate is built for arm64 only.
        ndk { abiFilters += "arm64-v8a" }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    // buildCore and generateBindings below produce these. They are not committed.
    sourceSets {
        getByName("main") {
            jniLibs.directories += "build/generated/jniLibs"
            kotlin.directories += "build/generated/uniffi"
        }
    }
}

val core = rootDir.resolve("..")
val jniLibs = file("build/generated/jniLibs")
val bindings = file("build/generated/uniffi")

// Android Studio launched from the Dock or the Start menu does not inherit the shell's PATH, so cargo is found in
// ~/.cargo/bin first.
val cargoBin = File(System.getProperty("user.home"), ".cargo/bin")
val cargo = listOf("cargo.exe", "cargo").map(cargoBin::resolve).firstOrNull(File::exists)?.path ?: "cargo"
val cargoPath = cargoBin.path + File.pathSeparator + System.getenv("PATH")

val buildCore by tasks.registering(Exec::class) {
    description = "Builds the Rust core crate for Android"
    workingDir = core
    // Crypto and SRTP are too slow in debug builds, so this always builds release regardless of the app's build type.
    commandLine(
        cargo, "ndk", "-t", "arm64-v8a", "-P", "${android.defaultConfig.minSdk}", "-o", jniLibs.path,
        "build", "--release",
    )
    environment("PATH", cargoPath)
    environment("ANDROID_HOME", androidComponents.sdkComponents.sdkDirectory.get().asFile.absolutePath)
    inputs.dir(core.resolve("src"))
    inputs.files(core.resolve("Cargo.toml"), core.resolve("Cargo.lock"))
    outputs.dir(jniLibs)
}

val generateBindings by tasks.registering(Exec::class) {
    description = "Generates the Kotlin bindings from the metadata of the core library"
    dependsOn(buildCore)
    workingDir = core
    val library = jniLibs.resolve("arm64-v8a/libnaru.so")
    commandLine(
        cargo, "run", "-q", "--example", "uniffi-bindgen", "--",
        "generate", library.path, "-l", "kotlin", "-o", bindings.path, "-n",
    )
    environment("PATH", cargoPath)
    doFirst { bindings.deleteRecursively() }
    inputs.files(library, core.resolve("uniffi.toml"))
    outputs.dir(bindings)
}

tasks.named("preBuild") { dependsOn(generateBindings) }

dependencies {
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.jna) { artifact { type = "aar" } }
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.foundation)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material3.navigation.suite)
    implementation(libs.androidx.compose.material.icons.extended)
    testImplementation(libs.junit)
}
