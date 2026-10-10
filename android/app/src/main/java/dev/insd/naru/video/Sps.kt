package dev.insd.naru.video

/**
 * SPS에서 디코더 설정과 시험에 필요한 값만 꺼낸 것입니다.
 *
 * - [maxRefFrames]: 서버의 SPS는 8장이라고 선언하지만 인코더가 9장까지 참조한다고 알려져 있어, 엄격한 디코더에서 번짐이 생길 수 있습니다.
 * - [maxPocLsb]: POC 하위 비트가 이 값에서 넘어갑니다. 넘김 지점에서 조용히 깨지는 하드웨어 디코더가 있었습니다.
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

        /** 시작 코드 없는 SPS NAL을 해석합니다. 형식이 맞지 않으면 예외를 던집니다. */
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

            // 크롭 단위는 4:2:0에서 가로 2, 세로 2(프레임 전용) 또는 4입니다.
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
