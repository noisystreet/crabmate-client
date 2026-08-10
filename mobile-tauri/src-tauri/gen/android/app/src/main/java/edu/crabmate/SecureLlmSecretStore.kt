package edu.crabmate

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import android.util.Log
import java.nio.ByteBuffer
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * 模型 API 密钥：Android Keystore AES-GCM + SharedPreferences。
 * 槽位：`client_llm` / `executor_llm` / `saved_models` / `github`（与桌面钥匙串账户对应）。
 *
 * 全部入口同步，避免 WebView JS 桥并发 `generateKey` 竞态（Key already exists → 写入失败）。
 */
internal object SecureLlmSecretStore {
  private const val ANDROID_KEYSTORE = "AndroidKeyStore"
  private const val KEY_ALIAS = "crabmate_llm_secrets_aes"
  private const val PREFS_NAME = "crabmate_secure_llm"
  private const val TRANSFORMATION = "AES/GCM/NoPadding"
  private const val GCM_TAG_BITS = 128
  private const val IV_BYTES = 12
  private const val TAG = "SecureLlmSecretStore"
  private const val WRITE_ATTEMPTS = 3

  private val allowedSlots = setOf("client_llm", "executor_llm", "saved_models", "github")
  private val lock = Any()

  fun read(
    context: Context,
    slot: String,
  ): String {
    val prefKey = prefKey(slot) ?: return ""
    val encoded =
      context
        .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        .getString(prefKey, null)
        ?: return ""
    return synchronized(lock) {
      try {
        decrypt(encoded)
      } catch (e: Exception) {
        Log.w(TAG, "decrypt failed for slot=$slot: ${e.javaClass.simpleName}")
        ""
      }
    }
  }

  fun write(
    context: Context,
    slot: String,
    value: String,
  ): Boolean {
    val prefKey = prefKey(slot) ?: return false
    val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    val trimmed = value.trim()
    if (trimmed.isEmpty()) {
      return prefs.edit().remove(prefKey).commit()
    }
    synchronized(lock) {
      var lastError: Exception? = null
      repeat(WRITE_ATTEMPTS) {
        try {
          val encoded = encrypt(trimmed)
          if (prefs.edit().putString(prefKey, encoded).commit()) {
            return true
          }
        } catch (e: Exception) {
          lastError = e
          Log.w(TAG, "encrypt/write attempt failed: ${e.javaClass.simpleName}: ${e.message}")
        }
      }
      if (lastError != null) {
        Log.e(TAG, "write failed for slot=$slot", lastError)
      }
      return false
    }
  }

  private fun prefKey(slot: String): String? {
    val s = slot.trim()
    if (!allowedSlots.contains(s)) {
      return null
    }
    return "llm_$s"
  }

  private fun encrypt(plain: String): String {
    val cipher = Cipher.getInstance(TRANSFORMATION)
    cipher.init(Cipher.ENCRYPT_MODE, secretKey())
    val iv = cipher.iv
    val ct = cipher.doFinal(plain.toByteArray(Charsets.UTF_8))
    val packed =
      ByteBuffer
        .allocate(iv.size + ct.size)
        .put(iv)
        .put(ct)
        .array()
    return Base64.encodeToString(packed, Base64.NO_WRAP)
  }

  private fun decrypt(encoded: String): String {
    val blob = Base64.decode(encoded, Base64.NO_WRAP)
    if (blob.size <= IV_BYTES) {
      return ""
    }
    val iv = blob.copyOfRange(0, IV_BYTES)
    val ct = blob.copyOfRange(IV_BYTES, blob.size)
    val cipher = Cipher.getInstance(TRANSFORMATION)
    cipher.init(Cipher.DECRYPT_MODE, secretKey(), GCMParameterSpec(GCM_TAG_BITS, iv))
    return String(cipher.doFinal(ct), Charsets.UTF_8).trim()
  }

  private fun secretKey(): SecretKey {
    val existing = loadExistingKey()
    if (existing != null) {
      return existing
    }
    return try {
      generateNewKey()
    } catch (e: Exception) {
      // 并发首写：另一线程可能已创建 alias。
      loadExistingKey() ?: throw e
    }
  }

  private fun loadExistingKey(): SecretKey? {
    val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
    return (ks.getEntry(KEY_ALIAS, null) as? KeyStore.SecretKeyEntry)?.secretKey
  }

  private fun generateNewKey(): SecretKey {
    val keyGenerator =
      KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
    keyGenerator.init(
      KeyGenParameterSpec
        .Builder(
          KEY_ALIAS,
          KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        ).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        .build(),
    )
    return keyGenerator.generateKey()
  }
}