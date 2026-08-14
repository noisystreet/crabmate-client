package edu.crabmate

/** 审批通知正文截断（不把完整 args / 密钥型参数塞进系统通知栏）。 */
internal object StreamKeepAliveText {
  const val PREVIEW_MAX_CHARS: Int = 80

  fun isEnglish(locale: String): Boolean = locale.trim().lowercase().startsWith("en")

  fun preview(
    command: String,
    args: String,
  ): String {
    val joined = "${command.trim()} ${args.trim()}".trim().replace(WHITESPACE, " ")
    if (joined.length <= PREVIEW_MAX_CHARS) {
      return joined
    }
    return joined.take(PREVIEW_MAX_CHARS) + "…"
  }

  private val WHITESPACE = Regex("\\s+")
}