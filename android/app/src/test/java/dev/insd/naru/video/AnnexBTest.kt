package dev.insd.naru.video

import org.junit.Assert.assertEquals
import org.junit.Test

class AnnexBTest {
    private val sps = byteArrayOf(0x67, 0x64)
    private val pps = byteArrayOf(0x68, 0xee.toByte())

    // 첫 매크로블록이 0인 슬라이스(둘째 바이트의 최상위 비트가 1)
    private val idr = byteArrayOf(0x65, 0x88.toByte(), 0x11)
    private val inter = byteArrayOf(0x41, 0x9a.toByte(), 0x22)

    // I 슬라이스만 담은 일반 NAL: first_mb_in_slice = 0('1'), slice_type = 7('0001000')
    private val intra = byteArrayOf(0x41, 0x88.toByte(), 0x00)

    // 같은 그림의 둘째 슬라이스(first_mb_in_slice ≠ 0)
    private val secondSlice = byteArrayOf(0x41, 0x40, 0x33)

    @Test
    fun parameterSetsJoinTheFollowingKeyframe() {
        val units = AccessUnit.split(annexB(sps, pps, idr, inter, sps, pps, intra))

        assertEquals(listOf(AccessUnit.Kind.IDR, AccessUnit.Kind.INTER, AccessUnit.Kind.INTRA), units.map { it.kind })
        assertEquals(3, units[0].nals.size)
        assertEquals(3, units[2].nals.size)
    }

    @Test
    fun laterSlicesOfOnePictureStayInOneUnit() {
        val units = AccessUnit.split(annexB(inter, secondSlice, inter))
        assertEquals(listOf(2, 1), units.map { it.nals.size })
    }

    @Test
    fun threeByteStartCodesAreAccepted() {
        val bytes = byteArrayOf(0, 0, 1) + idr + byteArrayOf(0, 0, 1) + inter
        assertEquals(2, AccessUnit.split(bytes).size)
    }

    @Test
    fun annexBRoundTripsTheUnit() {
        val unit = AccessUnit.split(annexB(sps, pps, idr)).single()
        assertEquals(annexB(sps, pps, idr).toList(), unit.annexB().toList())
    }

    private fun annexB(vararg nals: ByteArray) = nals.fold(ByteArray(0)) { bytes, nal -> bytes + AccessUnit.START_CODE + nal }
}
