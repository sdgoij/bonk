package com.github.sdgoij.bonk

import android.content.Context
import android.net.Uri
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
import androidx.documentfile.provider.DocumentFile
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/** The callbacks and the copy waiting for the save dialog to come back. */
private object PendingBackup {
    var path: String = ""
    var onDone: (String) -> Unit = {}
    var onError: (String) -> Unit = {}

    fun clear() {
        path = ""
        onDone = {}
        onError = {}
    }
}

/** The callbacks waiting for the folder picker to come back. */
private object PendingKit {
    var key: ByteArray = ByteArray(0)
    var note: String = ""
    var onDone: (String) -> Unit = {}
    var onError: (String) -> Unit = {}

    fun clear() {
        key = ByteArray(0)
        note = ""
        onDone = {}
        onError = {}
    }
}

@Composable
actual fun rememberBackupIo(): BackupIo {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var guarded by remember { mutableStateOf(false) }

    fun guard() {
        PickerGuard.enter()
        guarded = true
    }

    // A save dialog or a folder picker is another activity; without this the
    // archive would lock and the user would come back to the unlock form.
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

    val create = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")) {
        uri -> handleExport(context, uri)
    }

    val folder = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri ->
        handleKit(context, uri)
    }

    return remember(create, folder) {
        object : BackupIo {
            override fun export(
                path: String,
                name: String,
                mime: String,
                onDone: (String) -> Unit,
                onError: (String) -> Unit,
            ) {
                PendingBackup.path = path
                PendingBackup.onDone = onDone
                PendingBackup.onError = onError
                guard()
                create.launch(name)
            }

            override fun exportKit(
                key: ByteArray,
                note: String,
                onDone: (String) -> Unit,
                onError: (String) -> Unit,
            ) {
                PendingKit.key = key
                PendingKit.note = note
                PendingKit.onDone = onDone
                PendingKit.onError = onError
                guard()
                folder.launch(null)
            }
        }
    }
}

/**
 * Streams the verified copy to the destination the user chose, then deletes it.
 *
 * The copy can be far larger than a parcel, so it is read in chunks rather than
 * loaded whole. Cancelling leaves the cache copy in place, where the next export
 * prunes it.
 */
private fun handleExport(context: Context, uri: Uri?) {
    val path = PendingBackup.path
    val onDone = PendingBackup.onDone
    val onError = PendingBackup.onError
    PendingBackup.clear()
    if (uri == null) return

    val source = File(path)
    val failure = runCatching {
        context.contentResolver.openOutputStream(uri)?.use { output ->
            source.inputStream().use { input -> input.copyTo(output) }
        } ?: error("the destination could not be opened")
    }.exceptionOrNull()
    if (failure != null) {
        onError("Could not write the copy: ${failure.message ?: failure}")
        return
    }
    source.delete()
    onDone(displayName(context, uri))
}

/** Writes the key and its note into the folder the user chose. */
private fun handleKit(context: Context, uri: Uri?) {
    val key = PendingKit.key
    val note = PendingKit.note
    val onDone = PendingKit.onDone
    val onError = PendingKit.onError
    PendingKit.clear()
    if (uri == null) return

    val folder = DocumentFile.fromTreeUri(context, uri)
    if (folder == null) {
        onError("That folder cannot be written to.")
        return
    }
    val failure = runCatching {
        val keyFile = folder.createFile("application/octet-stream", "keystore.bin")
            ?: error("the key file could not be created")
        context.contentResolver.openOutputStream(keyFile.uri)?.use { it.write(key) }
            ?: error("the key file could not be opened")
        val noteFile = folder.createFile("text/plain", "RECOVERY.txt")
            ?: error("the note could not be created")
        context.contentResolver.openOutputStream(noteFile.uri)?.use { it.write(note.toByteArray()) }
            ?: error("the note could not be opened")
    }.exceptionOrNull()
    if (failure != null) {
        onError("Could not write the kit: ${failure.message ?: failure}")
        return
    }
    onDone(folder.name ?: "the folder you chose")
}
