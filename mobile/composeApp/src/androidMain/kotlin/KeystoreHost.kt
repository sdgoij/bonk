package com.github.sdgoij.bonk

import androidx.fragment.app.FragmentActivity
import uniffi.bonkc.AppException
import uniffi.bonkc.forgetDeviceSecret
import uniffi.bonkc.readDeviceSecret
import uniffi.bonkc.writeDeviceSecret

private const val ACCOUNT = "s3-account"

/**
 * Holds the Activity-backed store for the S3 probe. The store survives while
 * the Activity is alive; the probe runs on an IO thread and blocks there while
 * the biometric prompt is shown on the main thread.
 */
object KeystoreHost {
    private var store: KeystoreDeviceStore? = null

    fun attach(activity: FragmentActivity) {
        store = KeystoreDeviceStore(activity)
    }

    fun detach() {
        store = null
    }

    fun runS3Test(): String {
        val store = store ?: throw AppException.Store("the Keystore store is not attached")
        val secret = ByteArray(32) { 7 }

        writeDeviceSecret(store, ACCOUNT, secret)
        val read = readDeviceSecret(store, ACCOUNT)
        if (read?.contentEquals(secret) != true) {
            throw AppException.Store("the secret did not round-trip")
        }

        // Simulate a key invalidation: the file stays, the key goes away.
        store.invalidateForTest()
        val invalidated = try {
            readDeviceSecret(store, ACCOUNT)
            "not-detected"
        } catch (error: AppException.Invalidated) {
            "detected"
        }

        val deleted = forgetDeviceSecret(store, ACCOUNT)
        val after = readDeviceSecret(store, ACCOUNT)
        return "put=ok get=ok invalidated=$invalidated delete=$deleted " +
            "after=${if (after == null) "none" else "present"}"
    }
}

actual fun runKeystoreTest(): String = KeystoreHost.runS3Test()
