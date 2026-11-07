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

/** Drops the open archive and zeroizes its key. */
expect fun lockArchive()
