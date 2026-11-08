package com.github.sdgoij.bonk

import kotlin.test.Test
import kotlin.test.assertEquals

/** The pane split either side of a fold, and the usual split when there is none. */
class FoldTest {
    @Test
    fun no_fold_keeps_the_usual_split() {
        val split = paneSplit(1280f, null)

        assertEquals(2f, split.listWeight)
        assertEquals(0f, split.gapDp)
        assertEquals(3f, split.detailWeight)
    }

    @Test
    fun a_vertical_fold_splits_the_panes_at_the_hinge() {
        val split = paneSplit(1280f, FoldGap(FoldOrientation.Vertical, startDp = 620f, endDp = 660f))

        assertEquals(620f, split.listWeight)
        assertEquals(40f, split.gapDp)
        assertEquals(620f, split.detailWeight)
    }

    @Test
    fun a_horizontal_fold_leaves_the_side_by_side_split_alone() {
        val split = paneSplit(1280f, FoldGap(FoldOrientation.Horizontal, startDp = 380f, endDp = 420f))

        assertEquals(2f, split.listWeight)
        assertEquals(0f, split.gapDp)
    }

    @Test
    fun a_fold_at_the_edge_falls_back_to_the_usual_split() {
        val split = paneSplit(1280f, FoldGap(FoldOrientation.Vertical, startDp = 0f, endDp = 40f))

        assertEquals(2f, split.listWeight)
        assertEquals(0f, split.gapDp)
        assertEquals(3f, split.detailWeight)
    }

    @Test
    fun a_fold_that_leaves_a_narrow_pane_is_ignored() {
        val split = paneSplit(800f, FoldGap(FoldOrientation.Vertical, startDp = 100f, endDp = 140f))

        assertEquals(0f, split.gapDp)
    }

    @Test
    fun a_fold_beyond_the_window_falls_back() {
        val split = paneSplit(1000f, FoldGap(FoldOrientation.Vertical, startDp = 500f, endDp = 1200f))

        assertEquals(2f, split.listWeight)
        assertEquals(0f, split.gapDp)
        assertEquals(3f, split.detailWeight)
    }

    @Test
    fun the_gap_never_runs_backwards() {
        assertEquals(0f, FoldGap(FoldOrientation.Vertical, 500f, 400f).sizeDp)
        assertEquals(30f, FoldGap(FoldOrientation.Vertical, 500f, 530f).sizeDp)
    }
}
