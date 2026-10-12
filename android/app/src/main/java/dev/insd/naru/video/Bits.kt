package dev.insd.naru.video

/**
 * Reads an H.264 RBSP bit by bit from the front, skipping emulation prevention bytes (the `03` in `00 00 03`).
 *
 * - Throws [IllegalStateException] when the data runs out.
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

    /** Unsigned Exp-Golomb code (ue(v)) */
    fun ue(): Int {
        var leading = 0

        while (bit() == 0) {
            leading += 1
            check(leading <= 31) { "Exp-Golomb code too long" }
        }

        return ((1L shl leading) - 1 + bits(leading)).toInt()
    }

    /** Signed Exp-Golomb code (se(v)) */
    fun se(): Int {
        val code = ue()
        return if (code % 2 == 1) (code + 1) / 2 else -(code / 2)
    }

    private fun byteAt(index: Int): Int {
        check(index < bytes.size) { "RBSP ended" }
        return bytes[index].toInt() and 0xff
    }
}
