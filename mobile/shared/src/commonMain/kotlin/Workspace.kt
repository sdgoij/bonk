package com.github.sdgoij.bonk

import androidx.compose.runtime.Composable

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

/** Imports customers from CSV, skipping parties already on file. */
expect fun importCustomersCsv(text: String): CustomerImportReport

/** Records a reason for accepting a non-blocking finding. */
expect fun suppressFinding(draftId: String, code: String, field: String, reason: String)

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
