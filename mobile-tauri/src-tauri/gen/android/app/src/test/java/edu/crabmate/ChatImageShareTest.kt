package edu.crabmate

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatImageShareTest {
  @Test
  fun safeFileNameStripsPathAndKeepsExt() {
    assertEquals("a.png", ChatImageShare.safeImageFileName("/tmp/../a.png"))
    assertEquals("shot.webp", ChatImageShare.safeImageFileName("shot.webp"))
    assertEquals("image.png", ChatImageShare.safeImageFileName("///"))
    assertEquals("foo.png", ChatImageShare.safeImageFileName("foo"))
  }

  @Test
  fun safeFileNameKeepsNonImageExtAndCjk() {
    assertEquals("lib.rs", ChatImageShare.safeFileName("src/lib.rs"))
    assertEquals("notes.md", ChatImageShare.safeFileName("notes.md"))
    assertEquals("说明.md", ChatImageShare.safeFileName("docs/说明.md"))
    assertEquals("my_notes.md", ChatImageShare.safeFileName("my notes.md"))
    assertEquals("download.txt", ChatImageShare.safeFileName("///"))
  }

  @Test
  fun beginRejectsWhileBufferInUse() {
    val share = ChatImageShare()
    assertTrue(share.begin(true, "a.md", asImage = false))
    assertFalse(share.begin(true, "b.md", asImage = false))
    share.cancel()
    assertTrue(share.begin(true, "b.md", asImage = false))
  }

  @Test
  fun mimeForNameMapsRasterExt() {
    assertEquals("image/jpeg", ChatImageShare.mimeForName("x.jpg"))
    assertEquals("image/png", ChatImageShare.mimeForName("x.png"))
    assertEquals("image/webp", ChatImageShare.mimeForName("x.webp"))
  }

  @Test
  fun mimeForNameMapsTextAndBinary() {
    assertEquals("text/plain", ChatImageShare.mimeForName("lib.rs"))
    assertEquals("application/pdf", ChatImageShare.mimeForName("说明.pdf"))
    assertEquals("application/octet-stream", ChatImageShare.mimeForName("blob.bin"))
  }
}
