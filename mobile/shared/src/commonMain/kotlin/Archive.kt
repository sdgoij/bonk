package com.github.sdgoij.bonk

/** What the unlocked screen shows. */
data class ArchiveSummary(
    val sellers: Long,
    val drafts: Long,
    val schemaVersion: Long,
)

/** A failure with a message the lock screen can show. */
class ArchiveException(message: String) : Exception(message)

/** The outcome of a recovery-key reset. */
data class RecoveryResult(
    val summary: ArchiveSummary,
    val replacementKey: String,
)

/** True when both files of an archive exist in app storage. */
expect fun archiveExists(): Boolean

/** Creates the archive and returns its summary; throws on failure. */
expect fun createArchive(passphrase: String): ArchiveSummary

/** Opens the archive with a passphrase and returns its summary. */
expect fun openArchive(passphrase: String): ArchiveSummary

/** True when this device holds a key for the archive, without prompting. */
expect fun deviceReady(): Boolean

/** True when this device can offer biometric device unlock. */
expect fun deviceUnlockAvailable(): Boolean

/** Opens the archive with the key this device kept, authenticating the user. */
expect fun openArchiveWithDevice(): ArchiveSummary

/** Keeps the archive key behind device authentication. */
expect fun rememberDevice(passphrase: String)

/** Removes the device key. False when there was none. */
expect fun forgetDevice(): Boolean

/** Replaces a forgotten passphrase with the recovery key. */
expect fun resetPassphrase(recoveryKey: String, newPassphrase: String): RecoveryResult

/** Adds or replaces the recovery key, returning the secret to show once. */
expect fun addRecoveryKey(passphrase: String): String

/** Drops the open archive and zeroizes its key. */
expect fun lockArchive()

/** Records that a device key was revoked by an enrolment change. */
expect fun recordDeviceRevocation()

/** Remembers that the device key was invalidated, until the next unlock. */
expect fun markDeviceRevoked()

/** True once after a revocation was marked. */
expect fun takeDeviceRevoked(): Boolean

/** Scans the archive files for a plaintext SQLite header. */
expect fun runCanaryScan(): String
