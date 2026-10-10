package dev.insd.naru.video

/**
 * H.264 RBSP를 앞에서부터 비트 단위로 읽습니다. 에뮬레이션 방지 바이트(`00 00 03`의 `03`)는 건너뜁니다.
 *
 * - 데이터가 끝나면 [IllegalStateException]을 던집니다.
 */
class Bits(private val bytes: ByteArray, private var at: Int = 0) {
    private var bit = 0
    private var zeros = 0

    fun bit(): Int {
        if (bit == 0) {
            if (zeros >= 2 && byteAt(at) == 3) {
                at += 1
                zeros = 0
            }

            zeros = if (byteAt(at) == 0) zeros + 1 else 0
        }

        val value = (byteAt(at) shr (7 - bit)) and 1
        bit += 1

        if (bit == 8) {
            bit = 0
            at += 1
        }

        return value
    }

    fun bits(count: Int): Int = (0 until count).fold(0) { value, _ -> (value shl 1) or bit() }

    fun flag(): Boolean = bit() == 1

    /** 부호 없는 지수 골롬 부호(ue(v)) */
    fun ue(): Int {
        var leading = 0

        while (bit() == 0) {
            leading += 1
            check(leading <= 31) { "지수 골롬 부호가 너무 깁니다" }
        }

        return ((1L shl leading) - 1 + bits(leading)).toInt()
    }

    /** 부호 있는 지수 골롬 부호(se(v)) */
    fun se(): Int {
        val code = ue()
        return if (code % 2 == 1) (code + 1) / 2 else -(code / 2)
    }

    private fun byteAt(index: Int): Int {
        check(index < bytes.size) { "RBSP가 끝났습니다" }
        return bytes[index].toInt() and 0xff
    }
}
