package com.github.sdgoij.bonk

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.window.layout.FoldingFeature
import androidx.window.layout.WindowInfoTracker

@Composable
actual fun rememberFoldGap(): FoldGap? {
    val view = LocalView.current
    val density = LocalDensity.current
    var gap by remember { mutableStateOf<FoldGap?>(null) }

    LaunchedEffect(view) {
        val activity = view.context.findActivity() ?: return@LaunchedEffect
        WindowInfoTracker.getOrCreate(activity).windowLayoutInfo(activity).collect { info ->
            val feature = info.displayFeatures
                .filterIsInstance<FoldingFeature>()
                .firstOrNull { it.isSeparating }
            gap = feature?.let {
                val bounds = it.bounds
                val vertical = it.orientation == FoldingFeature.Orientation.VERTICAL
                FoldGap(
                    orientation = if (vertical) FoldOrientation.Vertical else FoldOrientation.Horizontal,
                    startDp = with(density) {
                        (if (vertical) bounds.left else bounds.top).toDp().value
                    },
                    endDp = with(density) {
                        (if (vertical) bounds.right else bounds.bottom).toDp().value
                    },
                )
            }
        }
    }
    return gap
}

private fun Context.findActivity(): Activity? {
    var context: Context? = this
    while (context is ContextWrapper) {
        if (context is Activity) return context
        context = context.baseContext
    }
    return null
}
