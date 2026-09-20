import groovy.json.JsonSlurper
import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

val keystorePropertiesFile = file("key.properties")
val keystoreProperties = Properties().apply {
    if (keystorePropertiesFile.exists()) {
        keystorePropertiesFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 36
    namespace = "edu.crabmate"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "edu.crabmate"
        minSdk = 24
        targetSdk = 36
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    signingConfigs {
        if (keystorePropertiesFile.exists()) {
            create("release") {
                keyAlias = keystoreProperties.getProperty("keyAlias")
                keyPassword = keystoreProperties.getProperty("keyPassword")
                storeFile = file(keystoreProperties.getProperty("storeFile"))
                storePassword = keystoreProperties.getProperty("storePassword")
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            // 局域网 WebView 仍需明文 HTTP；公网 http 由 crabmate-connect 连接策略拒绝（须 HTTPS）。
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isMinifyEnabled = true
            if (keystorePropertiesFile.exists()) {
                signingConfig = signingConfigs.getByName("release")
            }
            proguardFiles(
                *fileTree(".") { include("**/*.pro") }
                    .plus(getDefaultProguardFile("proguard-android-optimize.txt"))
                    .toList().toTypedArray()
            )
        }
    }
    kotlinOptions {
        // 与昨日可用构建对齐；JDK 21 下会有 source/target 8 弃用警告，但不引入新依赖面
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
}

rust {
    rootDirRel = "../../../"
}

// 产物文件名：crabmate.apk（勿用默认 app-universal-release.apk）
android.applicationVariants.configureEach {
    outputs.configureEach {
        (this as com.android.build.gradle.internal.api.BaseVariantOutputImpl).outputFileName =
            if (buildType.name == "release") {
                "crabmate.apk"
            } else {
                "crabmate-${buildType.name}.apk"
            }
    }
}

/**
 * `rustls-platform-verifier` 的 Android 组件（Kotlin AAR，走系统证书库）随 Rust crate 分发、
 * 未发布到 Maven Central：用 `cargo metadata` 定位 crate 解包目录里的本地 maven 仓库，
 * 版本以 Cargo.lock 为准。该组件由 `edu.crabmate.RustlsPlatformVerifier` 经 JNI 初始化。
 */
val (rustlsPlatformVerifierVersion, rustlsPlatformVerifierMavenRepo) =
    run {
        val metadata =
            providers.exec {
                commandLine(
                    "cargo",
                    "metadata",
                    "--format-version",
                    "1",
                    "--filter-platform",
                    "aarch64-linux-android",
                    "--manifest-path",
                    file("../../../Cargo.toml").absolutePath,
                )
            }.standardOutput.asText.get()
        val packages =
            ((JsonSlurper().parseText(metadata) as Map<*, *>)["packages"] as List<*>)
                .map { it as Map<*, *> }
        val pkg = packages.first { it["name"] == "rustls-platform-verifier-android" }
        (pkg["version"] as String) to
            File(File(pkg["manifest_path"] as String).parentFile, "maven").absolutePath
    }

repositories {
    maven {
        url = uri(rustlsPlatformVerifierMavenRepo)
        metadataSources.artifact()
    }
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    // rustls 平台验证器的 Android 组件（系统证书库校验；随 Rust crate 分发，见上）
    implementation("rustls:rustls-platform-verifier:$rustlsPlatformVerifierVersion")
    // Bearer 加密用平台 AndroidKeyStore（见 SecureBearerStore），勿引入 security-crypto/Tink
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")