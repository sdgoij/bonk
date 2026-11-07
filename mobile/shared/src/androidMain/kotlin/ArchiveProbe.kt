package com.github.sdgoij.bonk

import android.content.Context
import uniffi.bonkc.Archive
import java.io.File

/**
 * Creates, locks, and reopens a real encrypted archive through the FFI, using
 * paths in the app's private storage. This is the M11.1 smoke test: it exercises
 * SQLCipher, the keystore, UniFFI, and JNA together on the device.
 */
object ArchiveProbe {
    private var directory: File? = null

    fun attach(context: Context) {
        directory = File(context.filesDir, "archive-probe")
    }

    fun detach() {
        directory = null
    }

    fun run(): String {
        val dir = directory ?: throw IllegalStateException("the archive probe is not attached")
        dir.deleteRecursively()
        dir.mkdirs()
        val database = File(dir, "bonk.db").absolutePath
        val keystore = File(dir, "keystore.bin").absolutePath

        val archive = Archive.create(database, keystore, "passphrase")
        val summary = archive.summary()
        archive.lock()
        val locked = try {
            archive.summary()
            "not-locked"
        } catch (_: Exception) {
            "locked"
        }

        val reopened = Archive.open(database, keystore, "passphrase")
        val reopenedSummary = reopened.summary()
        reopened.close()
        dir.deleteRecursively()

        return "create=ok schema=${summary.schemaVersion} $locked open=ok drafts=${reopenedSummary.drafts}"
    }
}

actual fun runArchiveProbe(): String = ArchiveProbe.run()
