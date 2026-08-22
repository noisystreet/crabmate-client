package edu.crabmate

import android.content.ContentResolver
import android.net.Uri
import java.io.File
import java.util.concurrent.Executors

/**
 * 另存为过程中把字节落到 cache，避免进程被杀后只留下空文件。
 * 拷贝在后台线程，避免 16MiB 写主线程 ANR。
 */
internal class PendingDeviceSave(
  private val cacheDir: File,
) {
  private val lock = Any()
  private var pickerOpen = false
  private val io =
    Executors.newSingleThreadExecutor { r ->
      Thread(r, "cm-device-save").apply { isDaemon = true }
    }

  fun tryBeginWrite(bytes: ByteArray): Boolean {
    if (bytes.size > ChatImageShare.MAX_DECODED_BYTES) {
      return false
    }
    synchronized(lock) {
      if (pickerOpen) {
        return false
      }
      pickerOpen = true
    }
    return try {
      payloadFile().writeBytes(bytes)
      true
    } catch (_: Exception) {
      abort()
      false
    }
  }

  fun abort() {
    synchronized(lock) {
      pickerOpen = false
    }
    payloadFile().delete()
  }

  fun complete(
    uri: Uri?,
    resolver: ContentResolver,
  ) {
    io.execute {
      try {
        copyIfNeeded(uri, resolver)
      } catch (_: Exception) {
        // 取消或写入失败：不回传 JS
      } finally {
        abort()
      }
    }
  }

  private fun copyIfNeeded(
    uri: Uri?,
    resolver: ContentResolver,
  ) {
    if (uri == null) {
      return
    }
    val file = payloadFile()
    if (!file.exists() || file.length() > ChatImageShare.MAX_DECODED_BYTES) {
      return
    }
    file.inputStream().use { input ->
      resolver.openOutputStream(uri)?.use { output ->
        input.copyTo(output)
      }
    }
  }

  private fun payloadFile(): File = File(cacheDir, "cm-create-document.bin")
}