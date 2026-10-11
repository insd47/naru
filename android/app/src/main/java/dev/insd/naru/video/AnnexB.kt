package dev.insd.naru.video

/**
 * 디코더에 넣을 접근 단위 하나입니다.
 *
 * - [nals]는 시작 코드 없는 NAL 목록입니다. 코어가 넘겨주는 접근 단위와 같은 모양입니다.
 */
class AccessUnit(val nals: List<ByteArray>) {
    val kind: Kind = Kind.of(nals)

    /** 시작 코드를 붙여 이은 바이트 */
    fun annexB(): ByteArray {
        val size = nals.sumOf { it.size + START_CODE.size }
        val bytes = ByteArray(size)
        var at = 0

        for (nal in nals) {
            START_CODE.copyInto(bytes, at)
            nal.copyInto(bytes, at + START_CODE.size)
            at += START_CODE.size + nal.size
        }

        return bytes
    }

    /** 접근 단위의 종류입니다. 서버의 키프레임은 IDR이 아니라 I 슬라이스만 담은 일반 NAL로 올 수 있습니다. */
    enum class Kind {
        IDR,
        INTRA,
        INTER,
        NONE;

        companion object {
            fun of(nals: List<ByteArray>): Kind {
                val slice = nals.firstOrNull { it.type == NAL_SLICE || it.type == NAL_IDR } ?: return NONE

                if (slice.type == NAL_IDR) {
                    return IDR
                }

                val sliceType = runCatching {
                    val bits = Bits(slice, at = 1)
                    bits.ue()
                    bits.ue()
                }.getOrNull()

                return if (sliceType != null && sliceType % 5 == 2) INTRA else INTER
            }
        }
    }

    companion object {
        const val NAL_SLICE = 1
        const val NAL_IDR = 5

        val START_CODE = byteArrayOf(0, 0, 0, 1)
    }
}

/** NAL 머리의 유형(하위 5비트) */
val ByteArray.type: Int
    get() = this[0].toInt() and 0x1f
