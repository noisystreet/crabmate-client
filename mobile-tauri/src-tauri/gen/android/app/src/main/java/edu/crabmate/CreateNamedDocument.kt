package edu.crabmate

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.activity.result.contract.ActivityResultContract

/** 系统另存为（SAF）。分享页在小米等机型上常无「文件」目标。 */
internal class CreateNamedDocument : ActivityResultContract<CreateNamedDocument.Request, Uri?>() {
  data class Request(
    val mime: String,
    val displayName: String,
  )

  override fun createIntent(
    context: Context,
    input: Request,
  ): Intent {
    val mime = input.mime.ifBlank { "application/octet-stream" }
    return Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
      addCategory(Intent.CATEGORY_OPENABLE)
      type = mime
      putExtra(Intent.EXTRA_TITLE, input.displayName)
    }
  }

  override fun parseResult(
    resultCode: Int,
    intent: Intent?,
  ): Uri? {
    if (resultCode != Activity.RESULT_OK) {
      return null
    }
    return intent?.data
  }
}