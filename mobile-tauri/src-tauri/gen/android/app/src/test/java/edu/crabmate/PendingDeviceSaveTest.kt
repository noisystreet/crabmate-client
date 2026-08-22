package edu.crabmate

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files

class PendingDeviceSaveTest {
  @Test
  fun tryBeginWriteRejectsSecondUntilAbort() {
    val dir = Files.createTempDirectory("cm-save").toFile()
    try {
      val pending = PendingDeviceSave(dir)
      assertTrue(pending.tryBeginWrite(byteArrayOf(1, 2)))
      assertTrue(File(dir, "cm-create-document.bin").exists())
      assertFalse(pending.tryBeginWrite(byteArrayOf(3)))
      pending.abort()
      assertFalse(File(dir, "cm-create-document.bin").exists())
      assertTrue(pending.tryBeginWrite(byteArrayOf(4)))
    } finally {
      dir.deleteRecursively()
    }
  }
}
