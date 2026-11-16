package com.github.sdgoij.bonk

import org.junit.Assert.assertEquals
import org.junit.Test
import uniffi.bonkc.artefactHashes

/**
 * The sample invoice must render to the same bytes on the device as on the host.
 *
 * The values are the ones `artefact_hashes_match_the_golden_values` asserts in
 * `crates/ffi/src/lib.rs`, and `just mobile-golden` runs both, so a change that
 * drifts on either side fails here or there.
 */
class ArtefactDeterminismTest {
    @Test
    fun the_sample_invoice_matches_the_host() {
        val hashes = artefactHashes()

        assertEquals("UBL length", 4305UL, hashes.ublLen)
        // The PDF moved when the bundled font did: the CFF subset of Noto Serif CJK
        // is smaller than the TrueType one it replaced.
        assertEquals("PDF length", 15527UL, hashes.pdfLen)
        assertEquals(
            "UBL SHA-256",
            "7d7f70eb279208887daae6a0099bf1c61359a709f7fc0e6fb565ea7e9c5f7020",
            hashes.ublSha256,
        )
        assertEquals(
            "PDF SHA-256",
            "0f6aede0989aeecdd72841b0faf8b73f5b6b21d07ce2d4eeaab9f682f3ca0351",
            hashes.pdfSha256,
        )
    }

    @Test
    fun a_second_render_agrees_with_the_first() {
        val first = artefactHashes()
        val second = artefactHashes()

        assertEquals(first.ublSha256, second.ublSha256)
        assertEquals(first.pdfSha256, second.pdfSha256)
        assertEquals(first.ublLen, second.ublLen)
        assertEquals(first.pdfLen, second.pdfLen)
    }
}
