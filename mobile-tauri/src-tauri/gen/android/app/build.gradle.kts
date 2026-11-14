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
 * `rustls-platform-verifier` 的 Android 组件（Kotlin AAR，走系统证书库）自 0.7.1 起**不再**随
 * Rust crate 打包，改由上游 GitHub `maven-archive` 分支托管成远程 Maven 仓库。两处必须一起改：
 * 仓库 URL 换成该分支（坐标 groupId 也从 `rustls` 变成 `org.rustls`），依赖声明**刻意不带版本**，
 * 由下面的 `resolutionStrategy` 用 `Cargo.lock` 里的 `rustls-platform-verifier-android` 版本填充 ——
 * Kotlin 组件与 Rust crate 必须同一套语义版本，手抄版本号迟早漂移（#488）。
 *
 * 该组件由 `edu.crabmate.RustlsPlatformVerifier` 经 JNI 初始化；0.2.0 起 AAR 自带一份 manifest，
 * 其中的 `networkSecurityConfig`（CRL 拉取用）由 Android 构建期合并进本应用的 manifest。
 */
repositories {
    maven {
        url =
            uri(
                "https://github.com/rustls/rustls-platform-verifier/raw/maven-archive/android-release-support/maven/"
            )
    }
}

/** 从 `Cargo.lock` 读出 `rustls-platform-verifier-android` 的版本（配置缓存友好，不跑子进程）。 */
abstract class RustlsVersion : ValueSource<String, RustlsVersion.Params> {
    interface Params : ValueSourceParameters {
        val lockFile: RegularFileProperty
    }

    companion object {
        const val CRATE_NAME = "rustls-platform-verifier-android"
    }

    override fun obtain(): String {
        val lines = parameters.lockFile.get().asFile.readLines()
        val nameIdx = lines.indexOfFirst { it.trim() == "name = \"$CRATE_NAME\"" }
        val version =
            if (nameIdx < 0) {
                null
            } else {
                lines.drop(nameIdx + 1)
                    .firstOrNull { it.trimStart().startsWith("version = ") }
                    ?.substringAfter('"', "")
                    ?.substringBefore('"', "")
                    ?.takeIf { it.isNotEmpty() }
            }
        return version ?: error("$CRATE_NAME not found in Cargo.lock")
    }
}

val rustlsPlatformVerifierVersion =
    providers.of(RustlsVersion::class.java) {
        parameters.lockFile.set(layout.projectDirectory.file("../../../Cargo.lock"))
    }

configurations.configureEach {
    resolutionStrategy.eachDependency {
        if (requested.group == "org.rustls" && requested.name == "rustls-platform-verifier") {
            useVersion(rustlsPlatformVerifierVersion.get())
            because("AAR 版本须与 Cargo.lock 的 rustls-platform-verifier-android 完全一致")
        }
    }
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    // rustls 平台验证器的 Android 组件（系统证书库校验；版本由上面的 resolutionStrategy 从 Cargo.lock 定）
    implementation("org.rustls:rustls-platform-verifier")
    // Bearer 加密用平台 AndroidKeyStore（见 SecureBearerStore），勿引入 security-crypto/Tink
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")