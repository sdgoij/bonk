package com.github.sdgoij.bonk

import android.graphics.Bitmap
import android.graphics.pdf.PdfRenderer
import android.os.ParcelFileDescriptor
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import java.io.File

@Composable
actual fun rememberPdfPage(bytes: ByteArray): ImageBitmap? =
    remember(bytes) { renderFirstPage(bytes)?.asImageBitmap() }

/**
 * Draws the first page through the platform's PDF renderer.
 *
 * PdfRenderer reads a descriptor rather than bytes, so the document is written
 * to the app's scratch directory and read back while it is there. The file is
 * deleted even when the renderer refuses the document.
 */
private fun renderFirstPage(bytes: ByteArray): Bitmap? {
    val directory = MobileArchiveHost.scratchDirectory() ?: return null
    directory.mkdirs()
    return runCatching {
        val file = File.createTempFile("preview-", ".pdf", directory)
        try {
            file.writeBytes(bytes)
            ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY).use { descriptor ->
                PdfRenderer(descriptor).use { renderer ->
                    if (renderer.pageCount == 0) {
                        null
                    } else {
                        renderer.openPage(0).use { page ->
                            // A portrait page at 1080 px wide is enough for a pane on a
                            // tablet and is not worth caching at the screen's density.
                            val width = 1080
                            val height = (width.toLong() * page.height / page.width).toInt().coerceAtLeast(1)
                            val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
                            bitmap.eraseColor(android.graphics.Color.WHITE)
                            page.render(bitmap, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                            bitmap
                        }
                    }
                }
            }
        } finally {
            file.delete()
        }
    }.getOrNull()
}
