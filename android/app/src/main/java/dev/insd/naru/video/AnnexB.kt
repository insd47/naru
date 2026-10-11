package dev.insd.naru.video

import java.nio.ByteBuffer

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
        const val NAL_SPS = 7
        const val NAL_PPS = 8
        const val NAL_AUD = 9

        val START_CODE = byteArrayOf(0, 0, 0, 1)

        /** Annex-B 바이트를 접근 단위로 나눕니다. */
        fun split(bytes: ByteArray): List<AccessUnit> = units(ByteBuffer.wrap(bytes)).toList()

        /**
         * Annex-B 버퍼를 앞에서부터 읽으며 접근 단위를 하나씩 내놓습니다. 큰 캡처는 메모리 매핑한 버퍼를 넘겨 힙에 통째로 올리지 않습니다.
         *
         * - 파일에는 접근 단위의 경계가 없으므로, 첫 매크로블록이 0인 슬라이스나 SPS, PPS, AUD가 이전 단위의 슬라이스 뒤에 오면 새 단위로 봅니다.
         */
        fun units(buffer: ByteBuffer): Sequence<AccessUnit> = sequence {
            var current = mutableListOf<ByteArray>()

            for (nal in nals(buffer)) {
                val hasSlice = current.any { it.type == NAL_SLICE || it.type == NAL_IDR }
                val startsUnit = when (nal.type) {
                    NAL_SPS, NAL_PPS, NAL_AUD -> true
                    NAL_SLICE, NAL_IDR -> nal.size > 1 && (nal[1].toInt() and 0x80) != 0
                    else -> false
                }

                if (hasSlice && startsUnit) {
                    yield(AccessUnit(current))
                    current = mutableListOf()
                }

                current += nal
            }

            if (current.isNotEmpty()) {
                yield(AccessUnit(current))
            }
        }

        // 3바이트 또는 4바이트 시작 코드로 NAL을 나눕니다. 4바이트 시작 코드의 앞 0은 앞 NAL의 끝에서 떼어 냅니다.
        private fun nals(buffer: ByteBuffer): Sequence<ByteArray> = sequence {
            val limit = buffer.limit()
            var start = -1
            var index = 0

            fun copy(from: Int, until: Int): ByteArray {
                var end = until

                while (end > from && buffer.get(end - 1).toInt() == 0) {
                    end -= 1
                }

                return ByteArray(end - from).also { buffer.duplicate().apply { position(from) }.get(it) }
            }

            while (index + 2 < limit) {
                if (buffer.get(index).toInt() == 0 && buffer.get(index + 1).toInt() == 0 && buffer.get(index + 2).toInt() == 1) {
                    if (start >= 0) {
                        copy(start, index).takeIf { it.isNotEmpty() }?.let { yield(it) }
                    }

                    start = index + 3
                    index += 3
                } else {
                    index += 1
                }
            }

            if (start in 0 until limit) {
                val last = ByteArray(limit - start).also { buffer.duplicate().apply { position(start) }.get(it) }
                yield(last)
            }
        }
    }
}

/** NAL 머리의 유형(하위 5비트) */
val ByteArray.type: Int
    get() = this[0].toInt() and 0x1f
