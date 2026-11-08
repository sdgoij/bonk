package com.github.sdgoij.bonk

import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext

private object Pending {
    var exportText: String? = null
    var importCallback: ((String) -> Unit)? = null
}

@Composable
actual fun rememberCsvIo(): CsvIo {
    val context = LocalContext.current
    val create = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("text/csv")) { uri: Uri? ->
        val text = Pending.exportText
        Pending.exportText = null
        PickerGuard.exit()
        if (uri != null && text != null) {
            runCatching {
                context.contentResolver.openOutputStream(uri)?.use { stream -> stream.write(text.toByteArray()) }
            }
        }
    }
    val open = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
        val callback = Pending.importCallback
        Pending.importCallback = null
        PickerGuard.exit()
        if (uri != null && callback != null) {
            val text = runCatching {
                context.contentResolver.openInputStream(uri)?.bufferedReader()?.use { it.readText() }
            }.getOrNull()
            if (text != null) callback(text)
        }
    }

    return remember(create, open) {
        object : CsvIo {
            override fun export(text: String, suggestedName: String) {
                Pending.exportText = text
                PickerGuard.enter()
                create.launch(suggestedName)
            }

            override fun pick(onText: (String) -> Unit) {
                Pending.importCallback = onText
                PickerGuard.enter()
                open.launch(arrayOf("text/csv", "text/comma-separated-values", "text/plain"))
            }
        }
    }
}
