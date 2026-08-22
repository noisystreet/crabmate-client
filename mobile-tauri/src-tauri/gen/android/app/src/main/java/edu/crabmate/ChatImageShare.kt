package edu.crabmate

import android.util.Base64

/** 聊天灯箱 / 工作区保存：分块接收 base64（避开 JS 桥 ~1MiB 限制），再交系统另存为。 */
internal class ChatImageShare {
  private val lock = Any()
  private var buf: StringBuilder? = null
  private var filename: String = DEFAULT_NAME

  fun begin(
    originOk: Boolean,
    rawName: String,
    asImage: Boolean = true,
  ): Boolean {
    if (!originOk) {
      return false
    }
    synchronized(lock) {
      if (buf != null) {
        return false
      }
      buf = StringBuilder()
      filename = if (asImage) safeImageFileName(rawName) else safeFileName(rawName)
    }
    return true
  }

  fun append(
    originOk: Boolean,
    chunk: String,
  ): Boolean {
    if (!originOk || chunk.isEmpty()) {
      return false
    }
    if (chunk.length > MAX_CHUNK_CHARS) {
      return false
    }
    if (!chunk.all { it.isLetterOrDigit() || it == '+' || it == '/' || it == '=' }) {
      return false
    }
    synchronized(lock) {
      val b = buf ?: return false
      if (b.length + chunk.length > MAX_B64_CHARS) {
        buf = null
        return false
      }
      b.append(chunk)
    }
    return true
  }

  fun finish(originOk: Boolean): Pair<String, ByteArray>? {
    if (!originOk) {
      cancel()
      return null
    }
    val payload =
      synchronized(lock) {
        val b = buf?.toString()
        buf = null
        val name = filename
        Pair(b, name)
      }
    val b64 = payload.first ?: return null
    val bytes =
      try {
        Base64.decode(b64, Base64.DEFAULT)
      } catch (_: Exception) {
        return null
      }
    if (bytes.size > MAX_DECODED_BYTES) {
      return null
    }
    return Pair(payload.second, bytes)
  }

  fun cancel() {
    synchronized(lock) {
      buf = null
    }
  }

  companion object {
    const val DEFAULT_NAME: String = "image.png"
    const val DEFAULT_FILE_NAME: String = "download.txt"
    const val MAX_DECODED_BYTES: Int = 16 * 1024 * 1024
    const val MAX_B64_CHARS: Int = MAX_DECODED_BYTES * 2
    const val MAX_CHUNK_CHARS: Int = 240_000

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

    private fun mimeForNonImageExt(ext: String): String {
      if (ext == "pdf") {
        return "application/pdf"
      }
      if (ext == "zip") {
        return "application/zip"
      }
      val text =
        setOf(
          "txt",
          "md",
          "rs",
          "toml",
          "json",
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