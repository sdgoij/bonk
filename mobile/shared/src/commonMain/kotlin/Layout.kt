package com.github.sdgoij.bonk

import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/**
 * The width bands the design uses, measured in dp rather than read off the
 * device type, so a split-screen window or a folded foldable lands in the band
 * its remaining width deserves.
 */
enum class WindowWidth {
    Compact,
    Medium,
    Expanded,
    ;

    val isAtLeastMedium: Boolean get() = this != Compact

    companion object {
        fun of(width: Dp): WindowWidth = when {
            width < 600.dp -> Compact
            width < 840.dp -> Medium
            else -> Expanded
        }
    }
}

/** The band the current window falls in, available to any screen below the shell. */
val LocalWindowWidth = staticCompositionLocalOf { WindowWidth.Compact }
