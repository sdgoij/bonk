package com.github.sdgoij.bonk

import androidx.compose.ui.unit.dp
import kotlin.test.Test
import kotlin.test.assertEquals

/** The design's adaptive table, pinned so a layout change cannot move a band. */
class WindowWidthTest {
    @Test
    fun the_bands_meet_where_the_design_says() {
        assertEquals(WindowWidth.Compact, WindowWidth.of(599.dp))
        assertEquals(WindowWidth.Medium, WindowWidth.of(600.dp))
        assertEquals(WindowWidth.Medium, WindowWidth.of(839.dp))
        assertEquals(WindowWidth.Expanded, WindowWidth.of(840.dp))
    }

    @Test
    fun a_phone_in_landscape_is_medium_and_a_tablet_is_expanded() {
        assertEquals(WindowWidth.Compact, WindowWidth.of(360.dp))
        assertEquals(WindowWidth.Medium, WindowWidth.of(731.dp))
        assertEquals(WindowWidth.Expanded, WindowWidth.of(1280.dp))
    }

    @Test
    fun only_compact_keeps_the_single_pane() {
        assertEquals(false, WindowWidth.Compact.isAtLeastMedium)
        assertEquals(true, WindowWidth.Medium.isAtLeastMedium)
        assertEquals(true, WindowWidth.Expanded.isAtLeastMedium)
    }
}
