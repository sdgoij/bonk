package com.github.sdgoij.bonk

import uniffi.bonkc.artefactHashes

actual fun runArtefactHashes(): String {
    val hashes = artefactHashes()
    return buildString {
        appendLine("ubl_sha256=${hashes.ublSha256}")
        appendLine("pdf_sha256=${hashes.pdfSha256}")
        appendLine("ubl_len=${hashes.ublLen}")
        append("pdf_len=${hashes.pdfLen}")
    }
}
