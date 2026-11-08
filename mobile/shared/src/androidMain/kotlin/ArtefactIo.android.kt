package com.github.sdgoij.bonk

import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.FileProvider
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import java.io.File

/** The bytes and the failure callback waiting for the save dialog to come back. */
private object PendingArtefact {
    var bytes: ByteArray? = null
    var onError: (String) -> Unit = {}
}

@Composable
actual fun rememberArtefactIo(): ArtefactIo {
    val context = LocalContext.current
    var guarded by remember { mutableStateOf(false) }

    // Share sheets, viewers and the save dialog all stop this activity, which
    // would otherwise lock the archive and drop the screen the user was on.
    fun guard() {
        PickerGuard.enter()
        guarded = true
    }

    fun release() {
        if (guarded) {
            guarded = false
            PickerGuard.exit()
        }
    }

    val owner = context as? LifecycleOwner
    DisposableEffect(owner) {
        if (owner == null) return@DisposableEffect onDispose { }
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_START) release()
        }
        owner.lifecycle.addObserver(observer)
        onDispose { owner.lifecycle.removeObserver(observer) }
    }

    val create = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val bytes = PendingArtefact.bytes
        val onError = PendingArtefact.onError
        PendingArtefact.bytes = null
        PendingArtefact.onError = {}
        val uri = result.data?.data
        if (result.resultCode == Activity.RESULT_OK && uri != null && bytes != null) {
            val failure = runCatching {
                context.contentResolver.openOutputStream(uri)?.use { it.write(bytes) }
            }.exceptionOrNull()
            if (failure != null) onError("Could not write the file: ${failure.message ?: failure}")
        }
    }

    return remember(create) {
        object : ArtefactIo {
            override fun share(bytes: ByteArray, name: String, mime: String, onError: (String) -> Unit) {
                val uri = stage(context, bytes, name, onError) ?: return
                val send = Intent(Intent.ACTION_SEND).apply {
                    type = mime
                    putExtra(Intent.EXTRA_STREAM, uri)
                    putExtra(Intent.EXTRA_SUBJECT, name)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }
                guard()
                context.startActivity(Intent.createChooser(send, name))
            }

            override fun save(bytes: ByteArray, name: String, mime: String, onError: (String) -> Unit) {
                PendingArtefact.bytes = bytes
                PendingArtefact.onError = onError
                val intent = Intent(Intent.ACTION_CREATE_DOCUMENT).apply {
                    addCategory(Intent.CATEGORY_OPENABLE)
                    type = mime
                    putExtra(Intent.EXTRA_TITLE, name)
                }
                guard()
                create.launch(intent)
            }

            override fun open(bytes: ByteArray, name: String, mime: String, onError: (String) -> Unit) {
                val uri = stage(context, bytes, name, onError) ?: return
                val view = Intent(Intent.ACTION_VIEW).apply {
                    setDataAndType(uri, mime)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }
                guard()
                try {
                    context.startActivity(view)
                } catch (_: ActivityNotFoundException) {
                    // Android offers no viewer for every type, and a UBL file is
                    // the common case; say so instead of dropping the user back
                    // to the launcher.
                    release()
                    onError(
                        "Nothing on this device opens ${describe(mime)} files. " +
                            "Share it or save it instead.",
                    )
                }
            }
        }
    }
}

/**
 * Writes the bytes to the private cache and returns a URI another app may read,
 * or reports why the file could not be staged.
 *
 * The file name is the document number, so the receiving app shows something a
 * person recognises; a repeated share overwrites the previous copy.
 */
private fun stage(context: Context, bytes: ByteArray, name: String, onError: (String) -> Unit): Uri? {
    val file = runCatching {
        val directory = File(context.cacheDir, "artefacts").apply { mkdirs() }
        File(directory, name).apply { writeBytes(bytes) }
    }.getOrElse {
        onError("Could not prepare the file: ${it.message ?: it}")
        return null
    }
    return runCatching { FileProvider.getUriForFile(context, "${context.packageName}.fileprovider", file) }
        .getOrElse {
            onError("Could not prepare the file: ${it.message ?: it}")
            null
        }
}

private fun describe(mime: String): String = when (mime) {
    "application/pdf" -> "PDF"
    "application/xml" -> "UBL"
    else -> mime
}
