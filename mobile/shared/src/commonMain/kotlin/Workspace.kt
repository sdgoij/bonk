package com.github.sdgoij.bonk

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

/** A customer as the editor and the list use it. */
data class Customer(
    val id: String = "",
    val name: String = "",
    val street: String = "",
    val postal: String = "",
    val city: String = "",
    val country: String = "NL",
    val vat: String = "",
    val kvk: String = "",
    val peppol: String = "",
    val contact: String = "",
    val email: String = "",
    val phone: String = "",
)

/** The seller this archive issues as. */
data class Seller(
    val id: String = "",
    val name: String = "",
    val street: String = "",
    val postal: String = "",
    val city: String = "",
    val country: String = "NL",
    val vat: String = "",
    val kvk: String = "",
    val peppol: String = "",
    val contact: String = "",
    val email: String = "",
    val phone: String = "",
    val defaultCurrency: String = "EUR",
    val defaultLocale: String = "en",
)

/** The invoice a credit note corrects. */
data class PrecedingInvoice(
    val number: String,
    val issueDate: String,
)

/** One line of a draft, with money in micro-units. */
data class DraftLine(
    val id: String,
    val position: Int,
    val name: String,
    val description: String = "",
    val quantityMicros: Long,
    val unit: String = "HUR",
    val unitPriceMicros: Long,
    val priceBaseQuantityMicros: Long = 1_000_000,
    val taxCategory: String = "S",
    val taxRateBp: Int = 2100,
    val lineNetMicros: Long = 0,
    val exemptionReason: String = "",
    val exemptionReasonCode: String = "",
)

/** A draft as the editor holds it. */
data class Draft(
    val id: String,
    val sellerId: String,
    val customerId: String,
    val docType: String = "380",
    val currency: String = "EUR",
    val issueDate: String,
    val dueDate: String = "",
    val deliveryDate: String = "",
    val buyerReference: String = "",
    val orderReference: String = "",
    val note: String = "",
    val template: String = "",
    val precedingInvoice: PrecedingInvoice? = null,
    val lines: List<DraftLine> = emptyList(),
)

/** Totals for a draft preview, in micro-units. */
data class DraftTotals(
    val netMicros: Long,
    val taxMicros: Long,
    val grossMicros: Long,
)

/** One validation finding. */
data class Finding(
    val code: String,
    val severity: String,
    val bt: String = "",
    val field: String = "",
    val message: String = "",
) {
    val blocking: Boolean get() = severity == "blocking"
}

/** An issued document, as the result screen shows it. */
data class IssuedDocument(
    val id: String,
    val number: String,
    val issueDate: String,
    val currency: String,
    val netMicros: Long,
    val taxMicros: Long,
    val grossMicros: Long,
    val status: String,
)

/** The result of an issue attempt: a document, or the findings that stopped it. */
data class IssueResult(
    val issued: IssuedDocument?,
    val findings: List<Finding> = emptyList(),
    val ublSha256: String = "",
    val pdfSha256: String = "",
    val pdfError: String = "",
)

/** One row in the document register. */
data class DocumentHit(
    val id: String,
    val number: String,
    val direction: String,
    val party: String,
    val issueDate: String,
    val currency: String,
    val grossMicros: Long,
    val status: String,
    val matched: Boolean = false,
)

/** One line of a stored document, as recorded. */
data class DocumentLine(
    val position: Long,
    val name: String,
    val description: String = "",
    val quantity: String,
    val unit: String,
    val unitPriceMicros: Long,
    val vatCategory: String,
    val vatRateBp: Int,
    val lineNetMicros: Long,
    val taxMicros: Long,
)

/** One artefact filed under a document. */
data class DocumentArtefact(
    val kind: String,
    val bytesHash: String,
    val sizeBytes: Long,
    val renderedAt: String,
    val rendererVersion: String,
)

/** One status change on a document. */
data class DocumentEvent(
    val kind: String,
    val at: String,
    val actor: String,
    val detail: String = "",
)

/** A stored document, as the detail screen shows it. */
data class DocumentDetail(
    val id: String,
    val number: String,
    val direction: String,
    val docType: String,
    val currency: String,
    val issueDate: String,
    val dueDate: String = "",
    val deliveryDate: String = "",
    val buyerReference: String = "",
    val orderReference: String = "",
    val note: String = "",
    val seller: String,
    val buyer: String,
    val supplierId: String = "",
    val netMicros: Long,
    val taxMicros: Long,
    val grossMicros: Long,
    val status: String,
    val recordedAt: String,
    val lines: List<DocumentLine> = emptyList(),
    val artefacts: List<DocumentArtefact> = emptyList(),
    val events: List<DocumentEvent> = emptyList(),
)

/** Customers, ordered by display name. */
expect fun listCustomers(): List<Customer>

/** Creates a customer when the id is empty, otherwise updates one. */
expect fun saveCustomer(customer: Customer): Customer

/** Drafts, most recently updated first. */
expect fun listDrafts(): List<Draft>

/** Loads a draft by identity. */
expect fun loadDraft(id: String): Draft

/** Creates or updates a draft and returns the stored shape. */
expect fun saveDraft(draft: Draft): Draft

/** Validates a draft against its seller and customer. */
expect fun validateDraft(draft: Draft): List<Finding>

/** Totals for the editor preview, computed by the domain. */
expect fun draftTotals(draft: Draft): DraftTotals

/** Issues a draft, or returns the findings that stopped it. */
expect fun issueDraft(draft: Draft): IssueResult

/** Issued documents, best match first; an empty query lists newest first. */
expect fun listDocuments(query: String, status: String?, direction: String): List<DocumentHit>

/** Records the supplier of a received document against a customer on file. */
expect fun matchSupplier(documentId: String, customerId: String)

/** Moves a received document to a status it can hold, and returns that status. */
expect fun advanceDocument(documentId: String, target: String): String

/** One stored document for the detail screen. */
expect fun loadDocument(id: String): DocumentDetail

/** The seller this archive issues as. */
expect fun loadSeller(): Seller

/** Updates the seller's party data. */
expect fun saveSeller(seller: Seller): Seller

/** A new invoice draft for a customer, dated today and saved. */
expect fun newDraft(customerId: String): Draft

/** What a customer CSV import did. */
data class CustomerImportReport(
    val imported: Int,
    val skipped: Int,
    val errors: List<String>,
)

/** The customer table as CSV. */
expect fun customersCsv(): String

/** What became of one imported invoice. */
data class ImportReport(
    val number: String,
    val documentId: String,
    val supplier: String = "",
    val duplicate: Boolean = false,
)

/** A verified copy written for export. */
data class BackupExport(val path: String, val bytes: Long)

/** Writes a verified copy of the archive, for the user to keep. */
expect fun writeBackup(): BackupExport

/** Replaces this device's archive with a staged pair, after checking it opens. */
expect fun restoreArchive(passphrase: String): ArchiveSummary

/** The keystore bytes, for a recovery kit. */
expect fun recoveryKey(): ByteArray

/** The note that must travel with a recovery key. */
expect fun recoveryNote(): String

/** Free space where a copy is written before it is handed out. */
expect fun freeSpaceBytes(): Long

/** Size of the archive database on disk. */
expect fun databaseBytes(): Long

/**
 * Imports received invoice bytes, as a picker or a share hand-off delivers them.
 *
 * `name` is provenance for the record, not a path: nothing is read from the
 * filesystem and the archive decides the format from the bytes.
 */
expect fun importDocument(name: String, bytes: ByteArray): ImportReport

/** Imports customers from CSV, skipping parties already on file. */
expect fun importCustomersCsv(text: String): CustomerImportReport

/** Records a reason for accepting a non-blocking finding. */
expect fun suppressFinding(draftId: String, code: String, field: String, reason: String)

/** The bytes of one artefact, verified against the hash recorded for it. */
expect fun documentArtefact(id: String, kind: String): ByteArray

/** Reading and writing a text file through the platform picker. */
interface CsvIo {
    /** Opens a save dialog for `suggestedName` and writes `text`. */
    fun export(text: String, suggestedName: String)

    /** Opens a file picker and passes the chosen file's text. */
    fun pick(onText: (String) -> Unit)
}

/** The platform picker, created in a composable so Android can use its launchers. */
@Composable
expect fun rememberCsvIo(): CsvIo

/** Sharing, saving and opening a rendered artefact. */
interface ArtefactIo {
    /** Offers the bytes to other apps through the share sheet. */
    fun share(bytes: ByteArray, name: String, mime: String, onError: (String) -> Unit)

    /** Opens a save dialog and writes the bytes where the user picks. */
    fun save(bytes: ByteArray, name: String, mime: String, onError: (String) -> Unit)

    /** Opens the bytes in whatever app the platform prefers, reporting when none can. */
    fun open(bytes: ByteArray, name: String, mime: String, onError: (String) -> Unit)
}

/** The platform actions, created in a composable so Android can use its launchers. */
@Composable
expect fun rememberArtefactIo(): ArtefactIo

/** Picking one document file and reading its bytes. */
interface DocumentPicker {
    /** Opens a picker for an invoice and passes its display name and bytes. */
    fun pick(onFile: (name: String, bytes: ByteArray) -> Unit, onError: (String) -> Unit)
}

/** The platform picker, created in a composable so Android can use its launchers. */
@Composable
expect fun rememberDocumentPicker(): DocumentPicker

/** Writing the archive's own files where the user keeps them. */
interface BackupIo {
    /** Asks where to put a copy of [path] and streams it there. */
    fun export(path: String, name: String, mime: String, onDone: (String) -> Unit, onError: (String) -> Unit)

    /** Writes the key and its note into a folder the user picks. */
    fun exportKit(key: ByteArray, note: String, onDone: (String) -> Unit, onError: (String) -> Unit)

    /** Picks a database and its key, copying both into the app's staging area. */
    fun stagePair(onDone: () -> Unit, onError: (String) -> Unit)
}

/** The platform writing, created in a composable so Android can use its launchers. */
@Composable
expect fun rememberBackupIo(): BackupIo

/** A document the system handed to the app, waiting for the archive to unlock. */
class ImportRequest(val name: String, val bytes: ByteArray)

/**
 * The hand-off between the activity and the app: `ACTION_VIEW` and `ACTION_SEND`
 * can arrive before the archive is open, so the bytes wait here until a screen
 * takes them.
 */
object PendingImport {
    var request: ImportRequest? by mutableStateOf(null)
        private set

    fun offer(name: String?, bytes: ByteArray) {
        request = ImportRequest(name ?: "document", bytes)
    }

    fun take(): ImportRequest? {
        val current = request
        request = null
        return current
    }
}

/** Lets a platform file picker suspend the background lock while it is open. */
object PickerGuard {
    private var depth = 0

    val active: Boolean get() = depth > 0

    fun enter() {
        depth++
    }

    fun exit() {
        if (depth > 0) depth--
    }
}

/** A fresh identifier for a new line. */
expect fun newId(): String
