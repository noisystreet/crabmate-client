package edu.crabmate

import org.junit.Assert.assertEquals
import org.junit.Test

class ChatImageShareTest {
  @Test
  fun safeFileNameStripsPathAndKeepsExt() {
    assertEquals("a.png", ChatImageShare.safeFileName("/tmp/../a.png"))
    assertEquals("shot.webp", ChatImageShare.safeFileName("shot.webp"))
    assertEquals("image.png", ChatImageShare.safeFileName("///"))
    assertEquals("foo.png", ChatImageShare.safeFileName("foo"))
  }

  @Test
  fun mimeForNameMapsRasterExt() {
    assertEquals("image/jpeg", ChatImageShare.mimeForName("x.jpg"))
    assertEquals("image/png", ChatImageShare.mimeForName("x.png"))
    assertEquals("image/webp", ChatImageShare.mimeForName("x.webp"))
  }
}
