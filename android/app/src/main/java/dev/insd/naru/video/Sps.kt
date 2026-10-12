package dev.insd.naru.video

/**
 * The SPS values needed for the decoder config and tests.
 *
 * - [maxRefFrames]: the server's SPS declares 11. Used to check that the decoder reserves that many reference buffers.
 * - [maxPocLsb]: the POC LSBs wrap at this value. Some hardware decoders silently broke at the wrap point.
 */
data class Sps(
    val profile: Int,
    val level: Int,
    val width: Int,
    val height: Int,
    val maxRefFrames: Int,
    val maxPocLsb: Int?,
) {
    companion object {
        private val HIGH_PROFILES = setOf(100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135)

        /** Parses an SPS NAL without a start code. Throws if it is malformed. */
        fun parse(nal: ByteArray): Sps {
            val bits = Bits(nal, at = 1)
            val profile = bits.bits(8)
            bits.bits(8)
            val level = bits.bits(8)
            bits.ue()

            var chromaFormat = 1

            if (profile in HIGH_PROFILES) {
                chromaFormat = bits.ue()

                if (chromaFormat == 3) {
                    bits.flag()
                }

                bits.ue()
                bits.ue()
                bits.flag()

                if (bits.flag()) {
                    repeat(if (chromaFormat == 3) 12 else 8) { list ->
                        if (bits.flag()) {
                            skipScalingList(bits, if (list < 6) 16 else 64)
                        }
                    }
                }
            }

            bits.ue()

            var maxPocLsb: Int? = null

            when (bits.ue()) {
                0 -> maxPocLsb = 1 shl (bits.ue() + 4)
                1 -> {
                    bits.flag()
                    bits.se()
                    bits.se()
                    repeat(bits.ue()) { bits.se() }
                }
            }

            val maxRefFrames = bits.ue()
            bits.flag()

            val widthInMbs = bits.ue() + 1
            val heightInMapUnits = bits.ue() + 1
            val frameMbsOnly = bits.flag()

            if (!frameMbsOnly) {
                bits.flag()
            }

            bits.flag()

            var width = widthInMbs * 16
            var height = heightInMapUnits * 16 * (if (frameMbsOnly) 1 else 2)

            // In 4:2:0 the crop unit is 2 horizontally and 2 (frames only) or 4 vertically.
            if (bits.flag()) {
                val unitX = if (chromaFormat == 0 || chromaFormat == 3) 1 else 2
                val unitY = (if (chromaFormat == 1) 2 else 1) * (if (frameMbsOnly) 1 else 2)
                val (left, right, top, bottom) = List(4) { bits.ue() }
                width -= unitX * (left + right)
                height -= unitY * (top + bottom)
            }

            return Sps(profile, level, width, height, maxRefFrames, maxPocLsb)
        }

        private fun skipScalingList(bits: Bits, size: Int) {
            var last = 8
            var next = 8

            for (index in 0 until size) {
                if (next != 0) {
                    next = (last + bits.se() + 256) % 256
                }

                last = if (next == 0) last else next
            }
        }
    }
}
