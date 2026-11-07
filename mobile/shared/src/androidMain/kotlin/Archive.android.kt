package com.github.sdgoij.bonk

import android.content.Context
import uniffi.bonkc.AppException
import uniffi.bonkc.Archive
import uniffi.bonkc.archiveExists
import uniffi.bonkc.deviceReady as ffiDeviceReady
import java.io.File

/**
 * Holds the one open [Archive] handle. The activity supplies the archive
 * directory; the host keeps the derived [File], not the Context.
 */
object MobileArchiveHost {
    private var directory: File? = null
    private var archive: Archive? = null

    fun attach(context: Context) {
        directory = File(context.filesDir, "archive")
    }

    fun detach() {
        archive?.close()
        archive = null
        directory = null
    }

    fun exists(): Boolean {
        val (database, keystore) = paths()
        return archiveExists(database, keystore)
    }

    fun create(passphrase: String): ArchiveSummary {
        require(passphrase.isNotEmpty()) { "choose a passphrase" }
        val (database, keystore) = paths()
        archive?.close()
        val opened = Archive.create(database, keystore, passphrase)
        archive = opened
        return opened.summary().toCommon()
    }

    fun open(passphrase: String): ArchiveSummary {
        require(passphrase.isNotEmpty()) { "enter the passphrase" }
        val (database, keystore) = paths()
        archive?.close()
        val opened = Archive.open(database, keystore, passphrase)
        archive = opened
        return opened.summary().toCommon()
    }

    fun lock() {
        archive?.lock()
        archive?.close()
        archive = null
    }

    fun deviceReady(): Boolean {
        val (_, keystore) = paths()
        return ffiDeviceReady(keystore, KeystoreHost.deviceStore())
    }

    fun openWithDevice(): ArchiveSummary {
        val (database, keystore) = paths()
        archive?.close()
        val opened = Archive.openWithDevice(database, keystore, KeystoreHost.deviceStore())
        archive = opened
        return opened.summary().toCommon()
    }

    fun rememberDevice(passphrase: String) {
        val opened = archive ?: error("the archive is not open")
        opened.rememberDevice(passphrase, KeystoreHost.deviceStore())
    }

    fun forgetDevice(): Boolean {
        val opened = archive ?: error("the archive is not open")
        return opened.forgetDevice(KeystoreHost.deviceStore())
    }

    private fun paths(): Pair<String, String> {
        val base = directory ?: error("the archive host is not attached")
        base.mkdirs()
        return File(base, "bonk.db").absolutePath to File(base, "keystore.bin").absolutePath
    }
}

private fun uniffi.bonkc.ArchiveSummary.toCommon() =
    ArchiveSummary(sellers = sellers, drafts = drafts, schemaVersion = schemaVersion)

actual fun archiveExists(): Boolean = MobileArchiveHost.exists()

actual fun createArchive(passphrase: String): ArchiveSummary = try {
    MobileArchiveHost.create(passphrase)
} catch (error: AppException) {
    throw ArchiveException(error.toUserMessage())
}

actual fun openArchive(passphrase: String): ArchiveSummary = withUserError {
    MobileArchiveHost.open(passphrase)
}

actual fun deviceReady(): Boolean = runCatching { MobileArchiveHost.deviceReady() }.getOrDefault(false)

actual fun openArchiveWithDevice(): ArchiveSummary = withUserError {
    MobileArchiveHost.openWithDevice()
}

actual fun rememberDevice(passphrase: String) = withUserError {
    MobileArchiveHost.rememberDevice(passphrase)
}

actual fun forgetDevice(): Boolean = withUserError {
    MobileArchiveHost.forgetDevice()
}

actual fun lockArchive() = MobileArchiveHost.lock()

private inline fun <T> withUserError(block: () -> T): T = try {
    block()
} catch (error: AppException) {
    throw ArchiveException(error.toUserMessage())
}

private fun AppException.toUserMessage(): String = when (this) {
    is AppException.WrongPassphrase -> "Wrong passphrase. Try again."
    is AppException.ArchiveExists -> "An archive already exists on this device."
    is AppException.InvalidKeystore -> "The archive keystore is damaged or from a newer version."
    is AppException.NoRecoveryKey -> "This archive has no recovery key."
    is AppException.InvalidRecoveryKey -> "That recovery key is not valid."
    is AppException.Io -> "Storage error: $detail"
    is AppException.Store -> detail
    is AppException.Artefact -> "Rendering failed: $detail"
    is AppException.AuthFailed -> "Authentication failed: $detail"
    is AppException.Invalidated -> "The device secret was invalidated."
    is AppException.Closed -> "The archive is locked."
}
