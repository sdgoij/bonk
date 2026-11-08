package com.github.sdgoij.bonk

import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.time.LocalDate

/**
 * The §6.8 timings the application controls, measured on the device.
 *
 * This creates its own archive, so it replaces whatever is in the app's storage
 * on the device it runs on. The ceilings are wide on purpose: they are
 * regression guards on an emulator, while the design's numbers (2.5 s unlock,
 * 1.5 s issue) are for a mid-range phone and are read from the log line rather
 * than asserted here.
 */
@RunWith(AndroidJUnit4::class)
class BudgetTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun unlock_issue_and_preview_report_their_timings() {
        runCatching { lockArchive() }
        File(context.filesDir, "archive").deleteRecursively()
        MobileArchiveHost.attach(context)

        try {
            val createMillis = measureMillis { createArchive(PASSPHRASE) }

            val seller = saveSeller(
                loadSeller().copy(
                    name = "Budget Studio B.V.",
                    street = "Keizersgracht 1",
                    postal = "1015 AA",
                    city = "Amsterdam",
                    vat = "NL123456782B01",
                    peppol = "0088:1234567890123",
                ),
            )
            val customer = saveCustomer(
                Customer(
                    name = "Budget Buyer B.V.",
                    street = "Oudegracht 1",
                    postal = "3511 AA",
                    city = "Utrecht",
                    peppol = "0088:9876543210987",
                ),
            )

            val draft = newDraft(customer.id)
            val withLine = saveDraft(
                draft.copy(
                    buyerReference = "PO-BUDGET",
                    dueDate = LocalDate.now().plusDays(30).toString(),
                    lines = listOf(
                        DraftLine(
                            id = newId(),
                            position = 1,
                            name = "Consultancy",
                            quantityMicros = 1_000_000,
                            unitPriceMicros = 100_000_000,
                        ),
                    ),
                ),
            )
            assertTrue(
                "the draft must pass validation before it can be timed",
                validateDraft(withLine).none { it.blocking },
            )

            val previewMillis = measureMillis { draftPreview(withLine) }

            lockArchive()
            var reopened: ArchiveSummary? = null
            val unlockMillis = measureMillis { reopened = openArchive(PASSPHRASE) }
            assertNotNull("the archive did not reopen", reopened)

            var result: IssueResult? = null
            val issueMillis = measureMillis { result = issueDraft(withLine) }
            assertNotNull(
                "the draft did not issue: " + result?.findings?.joinToString { "${it.code} ${it.field} ${it.message}" },
                result?.issued,
            )

            Log.i(
                TAG,
                "create=${createMillis}ms unlock=${unlockMillis}ms issue=${issueMillis}ms preview=${previewMillis}ms",
            )

            assertTrue("unlock took $unlockMillis ms", unlockMillis < 5_000)
            assertTrue("issue took $issueMillis ms", issueMillis < 5_000)
            assertTrue("preview took $previewMillis ms", previewMillis < 2_000)
        } finally {
            runCatching { lockArchive() }
            MobileArchiveHost.detach()
        }
    }

    private inline fun measureMillis(block: () -> Unit): Long {
        val start = System.nanoTime()
        block()
        return (System.nanoTime() - start) / 1_000_000
    }

    private companion object {
        const val TAG = "BonkBudget"
        const val PASSPHRASE = "budget-passphrase"
    }
}
