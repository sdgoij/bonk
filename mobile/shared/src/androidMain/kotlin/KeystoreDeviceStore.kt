package com.github.sdgoij.bonk

import android.os.Build
import android.os.Handler
import android.os.Looper
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.UserNotAuthenticatedException
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import java.io.File
import java.io.IOException
import java.security.GeneralSecurityException
import java.security.KeyStore
import java.security.UnrecoverableKeyException
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.bonkc.AppException
import uniffi.bonkc.DeviceSecretStore

/**
 * S3: an Android Keystore implementation of the UniFFI `DeviceSecretStore`
 * callback.
 *
 * The Keystore key is AES-256-GCM and requires biometric authentication for
 * every use. It is biometric-only on purpose: a key that also allows the device
 * credential is not invalidated when a new fingerprint or face is enrolled. The
 * passphrase is the fallback when biometrics are unavailable or have changed.
 * The wrapped secret is `iv || ciphertext` in `noBackupFilesDir`, which the OS
 * does not back up.
 */
class KeystoreDeviceStore(private val activity: FragmentActivity) : DeviceSecretStore {
    private val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private val mainHandler = Handler(Looper.getMainLooper())
    private val mainExecutor = ContextCompat.getMainExecutor(activity)

    override fun put(account: String, secret: ByteArray) {
        if (secret.size != SECRET_LENGTH) {
            throw AppException.Store("expected a $SECRET_LENGTH-byte secret")
        }
        if (!biometricAvailable()) {
            throw AppException.Store("this device has no biometric authentication to protect the key")
        }
        ensureKey()
        val cipher = newCipher()
        try {
            cipher.init(Cipher.ENCRYPT_MODE, key())
        } catch (error: UserNotAuthenticatedException) {
            // Expected: the prompt below authorizes this use of the key.
        } catch (error: KeyPermanentlyInvalidatedException) {
            deleteKey()
            throw AppException.Invalidated()
        } catch (error: UnrecoverableKeyException) {
            deleteKey()
            throw AppException.Invalidated()
        } catch (error: GeneralSecurityException) {
            throw AppException.Store(error.message ?: "the Keystore key could not be used")
        }

        val data = withAuthentication(cipher) { authorized ->
            authorized.iv + authorized.doFinal(secret)
        }
        val file = fileFor(account)
        try {
            file.parentFile?.mkdirs()
            file.writeBytes(data)
        } catch (error: IOException) {
            throw AppException.Store(error.message ?: "the wrapped secret could not be written")
        }
    }

    override fun get(account: String): ByteArray? {
        val file = fileFor(account)
        if (!file.exists()) return null
        if (!biometricAvailable()) {
            file.delete()
            throw AppException.Invalidated()
        }

        val bytes = try {
            file.readBytes()
        } catch (error: IOException) {
            throw AppException.Store(error.message ?: "the wrapped secret could not be read")
        }
        if (bytes.size <= IV_LENGTH) {
            throw AppException.Store("the stored entry is truncated")
        }
        val iv = bytes.copyOfRange(0, IV_LENGTH)
        val ciphertext = bytes.copyOfRange(IV_LENGTH, bytes.size)

        val cipher = newCipher()
        try {
            cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(TAG_BITS, iv))
        } catch (error: UserNotAuthenticatedException) {
            // Expected: the prompt below authorizes this use of the key.
        } catch (error: KeyPermanentlyInvalidatedException) {
            file.delete()
            throw AppException.Invalidated()
        } catch (error: UnrecoverableKeyException) {
            file.delete()
            throw AppException.Invalidated()
        } catch (error: AppException.Invalidated) {
            file.delete()
            throw error
        } catch (error: GeneralSecurityException) {
            throw AppException.Store(error.message ?: "the stored entry could not be opened")
        }

        return withAuthentication(cipher) { authorized -> authorized.doFinal(ciphertext) }
    }

    /**
     * Whether an entry exists and this device can still authenticate it.
     *
     * This never reads the entry: reading unwraps the key and shows the prompt.
     */
    override fun has(account: String): Boolean =
        fileFor(account).exists() && biometricAvailable()

    /** True when the device has an enrolled strong biometric. */
    fun biometricAvailable(): Boolean =
        BiometricManager.from(activity)
            .canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) ==
            BiometricManager.BIOMETRIC_SUCCESS

    override fun delete(account: String): Boolean {
        val file = fileFor(account)
        val existed = file.exists()
        file.delete()
        return existed
    }

    /** Simulates a key invalidation without changing the device credential. */
    fun invalidateForTest() {
        deleteKey()
    }

    private fun ensureKey() {
        try {
            if (keyStore.containsAlias(KEY_ALIAS)) return

            val builder = KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setUserAuthenticationRequired(true)
                .setInvalidatedByBiometricEnrollment(true)

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                builder.setUserAuthenticationParameters(
                    0,
                    KeyProperties.AUTH_BIOMETRIC_STRONG,
                )
            } else {
                @Suppress("DEPRECATION")
                builder.setUserAuthenticationValidityDurationSeconds(-1)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                builder.setUnlockedDeviceRequired(true)
            }

            val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
            generator.init(builder.build())
            generator.generateKey()
            // A fresh key cannot open entries wrapped under the previous one, so
            // remove them rather than leave undecryptable files behind.
            File(activity.noBackupFilesDir, "device").deleteRecursively()
        } catch (error: GeneralSecurityException) {
            throw AppException.Store(error.message ?: "the Keystore key could not be created")
        } catch (error: IllegalArgumentException) {
            throw AppException.Store(error.message ?: "the Keystore key parameters are invalid")
        }
    }

    private fun key(): SecretKey =
        try {
            keyStore.getKey(KEY_ALIAS, null) as? SecretKey
                ?: throw AppException.Invalidated()
        } catch (error: UnrecoverableKeyException) {
            throw AppException.Invalidated()
        } catch (error: GeneralSecurityException) {
            throw AppException.Store(error.message ?: "the Keystore key is unavailable")
        }

    private fun newCipher(): Cipher =
        try {
            Cipher.getInstance("AES/GCM/NoPadding")
        } catch (error: GeneralSecurityException) {
            throw AppException.Store(error.message ?: "AES/GCM is unavailable")
        }

    private fun deleteKey() {
        try {
            if (keyStore.containsAlias(KEY_ALIAS)) keyStore.deleteEntry(KEY_ALIAS)
        } catch (error: GeneralSecurityException) {
            throw AppException.Store(error.message ?: "the Keystore key could not be deleted")
        }
    }

    private fun <T> withAuthentication(cipher: Cipher, block: (Cipher) -> T): T {
        val latch = CountDownLatch(1)
        val outcome = AtomicReference<Result<T>>()

        mainHandler.post {
            val prompt = BiometricPrompt(
                activity,
                mainExecutor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        try {
                            val authorized = result.cryptoObject?.cipher
                                ?: throw AppException.AuthFailed("the prompt returned no cipher")
                            outcome.set(Result.success(block(authorized)))
                        } catch (error: Throwable) {
                            outcome.set(Result.failure(error))
                        } finally {
                            latch.countDown()
                        }
                    }

                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        outcome.set(Result.failure(AppException.AuthFailed(errString.toString())))
                        latch.countDown()
                    }

                    override fun onAuthenticationFailed() {
                        // The prompt stays open for a retry.
                    }
                },
            )
            val info = BiometricPrompt.PromptInfo.Builder()
                .setTitle("Unlock Bonk")
                .setSubtitle("Use your fingerprint or face")
                .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
                .setNegativeButtonText("Use passphrase")
                .build()
            prompt.authenticate(info, BiometricPrompt.CryptoObject(cipher))
        }

        if (!latch.await(AUTH_TIMEOUT_SECONDS, TimeUnit.SECONDS)) {
            throw AppException.AuthFailed("timed out waiting for authentication")
        }
        return outcome.get().getOrThrow()
    }

    private fun fileFor(account: String): File {
        val safe = account.replace(Regex("[^A-Za-z0-9._-]"), "_")
        return File(File(activity.noBackupFilesDir, "device"), "$safe.bin")
    }

    private companion object {
        const val KEY_ALIAS = "bonk.device.biometric"
        const val SECRET_LENGTH = 32
        const val IV_LENGTH = 12
        const val TAG_BITS = 128
        const val AUTH_TIMEOUT_SECONDS = 60L
    }
}
