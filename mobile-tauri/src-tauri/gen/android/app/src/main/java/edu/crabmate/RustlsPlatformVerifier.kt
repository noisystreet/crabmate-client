package edu.crabmate

import android.content.Context

/**
 * `rustls-platform-verifier` 的 Android 宿主初始化桥。
 *
 * reqwest 的 `rustls` 后端在 Android 上走平台验证器（系统证书库）；其进程级单例必须在
 * **任何 TLS 请求之前**经 JNI 注入 `Context`，否则首次请求会 panic 并被 Tauri 入口 abort，
 * 表现为连接页一直停在「正在连接」。
 *
 * 本地实现见 `src-tauri/src/android_tls.rs`（同属 `crabmate_mobile_lib`）。
 */
internal object RustlsPlatformVerifier {
  init {
    // 与 tauri 生成的 `Rust` 对象同库；重复 load 由 JVM 去重。
    System.loadLibrary("crabmate_mobile_lib")
  }

  /** 进程内调用一次即可；验证器在整个进程生命周期内有效。 */
  @JvmStatic
  external fun nativeInitRustlsPlatformVerifier(context: Context)
}