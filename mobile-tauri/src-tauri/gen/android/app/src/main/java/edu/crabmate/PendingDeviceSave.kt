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

  /** 最近一次 [tryBeginWrite] 的失败原因码（`""` = 成功），由 JS 桥原样回传前端。 */
  @Volatile
  var lastError: String = ""
    private set

  fun tryBeginWrite(bytes: ByteArray): Boolean {
    if (bytes.size > ChatImageShare.MAX_DECODED_BYTES) {
      lastError = ChatImageShare.ERR_TOO_LARGE
      return false
    }
    synchronized(lock) {
      if (pickerOpen) {
        lastError = ChatImageShare.ERR_BUSY
        return false
      }
      pickerOpen = true
    }
    return try {
      payloadFile().writeBytes(bytes)
      lastError = ""
      true
    } catch (_: Exception) {
      lastError = ERR_WRITE
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

  /**
   * picker 返回后落盘，并把**最终**结果回传。
   *
   * 原先取消与写盘失败都被吞掉，前端 `finishDeviceFileSave()` 只要对话框拉起来就算成功，
   * 于是用户点了取消或存储写失败时界面一声不响（issue #29 验收「错误在应用内可见」）。
   * `onDone` 在 io 线程调用，取值只会是 [SAVED] / [CANCELLED] / [FAILED]。
   */
  fun complete(
    uri: Uri?,
    resolver: ContentResolver,
    onDone: (String) -> Unit,
  ) {
    io.execute {
      val status =
        try {
          if (uri == null) {
            CANCELLED
          } else {
            copyPayload(uri, resolver)
            SAVED
          }
        } catch (_: Exception) {
          FAILED
        } finally {
          abort()
        }
      onDone(status)
    }
  }

  private fun copyPayload(
    uri: Uri,
    resolver: ContentResolver,
  ) {
    val file = payloadFile()
    if (!file.exists() || file.length() > ChatImageShare.MAX_DECODED_BYTES) {
      throw IllegalStateException("payload missing or oversized")
    }
    file.inputStream().use { input ->
      val output = resolver.openOutputStream(uri) ?: throw IllegalStateException("no output stream")
      output.use { input.copyTo(it) }
    }
  }

  private fun payloadFile(): File = File(cacheDir, "cm-create-document.bin")

  companion object {
    /** 与前端 `__cmDeviceSaveResult` 约定的三个终态；勿改字面量（见 issue #29）。 */
    const val SAVED: String = "saved"
    const val CANCELLED: String = "cancelled"
    const val FAILED: String = "failed"
    const val ERR_WRITE: String = "write"
  }
}