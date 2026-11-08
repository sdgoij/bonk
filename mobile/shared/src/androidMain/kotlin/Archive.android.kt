package com.github.sdgoij.bonk

import android.content.Context
import android.os.StatFs
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
    private var cache: File? = null
    private var staging: File? = null
    private var archive: Archive? = null

    fun attach(context: Context) {
        directory = File(context.filesDir, "archive")
        // Where a verified copy waits for the user to choose a destination.
        cache = File(context.cacheDir, "export")
        // Where the two picked files wait for a restore.
        staging = File(context.filesDir, "restore")
    }

    fun detach() {
        archive?.close()
        archive = null
        directory = null
        cache = null
        staging = null
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

    fun recordDeviceRevocation() {
        val opened = archive ?: error("the archive is not open")
        opened.recordDeviceRevocation()
    }

    fun markDeviceRevoked() {
        val base = directory ?: return
        File(base, "device-revoked").writeText("1")
    }

    fun takeDeviceRevoked(): Boolean {
        val base = directory ?: return false
        val flag = File(base, "device-revoked")
        if (!flag.exists()) return false
        flag.delete()
        return true
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

    fun addRecoveryKey(passphrase: String): String {
        val opened = archive ?: error("the archive is not open")
        return opened.addRecoveryKey(passphrase)
    }

    fun resetPassphrase(recoveryKey: String, newPassphrase: String): RecoveryResult {
        require(recoveryKey.isNotBlank()) { "enter the recovery key" }
        require(newPassphrase.isNotEmpty()) { "choose a new passphrase" }
        val (database, keystore) = paths()
        archive?.close()
        val opened = Archive.resetPassphrase(database, keystore, recoveryKey, newPassphrase)
        archive = opened
        val replacement = opened.takeReplacementKey() ?: error("the replacement key is missing")
        return RecoveryResult(opened.summary().toCommon(), replacement)
    }

    fun canaryScan(): String {
        val (database, keystore) = paths()
        val magic = "SQLite format 3".toByteArray()
        val files = listOf(database, "$database-wal", keystore)
        val checked = files.mapNotNull { path ->
            val file = File(path)
            if (!file.exists()) return@mapNotNull null
            val bytes = file.readBytes()
            val header = if (bytes.containsSequence(magic)) "PLAINTEXT HEADER" else "clean"
            "${file.name}: $header (${bytes.size} bytes)"
        }
        return if (checked.isEmpty()) "no archive files found" else checked.joinToString(", ")
    }

    fun listCustomers(): List<Customer> {
        val opened = archive ?: error("the archive is not open")
        return opened.listCustomers().map { it.toCommon() }
    }

    fun saveCustomer(customer: Customer): Customer {
        val opened = archive ?: error("the archive is not open")
        return opened.saveCustomer(customer.toFfi()).toCommon()
    }

    fun listDrafts(): List<Draft> {
        val opened = archive ?: error("the archive is not open")
        return opened.listDrafts().map { it.toCommon() }
    }

    fun loadDraft(id: String): Draft {
        val opened = archive ?: error("the archive is not open")
        return opened.loadDraft(id).toCommon()
    }

    fun saveDraft(draft: Draft): Draft {
        val opened = archive ?: error("the archive is not open")
        return opened.saveDraft(draft.toFfi()).toCommon()
    }

    fun validateDraft(draft: Draft): List<Finding> {
        val opened = archive ?: error("the archive is not open")
        return opened.validateDraft(draft.toFfi()).map { it.toCommon() }
    }

    fun draftTotals(draft: Draft): DraftTotals {
        val opened = archive ?: error("the archive is not open")
        return opened.draftTotals(draft.toFfi()).toCommon()
    }

    fun newDraft(customerId: String): Draft {
        val opened = archive ?: error("the archive is not open")
        return opened.newDraft(customerId).toCommon()
    }

    fun issueDraft(draft: Draft): IssueResult {
        val opened = archive ?: error("the archive is not open")
        return opened.issueDraft(draft.toFfi()).toCommon()
    }

    fun listDocuments(query: String, status: String?, direction: String): List<DocumentHit> {
        val opened = archive ?: error("the archive is not open")
        return opened.documents(query.ifBlank { null }, status, direction).map { it.toCommon() }
    }

    fun matchSupplier(documentId: String, customerId: String) {
        val opened = archive ?: error("the archive is not open")
        opened.matchSupplier(documentId, customerId)
    }

    fun advanceDocument(documentId: String, target: String): String {
        val opened = archive ?: error("the archive is not open")
        return opened.advanceStatus(documentId, target)
    }

    fun loadDocument(id: String): DocumentDetail {
        val opened = archive ?: error("the archive is not open")
        return opened.document(id).toCommon()
    }

    fun documentArtefact(id: String, kind: String): ByteArray {
        val opened = archive ?: error("the archive is not open")
        return opened.documentArtefact(id, kind)
    }

    fun importDocument(name: String, bytes: ByteArray): ImportReport {
        val opened = archive ?: error("the archive is not open")
        return opened.importDocument(name.ifBlank { null }, bytes).toCommon()
    }

    fun writeBackup(): BackupExport {
        val opened = archive ?: error("the archive is not open")
        val target = cache ?: error("the archive host is not attached")
        target.mkdirs()
        return opened.writeBackup(target.absolutePath).toCommon()
    }

    fun recoveryKey(): ByteArray {
        val opened = archive ?: error("the archive is not open")
        return opened.recoveryKey()
    }

    fun recoveryNote(): String {
        val opened = archive ?: error("the archive is not open")
        return opened.recoveryNote()
    }

    fun freeSpaceBytes(): Long {
        val target = cache ?: error("the archive host is not attached")
        target.mkdirs()
        return StatFs(target.absolutePath).availableBytes
    }

    fun databaseBytes(): Long {
        val (database, _) = paths()
        return File(database).length()
    }

    fun stagingPaths(): Pair<String, String> {
        val base = staging ?: error("the archive host is not attached")
        base.mkdirs()
        return File(base, "bonk.db").absolutePath to File(base, "keystore.bin").absolutePath
    }

    /**
     * Replaces the archive with the staged pair and opens it again.
     *
     * The open handle is closed first because the files move underneath it. A
     * failure leaves no archive open: the swap only happens after the staged
     * pair checks out, so what is on disk is still the archive that was here, but
     * the app has to unlock it again.
     */
    fun restore(passphrase: String): ArchiveSummary {
        require(passphrase.isNotEmpty()) { "enter the passphrase" }
        val (database, keystore) = paths()
        val (stagedDatabase, stagedKeystore) = stagingPaths()
        archive?.close()
        archive = null
        uniffi.bonkc.restoreArchive(database, keystore, stagedDatabase, stagedKeystore, passphrase)
        val opened = Archive.open(database, keystore, passphrase)
        archive = opened
        return opened.summary().toCommon()
    }

    fun loadSeller(): Seller {
        val opened = archive ?: error("the archive is not open")
        return opened.seller().toCommon()
    }

    fun saveSeller(seller: Seller): Seller {
        val opened = archive ?: error("the archive is not open")
        return opened.saveSeller(seller.toFfi()).toCommon()
    }

    fun customersCsv(): String {
        val opened = archive ?: error("the archive is not open")
        return opened.customersCsv()
    }

    fun importCustomersCsv(text: String): CustomerImportReport {
        val opened = archive ?: error("the archive is not open")
        return opened.importCustomersCsv(text).toCommon()
    }

    fun suppressFinding(draftId: String, code: String, field: String, reason: String) {
        val opened = archive ?: error("the archive is not open")
        opened.suppressFinding(draftId, code, field, reason)
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

private fun uniffi.bonkc.CustomerRecord.toCommon() = Customer(
    id = id,
    name = name,
    street = street.orEmpty(),
    postal = postal.orEmpty(),
    city = city,
    country = country,
    vat = vat.orEmpty(),
    kvk = kvk.orEmpty(),
    peppol = peppol.orEmpty(),
    contact = contact.orEmpty(),
    email = email.orEmpty(),
    phone = phone.orEmpty(),
)

private fun Customer.toFfi() = uniffi.bonkc.CustomerRecord(
    id = id,
    name = name,
    street = street.ifBlank { null },
    postal = postal.ifBlank { null },
    city = city,
    country = country,
    vat = vat.ifBlank { null },
    kvk = kvk.ifBlank { null },
    peppol = peppol.ifBlank { null },
    contact = contact.ifBlank { null },
    email = email.ifBlank { null },
    phone = phone.ifBlank { null },
)

private fun uniffi.bonkc.DraftRecord.toCommon() = Draft(
    id = id,
    sellerId = sellerId,
    customerId = customerId,
    docType = docType,
    currency = currency,
    issueDate = issueDate,
    dueDate = dueDate.orEmpty(),
    deliveryDate = deliveryDate.orEmpty(),
    buyerReference = buyerReference.orEmpty(),
    orderReference = orderReference.orEmpty(),
    note = note.orEmpty(),
    template = template.orEmpty(),
    precedingInvoice = precedingInvoice?.let { PrecedingInvoice(it.number, it.issueDate) },
    lines = lines.map { it.toCommon() },
)

private fun Draft.toFfi() = uniffi.bonkc.DraftRecord(
    id = id,
    sellerId = sellerId,
    customerId = customerId,
    docType = docType,
    currency = currency,
    issueDate = issueDate,
    dueDate = dueDate.ifBlank { null },
    deliveryDate = deliveryDate.ifBlank { null },
    buyerReference = buyerReference.ifBlank { null },
    orderReference = orderReference.ifBlank { null },
    note = note.ifBlank { null },
    template = template.ifBlank { null },
    precedingInvoice = precedingInvoice?.let { uniffi.bonkc.PrecedingInvoiceRecord(it.number, it.issueDate) },
    lines = lines.map { it.toFfi() },
)

private fun uniffi.bonkc.DraftLineRecord.toCommon() = DraftLine(
    id = id,
    position = position.toInt(),
    name = name,
    description = description.orEmpty(),
    quantityMicros = quantityMicros,
    unit = unit,
    unitPriceMicros = unitPriceMicros,
    priceBaseQuantityMicros = priceBaseQuantityMicros,
    taxCategory = taxCategory,
    taxRateBp = taxRateBp.toInt(),
    lineNetMicros = lineNetMicros,
    exemptionReason = exemptionReason.orEmpty(),
    exemptionReasonCode = exemptionReasonCode.orEmpty(),
)

private fun DraftLine.toFfi() = uniffi.bonkc.DraftLineRecord(
    id = id,
    position = position.toUInt(),
    name = name,
    description = description.ifBlank { null },
    quantityMicros = quantityMicros,
    unit = unit,
    unitPriceMicros = unitPriceMicros,
    priceBaseQuantityMicros = priceBaseQuantityMicros,
    taxCategory = taxCategory,
    taxRateBp = taxRateBp.toUShort(),
    lineNetMicros = lineNetMicros,
    exemptionReason = exemptionReason.ifBlank { null },
    exemptionReasonCode = exemptionReasonCode.ifBlank { null },
)

private fun uniffi.bonkc.FindingRecord.toCommon() = Finding(
    code = code,
    severity = severity,
    bt = bt.orEmpty(),
    field = field,
    message = detail,
)

private fun uniffi.bonkc.DraftTotalsRecord.toCommon() = DraftTotals(
    netMicros = netMicros,
    taxMicros = taxMicros,
    grossMicros = grossMicros,
)

private fun uniffi.bonkc.IssueRecord.toCommon() = IssueResult(
    issued = issued?.let { summary ->
        IssuedDocument(
            id = summary.id,
            number = summary.number,
            issueDate = summary.issueDate,
            currency = summary.currency,
            netMicros = summary.netMicros,
            taxMicros = summary.taxMicros,
            grossMicros = summary.grossMicros,
            status = summary.status,
        )
    },
    findings = findings.map { it.toCommon() },
    ublSha256 = ublSha256.orEmpty(),
    pdfSha256 = pdfSha256.orEmpty(),
    pdfError = pdfError.orEmpty(),
)

private fun uniffi.bonkc.SellerRecord.toCommon() = Seller(
    id = id,
    name = name,
    street = street.orEmpty(),
    postal = postal.orEmpty(),
    city = city,
    country = country,
    vat = vat.orEmpty(),
    kvk = kvk.orEmpty(),
    peppol = peppol.orEmpty(),
    contact = contact.orEmpty(),
    email = email.orEmpty(),
    phone = phone.orEmpty(),
    defaultCurrency = defaultCurrency,
    defaultLocale = defaultLocale,
)

private fun Seller.toFfi() = uniffi.bonkc.SellerRecord(
    id = id,
    name = name,
    street = street.ifBlank { null },
    postal = postal.ifBlank { null },
    city = city,
    country = country,
    vat = vat.ifBlank { null },
    kvk = kvk.ifBlank { null },
    peppol = peppol.ifBlank { null },
    contact = contact.ifBlank { null },
    email = email.ifBlank { null },
    phone = phone.ifBlank { null },
    defaultCurrency = defaultCurrency,
    defaultLocale = defaultLocale,
)

private fun uniffi.bonkc.DocumentHitRecord.toCommon() = DocumentHit(
    id = id,
    number = number,
    direction = direction,
    party = party,
    issueDate = issueDate,
    currency = currency,
    grossMicros = grossMicros,
    status = status,
    matched = matched,
)

private fun uniffi.bonkc.DocumentLineRecord.toCommon() = DocumentLine(
    position = position,
    name = name,
    description = description.orEmpty(),
    quantity = quantity,
    unit = unit,
    unitPriceMicros = unitPriceMicros,
    vatCategory = vatCategory,
    vatRateBp = vatRateBp.toInt(),
    lineNetMicros = lineNetMicros,
    taxMicros = taxMicros,
)

private fun uniffi.bonkc.DocumentArtefactRecord.toCommon() = DocumentArtefact(
    kind = kind,
    bytesHash = bytesHash,
    sizeBytes = sizeBytes,
    renderedAt = renderedAt,
    rendererVersion = rendererVersion,
)

private fun uniffi.bonkc.ImportRecord.toCommon() = ImportReport(
    number = number,
    documentId = documentId,
    supplier = supplier.orEmpty(),
    duplicate = duplicate,
)

private fun uniffi.bonkc.BackupRecord.toCommon() = BackupExport(path = path, bytes = bytes)

private fun uniffi.bonkc.DocumentEventRecord.toCommon() = DocumentEvent(
    kind = kind,
    at = at,
    actor = actor,
    detail = detail.orEmpty(),
)

private fun uniffi.bonkc.DocumentDetailRecord.toCommon() = DocumentDetail(
    id = id,
    number = number,
    direction = direction,
    docType = docType,
    currency = currency,
    issueDate = issueDate,
    dueDate = dueDate.orEmpty(),
    deliveryDate = deliveryDate.orEmpty(),
    buyerReference = buyerReference.orEmpty(),
    orderReference = orderReference.orEmpty(),
    note = note.orEmpty(),
    seller = seller,
    buyer = buyer,
    supplierId = supplierId.orEmpty(),
    netMicros = netMicros,
    taxMicros = taxMicros,
    grossMicros = grossMicros,
    status = status,
    recordedAt = recordedAt,
    lines = lines.map { it.toCommon() },
    artefacts = artefacts.map { it.toCommon() },
    events = events.map { it.toCommon() },
)

private fun uniffi.bonkc.CustomerImportRecord.toCommon() = CustomerImportReport(
    imported = imported.toInt(),
    skipped = skipped.toInt(),
    errors = errors,
)

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

actual fun resetPassphrase(recoveryKey: String, newPassphrase: String): RecoveryResult = withUserError {
    MobileArchiveHost.resetPassphrase(recoveryKey, newPassphrase)
}

actual fun addRecoveryKey(passphrase: String): String = withUserError {
    MobileArchiveHost.addRecoveryKey(passphrase)
}

actual fun lockArchive() = MobileArchiveHost.lock()

actual fun recordDeviceRevocation() = withUserError { MobileArchiveHost.recordDeviceRevocation() }

actual fun markDeviceRevoked() = MobileArchiveHost.markDeviceRevoked()

actual fun takeDeviceRevoked(): Boolean = MobileArchiveHost.takeDeviceRevoked()

actual fun deviceUnlockAvailable(): Boolean =
    runCatching { KeystoreHost.biometricAvailable() }.getOrDefault(false)

actual fun runCanaryScan(): String = MobileArchiveHost.canaryScan()

private fun ByteArray.containsSequence(needle: ByteArray): Boolean {
    if (needle.isEmpty()) return true
    if (needle.size > size) return false
    for (start in 0..size - needle.size) {
        var matches = true
        for (index in needle.indices) {
            if (this[start + index] != needle[index]) {
                matches = false
                break
            }
        }
        if (matches) return true
    }
    return false
}

internal inline fun <T> withUserError(block: () -> T): T = try {
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
    is AppException.Import -> "Could not import the file: $detail"
    is AppException.AuthFailed -> "Authentication failed: $detail"
    is AppException.Invalidated -> "The device secret was invalidated."
    is AppException.Closed -> "The archive is locked."
}
