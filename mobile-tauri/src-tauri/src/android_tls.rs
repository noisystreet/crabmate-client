//! Android 平台证书验证器（`rustls-platform-verifier`）的宿主初始化。
//!
//! reqwest 0.13 的 `rustls` feature 固定使用平台验证器（`dep:rustls-platform-verifier`）。
//! Android 上该验证器的进程级单例必须**在任何 TLS 请求之前**由 JVM 注入 `Context`，
//! 否则请求路径调用 `global()` 会 panic（`Expect rustls-platform-verifier to be initialized`），
//! 表现为连接页一直停在「正在连接」。
//!
//! Kotlin 调用方：`gen/android/app/src/main/java/edu/crabmate/RustlsPlatformVerifier.kt`。

use jni::objects::JObject;
use jni::{EnvUnowned, errors::ThrowRuntimeExAndDefault};

/// `RustlsPlatformVerifier.nativeInitRustlsPlatformVerifier(Context)` 的本地实现。
#[unsafe(no_mangle)]
pub extern "system" fn Java_edu_crabmate_RustlsPlatformVerifier_nativeInitRustlsPlatformVerifier<
    'local,
>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JObject<'local>,
    context: JObject<'local>,
) {
    unowned_env
        .with_env(|env| rustls_platform_verifier::android::init_with_env(env, context))
        .resolve::<ThrowRuntimeExAndDefault>();
}
