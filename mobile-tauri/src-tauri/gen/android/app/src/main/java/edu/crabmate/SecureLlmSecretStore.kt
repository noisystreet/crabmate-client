package edu.crabmate

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.nio.ByteBuffer
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * 模型 API 密钥：Android Keystore AES-GCM + SharedPreferences。
 * 槽位：`client_llm` / `executor_llm` / `saved_models` / `github`（与桌面钥匙串账户对应）。
 */
internal object SecureLlmSecretStore {
  private const val ANDROID_KEYSTORE = "AndroidKeyStore"
  private const val KEY_ALIAS = "crabmate_llm_secrets_aes"
  private const val PREFS_NAME = "crabmate_secure_llm"
  private const val TRANSFORMATION = "AES/GCM/NoPadding"
  private const val GCM_TAG_BITS = 128
  private const val IV_BYTES = 12

  private val allowedSlots = setOf("client_llm", "executor_llm", "saved_models", "github")

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
    return try {
      decrypt(encoded)
    } catch (_: Exception) {
      ""
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
    return try {
      prefs.edit().putString(prefKey, encrypt(trimmed)).commit()
    } catch (_: Exception) {
      false
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
    val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
    (ks.getEntry(KEY_ALIAS, null) as? KeyStore.SecretKeyEntry)?.secretKey?.let {
      return it
    }
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