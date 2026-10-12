package dev.insd.naru.video

import org.junit.Assert.assertEquals
import org.junit.Test

class SpsTest {
    // An SPS a Mac sent for a 2560×1600 virtual display. The values were checked separately with ffmpeg's trace_headers.
    private val captured = hex("27640033ac13143002800c9a6e181a0204")

    @Test
    fun capturedSpsDeclaresElevenReferencesAndThirteenBitPoc() {
        val sps = Sps.parse(captured)

        assertEquals(Sps(profile = 100, level = 51, width = 2560, height = 1600, maxRefFrames = 11, maxPocLsb = 8192), sps)
    }

    private fun hex(text: String) = ByteArray(text.length / 2) { text.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
}
