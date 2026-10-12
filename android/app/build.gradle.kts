plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "dev.insd.naru"
    // Material 3 Expressive(material3 1.5)가 API 37 이상으로 컴파일할 것을 요구합니다.
    compileSdk {
        version = release(37)
    }

    defaultConfig {
        applicationId = "dev.insd.naru"
        // MediaCodecInfo.isHardwareAccelerated가 29부터 있습니다.
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"

        // Rust 프로토콜 크레이트는 arm64만 빌드합니다.
        ndk { abiFilters += "arm64-v8a" }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    // Rust 프로토콜 크레이트와 그 Kotlin 바인딩은 build-protocol.sh가 만듭니다.
    sourceSets {
        getByName("main") {
            jniLibs.directories += "build/generated/jniLibs"
            kotlin.directories += "build/generated/uniffi"
        }
    }
}

val buildProtocol by tasks.registering(Exec::class) {
    description = "Rust 프로토콜 크레이트를 Android용으로 빌드하고 Kotlin 바인딩을 만듭니다"
    workingDir = rootDir
    commandLine("sh", "build-protocol.sh")
    environment("ANDROID_HOME", androidComponents.sdkComponents.sdkDirectory.get().asFile.absolutePath)
    val protocol = rootDir.resolve("../protocol")
    inputs.dir(protocol.resolve("src"))
    inputs.files(
        protocol.resolve("Cargo.toml"),
        protocol.resolve("Cargo.lock"),
        protocol.resolve("uniffi.toml"),
        rootDir.resolve("build-protocol.sh"),
    )
    outputs.dirs("build/generated/jniLibs", "build/generated/uniffi")
}

tasks.named("preBuild") { dependsOn(buildProtocol) }

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
