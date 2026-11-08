package com.github.sdgoij.bonk

import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.ImageBitmap

/**
 * The first page of a PDF, drawn by the platform's own renderer.
 *
 * Returns null when the platform cannot read the bytes, so a screen can say so
 * rather than fail. The page is remembered per byte array, because a preview is
 * only re-read when the document it shows changes.
 */
@Composable
expect fun rememberPdfPage(bytes: ByteArray): ImageBitmap?
