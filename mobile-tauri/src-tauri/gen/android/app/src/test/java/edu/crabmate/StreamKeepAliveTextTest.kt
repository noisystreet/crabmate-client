package edu.crabmate

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class StreamKeepAliveTextTest {
  @Test
  fun previewJoinsAndCollapsesWhitespace() {
    assertEquals("rm -rf /tmp", StreamKeepAliveText.preview(" rm ", "  -rf   /tmp "))
  }

  @Test
  fun previewTruncatesToMaxChars() {
    val cmd = "a".repeat(50)
    val args = "b".repeat(50)
    val out = StreamKeepAliveText.preview(cmd, args)
    assertEquals(StreamKeepAliveText.PREVIEW_MAX_CHARS + 1, out.length)
    assertTrue(out.endsWith("…"))
  }

  @Test
  fun localeEnglishDetect() {
    assertTrue(StreamKeepAliveText.isEnglish("en"))
    assertTrue(StreamKeepAliveText.isEnglish("en-US"))
    assertFalse(StreamKeepAliveText.isEnglish("zh-Hans"))
    assertFalse(StreamKeepAliveText.isEnglish(""))
  }
}
