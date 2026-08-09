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
 * 连接页 Web API Bearer：Android Keystore AES-GCM + 普通 SharedPreferences。
 *
 * 不用 `androidx.security:security-crypto`（会拉入 Tink，release R8 minify 后
 * 在部分 OEM / HyperOS 上启动期闪退）。密钥不出 Keystore。
 */
internal object SecureBearerStore {
  private const val ANDROID_KEYSTORE = "AndroidKeyStore"
  private const val KEY_ALIAS = "crabmate_connect_bearer_aes"
  private const val PREFS_NAME = "crabmate_secure_connect"
  private const val PREF_CIPHERTEXT = "web_api_bearer_ct"
  private const val TRANSFORMATION = "AES/GCM/NoPadding"
  private const val GCM_TAG_BITS = 128
  private const val IV_BYTES = 12

  fun read(context: Context): String {
    val encoded =
      context
        .getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        .getString(PREF_CIPHERTEXT, null)
        ?: return ""
    return try {
      val blob = Base64.decode(encoded, Base64.NO_WRAP)
      if (blob.size <= IV_BYTES) {
        return ""
      }
      val iv = blob.copyOfRange(0, IV_BYTES)
      val ct = blob.copyOfRange(IV_BYTES, blob.size)
      val cipher = Cipher.getInstance(TRANSFORMATION)
      cipher.init(Cipher.DECRYPT_MODE, secretKey(), GCMParameterSpec(GCM_TAG_BITS, iv))
      String(cipher.doFinal(ct), Charsets.UTF_8).trim()
    } catch (_: Exception) {
      ""
    }
  }

  fun write(
    context: Context,
    bearer: String,
  ): Boolean {
    val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
    val trimmed = bearer.trim()
    if (trimmed.isEmpty()) {
      return prefs.edit().remove(PREF_CIPHERTEXT).commit()
    }
    return try {
      val cipher = Cipher.getInstance(TRANSFORMATION)
      cipher.init(Cipher.ENCRYPT_MODE, secretKey())
      val iv = cipher.iv
      val ct = cipher.doFinal(trimmed.toByteArray(Charsets.UTF_8))
      val packed =
        ByteBuffer
          .allocate(iv.size + ct.size)
          .put(iv)
          .put(ct)
          .array()
      val encoded = Base64.encodeToString(packed, Base64.NO_WRAP)
      prefs.edit().putString(PREF_CIPHERTEXT, encoded).commit()
    } catch (_: Exception) {
      false
    }
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