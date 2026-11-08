package com.github.sdgoij.bonk

import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf

/** Which way the fold line runs across the window. */
enum class FoldOrientation {
    /** A vertical fold line: the two halves sit side by side. */
    Vertical,

    /** A horizontal fold line: the two halves sit above and below. */
    Horizontal,
}

/**
 * The strip a fold takes out of the window.
 *
 * The start and end are along the axis the fold line runs perpendicular to, in
 * dp from the window's left edge for a vertical fold and from its top edge for a
 * horizontal one, so a layout can put a pane on either side of the strip.
 */
data class FoldGap(
    val orientation: FoldOrientation,
    val startDp: Float,
    val endDp: Float,
) {
    val sizeDp: Float get() = (endDp - startDp).coerceAtLeast(0f)
}

/** The fold the window currently has, or null when it has none. */
val LocalFoldGap = staticCompositionLocalOf<FoldGap?> { null }

/**
 * Watches the window for a fold. The platform only reports one on a foldable
 * that is actually half open, so a flat tablet gives null.
 */
@Composable
expect fun rememberFoldGap(): FoldGap?

/** The narrowest a pane may be before the fold is treated as unusable. */
private const val MIN_PANE_DP = 160f

/** How a two-pane layout splits a window that may have a fold down the middle. */
data class PaneSplit(val listWeight: Float, val gapDp: Float, val detailWeight: Float)

/**
 * The list and detail widths for a window.
 *
 * With no fold the list takes two fifths and the detail three. With a vertical
 * fold the split follows the fold, so neither pane is drawn across it; a fold
 * that sits at an edge, or leaves a pane too narrow to use, falls back to the
 * usual weights.
 */
fun paneSplit(windowWidthDp: Float, fold: FoldGap?): PaneSplit {
    if (fold == null || fold.orientation != FoldOrientation.Vertical) {
        return PaneSplit(listWeight = 2f, gapDp = 0f, detailWeight = 3f)
    }
    val start = fold.startDp.coerceIn(0f, windowWidthDp)
    val end = fold.endDp.coerceIn(start, windowWidthDp)
    val list = start
    val detail = windowWidthDp - end
    if (list < MIN_PANE_DP || detail < MIN_PANE_DP) {
        return PaneSplit(listWeight = 2f, gapDp = 0f, detailWeight = 3f)
    }
    return PaneSplit(listWeight = list, gapDp = end - start, detailWeight = detail)
}
