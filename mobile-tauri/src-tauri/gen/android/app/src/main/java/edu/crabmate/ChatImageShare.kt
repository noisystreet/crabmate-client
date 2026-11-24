package edu.crabmate

import android.util.Base64

/** 聊天灯箱 / 工作区保存：分块接收 base64（避开 JS 桥 ~1MiB 限制），再交系统另存为。 */
internal class ChatImageShare {
  private val lock = Any()
  private var buf: StringBuilder? = null
  private var filename: String = DEFAULT_NAME

  /**
   * 最近一次调用的失败原因码（`""` = 成功），供 JS 桥把「为什么没存成」带回前端。
   *
   * 原先只返回 `Boolean`，前端只能弹一句「保存失败」，无法区分「文件过大」这类可解释的失败。
   */
  @Volatile
  var lastError: String = ""
    private set

  fun begin(
    originOk: Boolean,
    rawName: String,
    asImage: Boolean = true,
  ): Boolean {
    if (!originOk) {
      lastError = ERR_ORIGIN
      return false
    }
    synchronized(lock) {
      if (buf != null) {
        lastError = ERR_BUSY
        return false
      }
      buf = StringBuilder()
      filename = if (asImage) safeImageFileName(rawName) else safeFileName(rawName)
      lastError = ""
    }
    return true
  }

  fun append(
    originOk: Boolean,
    chunk: String,
  ): Boolean {
    if (!originOk || chunk.isEmpty() || chunk.length > MAX_CHUNK_CHARS) {
      lastError = ERR_CHUNK
      return false
    }
    if (!chunk.all { it.isLetterOrDigit() || it == '+' || it == '/' || it == '=' }) {
      lastError = ERR_CHUNK
      return false
    }
    synchronized(lock) {
      val b = buf
      if (b == null) {
        lastError = ERR_NOT_BEGUN
        return false
      }
      if (b.length + chunk.length > MAX_B64_CHARS) {
        buf = null
        lastError = ERR_TOO_LARGE
        return false
      }
      b.append(chunk)
    }
    lastError = ""
    return true
  }

  fun finish(originOk: Boolean): Pair<String, ByteArray>? {
    if (!originOk) {
      cancel()
      lastError = ERR_ORIGIN
      return null
    }
    val payload =
      synchronized(lock) {
        val b = buf?.toString()
        buf = null
        val name = filename
        Pair(b, name)
      }
    val b64 = payload.first
    if (b64 == null) {
      lastError = ERR_NOT_BEGUN
      return null
    }
    val bytes =
      try {
        Base64.decode(b64, Base64.DEFAULT)
      } catch (_: Exception) {
        lastError = ERR_DECODE
        return null
      }
    if (bytes.size > MAX_DECODED_BYTES) {
      lastError = ERR_TOO_LARGE
      return null
    }
    lastError = ""
    return Pair(payload.second, bytes)
  }

  fun cancel() {
    synchronized(lock) {
      buf = null
    }
    lastError = ""
  }

  companion object {
    const val DEFAULT_NAME: String = "image.png"
    const val DEFAULT_FILE_NAME: String = "download.txt"
    const val MAX_DECODED_BYTES: Int = 16 * 1024 * 1024
    const val MAX_B64_CHARS: Int = MAX_DECODED_BYTES * 2
    const val MAX_CHUNK_CHARS: Int = 240_000

    /** 桥把 `lastError` 原样回传；前端按这组稳定码选提示文案（勿改字面量，见 issue #29）。 */
    const val ERR_ORIGIN: String = "origin"
    const val ERR_BUSY: String = "busy"
    const val ERR_CHUNK: String = "chunk"
    const val ERR_NOT_BEGUN: String = "not_begun"
    const val ERR_TOO_LARGE: String = "too_large"
    const val ERR_DECODE: String = "decode"

    /** `isLetterOrDigit` keeps CJK; spaces become `_`. Empty / dots-only → [fallback]. */
    fun safeFileName(
      raw: String,
      fallback: String = DEFAULT_FILE_NAME,
    ): String {
      val last =
        raw
          .replace('\\', '/')
          .substringAfterLast('/')
          .replace(Regex("\\s+"), "_")
          .filter { it.isLetterOrDigit() || it == '.' || it == '-' || it == '_' }
          .take(80)
          .trim('.')
      if (last.isEmpty()) {
        return fallback
      }
      return last
    }

    fun safeImageFileName(raw: String): String {
      val last = safeFileName(raw, DEFAULT_NAME)
      return if (hasRasterExt(last)) last else "$last.png"
    }

    fun mimeForName(name: String): String {
      val ext = name.substringAfterLast('.', "").lowercase()
      if (ext == "jpg" || ext == "jpeg") {
        return "image/jpeg"
      }
      if (ext == "png") {
        return "image/png"
      }
      if (ext == "webp") {
        return "image/webp"
      }
      if (ext == "gif") {
        return "image/gif"
      }
      return mimeForNonImageExt(ext)
    }

    /**
     * 非图片扩展名 → MIME。
     *
     * `json` / `md` 用注册类型而不是 `text/plain`：另存为的类型会落到 DocumentsProvider /
     * MediaStore，之后「用其它 App 打开」或分享时按它挑应用，`text/plain` 会把 `.json`
     * 送进纯文本编辑器。代价是部分第三方 provider 会按类型过滤掉目标位置（见 runbook §3b）。
     */
    private fun mimeForNonImageExt(ext: String): String {
      if (ext == "pdf") {
        return "application/pdf"
      }
      if (ext == "zip") {
        return "application/zip"
      }
      if (ext == "json") {
        return "application/json"
      }
      if (ext == "md" || ext == "markdown") {
        return "text/markdown"
      }
      val text =
        setOf(
          "txt",
          "rs",
          "toml",
          "css",
          "html",
          "xml",
          "yml",
          "yaml",
          "sh",
          "py",
          "ts",
          "js",
          "kt",
          "kts",
          "c",
          "h",
          "cpp",
          "go",
          "java",
        )
      return if (ext in text) "text/plain" else "application/octet-stream"
    }

    private fun hasRasterExt(name: String): Boolean {
      val ext = name.substringAfterLast('.', missingDelimiterValue = "").lowercase()
      if (ext.isEmpty() || ext.length == name.length) {
        return false
      }
      val stem = name.substringBeforeLast('.')
      return stem.isNotEmpty() && ext in setOf("png", "jpg", "jpeg", "webp", "gif")
    }
  }
}