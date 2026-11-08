package com.github.sdgoij.bonk

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner

/**
 * The archive is process-private, so leaving the screen should not leave the
 * key in memory. ON_STOP also covers the recents screen and a locked device.
 */
@Composable
actual fun LockOnBackground(onLock: () -> Unit) {
    val owner = LocalContext.current as? LifecycleOwner ?: return
    val current = rememberUpdatedState(onLock)
    DisposableEffect(owner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) current.value()
        }
        owner.lifecycle.addObserver(observer)
        onDispose { owner.lifecycle.removeObserver(observer) }
    }
}
