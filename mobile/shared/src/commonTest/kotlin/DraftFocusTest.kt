package com.github.sdgoij.bonk

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class DraftFocusTest {
    private fun line(id: String, position: Int) = DraftLine(
        id = id,
        position = position,
        name = "Consultancy",
        quantityMicros = 1_000_000,
        unitPriceMicros = 100_000_000,
    )

    private val lines = listOf(line("first", 1), line("second", 2))

    @Test
    fun the_document_field_points_at_the_line_list() {
        assertEquals(DraftFocus.Lines, draftFocus("lines", lines))
    }

    @Test
    fun line_fields_resolve_to_the_line_that_validation_indexed() {
        assertEquals(DraftFocus.Line("second", LineField.Name), draftFocus("lines[1].name", lines))
        assertEquals(DraftFocus.Line("first", LineField.Quantity), draftFocus("lines[0].quantity", lines))
        assertEquals(DraftFocus.Line("first", LineField.Unit), draftFocus("lines[0].unit", lines))
        assertEquals(DraftFocus.Line("first", LineField.UnitPrice), draftFocus("lines[0].unit_price", lines))
        assertEquals(
            DraftFocus.Line("first", LineField.UnitPrice),
            draftFocus("lines[0].price_base_quantity", lines),
        )
        assertEquals(DraftFocus.Line("first", LineField.TaxRate), draftFocus("lines[0].vat_category", lines))
        assertEquals(DraftFocus.Line("first", LineField.TaxRate), draftFocus("lines[0].exemption_reason", lines))
    }

    @Test
    fun header_fields_resolve_to_their_inputs() {
        assertEquals(DraftFocus.IssueDate, draftFocus("issue_date", lines))
        assertEquals(DraftFocus.DueDate, draftFocus("due_date", lines))
        assertEquals(DraftFocus.Note, draftFocus("note", lines))
        assertEquals(DraftFocus.Customer, draftFocus("customer.legal_name", lines))
        assertEquals(DraftFocus.Customer, draftFocus("customer.city", lines))
    }

    @Test
    fun fields_the_editor_cannot_show_are_not_focusable() {
        assertNull(draftFocus("lines[0].position", lines))
        assertNull(draftFocus("lines[2].name", lines))
        assertNull(draftFocus("seller.legal_name", lines))
        assertNull(draftFocus("preceding_invoice", lines))
    }
}
