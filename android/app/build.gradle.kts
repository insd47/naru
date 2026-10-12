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

    // build-core.sh builds the Rust core crate and its Kotlin bindings.
    sourceSets {
        getByName("main") {
            jniLibs.directories += "build/generated/jniLibs"
            kotlin.directories += "build/generated/uniffi"
        }
    }
}

val buildCore by tasks.registering(Exec::class) {
    description = "Builds the Rust core crate for Android and generates its Kotlin bindings"
    workingDir = rootDir
    commandLine("sh", "build-core.sh")
    environment("ANDROID_HOME", androidComponents.sdkComponents.sdkDirectory.get().asFile.absolutePath)
    val core = rootDir.resolve("..")
    inputs.dir(core.resolve("src"))
    inputs.files(
        core.resolve("Cargo.toml"),
        core.resolve("Cargo.lock"),
        core.resolve("uniffi.toml"),
        rootDir.resolve("build-core.sh"),
    )
    outputs.dirs("build/generated/jniLibs", "build/generated/uniffi")
}

tasks.named("preBuild") { dependsOn(buildCore) }

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
