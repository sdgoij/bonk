package com.github.sdgoij.bonk

/** What the unlocked screen shows. */
data class ArchiveSummary(
    val sellers: Long,
    val drafts: Long,
    val schemaVersion: Long,
)

/** A failure with a message the lock screen can show. */
class ArchiveException(message: String) : Exception(message)

/** True when both files of an archive exist in app storage. */
expect fun archiveExists(): Boolean

/** Creates the archive and returns its summary; throws on failure. */
expect fun createArchive(passphrase: String): ArchiveSummary

/** Opens the archive with a passphrase and returns its summary. */
expect fun openArchive(passphrase: String): ArchiveSummary

/** True when this device holds a key for the archive. */
expect fun deviceReady(): Boolean

/** Opens the archive with the key this device kept, authenticating the user. */
expect fun openArchiveWithDevice(): ArchiveSummary

/** Keeps the archive key behind device authentication. */
expect fun rememberDevice(passphrase: String)

/** Removes the device key. False when there was none. */
expect fun forgetDevice(): Boolean

/** Drops the open archive and zeroizes its key. */
expect fun lockArchive()
