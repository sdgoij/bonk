package com.github.sdgoij.bonk

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** The callbacks waiting for the picker to come back. */
private object PendingPick {
    var onFile: (String, ByteArray) -> Unit = { _, _ -> }
    var onError: (String) -> Unit = {}
}

@Composable
actual fun rememberDocumentPicker(): DocumentPicker {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var guarded by remember { mutableStateOf(false) }

    fun guard() {
        PickerGuard.enter()
        guarded = true
    }

    // The picker is another activity; without this the archive would lock and
    // the screen the user returns to would be the unlock form.
    val owner = context as? LifecycleOwner
    DisposableEffect(owner) {
        if (owner == null) return@DisposableEffect onDispose { }
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_START && guarded) {
                guarded = false
                PickerGuard.exit()
            }
        }
        owner.lifecycle.addObserver(observer)
        onDispose { owner.lifecycle.removeObserver(observer) }
    }

    val open = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        val onFile = PendingPick.onFile
        val onError = PendingPick.onError
        PendingPick.onFile = { _, _ -> }
        PendingPick.onError = {}
        if (uri == null) return@rememberLauncherForActivityResult
        scope.launch {
            val read = withContext(Dispatchers.IO) { runCatching { readDocument(context, uri) } }
            read.onSuccess { (name, bytes) -> onFile(name, bytes) }
                .onFailure { onError("Could not read the file: ${it.message ?: it}") }
        }
    }

    return remember(open) {
        object : DocumentPicker {
            override fun pick(onFile: (name: String, bytes: ByteArray) -> Unit, onError: (String) -> Unit) {
                PendingPick.onFile = onFile
                PendingPick.onError = onError
                guard()
                // Any type: providers disagree about which MIME type an XML
                // invoice has, and the archive sniffs the bytes anyway.
                open.launch(arrayOf("*/*"))
            }
        }
    }
}

/** Reads one picked document and the name the provider shows for it. */
internal fun readDocument(context: Context, uri: Uri): Pair<String, ByteArray> {
    val bytes = context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
        ?: error("the file could not be opened")
    return displayName(context, uri) to bytes
}

/**
 * The provider's display name for a URI, falling back to the last path segment.
 *
 * `OpenableColumns` is what the system picker itself reports, so the name the
 * archive records is the one the user saw in the list.
 */
internal fun displayName(context: Context, uri: Uri): String {
    val projection = arrayOf(OpenableColumns.DISPLAY_NAME)
    context.contentResolver.query(uri, projection, null, null, null)?.use { cursor ->
        if (cursor.moveToFirst()) {
            val column = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (column >= 0) {
                val name = cursor.getString(column)
                if (!name.isNullOrBlank()) return name
            }
        }
    }
    return uri.lastPathSegment ?: "document"
}
