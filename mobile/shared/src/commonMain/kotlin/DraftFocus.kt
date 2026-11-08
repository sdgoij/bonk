package com.github.sdgoij.bonk

import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalSoftwareKeyboardController

/**
 * A request to move the user to the control a finding points at. [token] makes
 * a second tap on the same finding run the effect again.
 */
internal data class DraftFocusRequest(val target: DraftFocus, val token: Int)

/** The editor control a validation finding points at. */
internal sealed interface DraftFocus {
    data object Customer : DraftFocus

    data object IssueDate : DraftFocus

    data object DueDate : DraftFocus

    data object Note : DraftFocus

    data object Lines : DraftFocus

    data class Line(val lineId: String, val field: LineField) : DraftFocus
}

/** The line controls that can carry a finding. */
internal enum class LineField { Name, Quantity, Unit, UnitPrice, TaxRate }

private val linePath = Regex("""^lines\[(\d+)]\.([a-z_]+)$""")

/**
 * Maps a field path from `crux::validate_draft` to the control that shows it.
 * Returns null when this screen has no control for the finding, such as a
 * seller field or a line position the compact editor does not edit.
 *
 * Paths index [lines] in draft order, which is the order validation uses.
 */
internal fun draftFocus(field: String, lines: List<DraftLine>): DraftFocus? {
    if (field == "lines") return DraftFocus.Lines
    linePath.matchEntire(field)?.let { match ->
        val line = match.groupValues[1].toIntOrNull()?.let(lines::getOrNull) ?: return null
        val control = when (match.groupValues[2]) {
            "name", "description" -> LineField.Name
            "quantity" -> LineField.Quantity
            "unit" -> LineField.Unit
            "unit_price" -> LineField.UnitPrice
            // The amount the finding is about; the base quantity has no input here.
            "price_base_quantity" -> LineField.UnitPrice
            // The compact editor edits the rate, not the category or the
            // exemption text.
            "vat_category", "exemption_reason" -> LineField.TaxRate
            else -> null
        } ?: return null
        return DraftFocus.Line(line.id, control)
    }
    return when {
        field == "issue_date" -> DraftFocus.IssueDate
        field == "due_date" -> DraftFocus.DueDate
        field == "note" -> DraftFocus.Note
        field.startsWith("customer.") -> DraftFocus.Customer
        else -> null
    }
}

/**
 * Scrolls the control into view, and focuses it unless [focus] is false, when
 * [request] names it. Focusing first lets the keyboard settle before the
 * scroll measures the space that is left.
 */
@Composable
internal fun Modifier.revealOn(
    request: DraftFocusRequest?,
    target: DraftFocus,
    focus: Boolean = true,
): Modifier {
    val focusRequester = remember { FocusRequester() }
    val bringIntoView = remember { BringIntoViewRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    val token = request?.token
    LaunchedEffect(token) {
        if (request == null || request.target != target) return@LaunchedEffect
        if (focus) {
            focusRequester.requestFocus()
            // Programmatic focus moves the cursor but does not always raise the
            // keyboard, so ask for it as well.
            keyboard?.show()
        }
        bringIntoView.bringIntoView()
    }
    return focusRequester(focusRequester).bringIntoViewRequester(bringIntoView)
}
