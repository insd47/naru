package dev.insd.naru.video

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.view.Surface
import dev.insd.naru.protocol.AccessUnit
import java.nio.BufferOverflowException
import java.nio.ByteBuffer

/**
 * 하드웨어 H.264 디코더 하나를 감쌉니다. 접근 단위를 넣으면 Surface에 그립니다.
 *
 * - OS 내장 하드웨어 디코더만 씁니다. 하드웨어 디코더가 없으면 생성할 때 실패합니다.
 * - 비동기 모드입니다. 입력 버퍼가 비는 대로 넣고, 출력은 나오는 대로 그립니다. 코덱의 콜백은 [handler]의 스레드에서 오므로, 이 객체도 그 스레드에서만 부르십시오.
 * - 새 SPS(해상도 변경, 새 SSRC)가 오면 [release] 후 새로 만듭니다. 이전 세대의 참조와 섞으면 깨지기 때문입니다.
 */
class Decoder(sps: ByteArray, pps: ByteArray, surface: Surface, handler: Handler, private val listener: Listener) {
    val info: MediaCodecInfo = hardwareDecoder() ?: error("하드웨어 H.264 디코더가 없습니다")

    private val codec = MediaCodec.createByCodecName(info.name)
    private val units = ArrayDeque<Pair<AccessUnit, Long>>()
    private val inputs = ArrayDeque<Int>()

    // 해제했거나 오류를 낸 뒤에도 이미 올라온 콜백이 남아 있을 수 있으므로 코덱을 만지지 않게 합니다.
    private var released = false

    interface Listener {
        /** 프레임 하나를 Surface에 그렸습니다. */
        fun onFrame()

        /** 코덱이 오류를 냈거나, Surface를 잃었거나, 입력 버퍼보다 큰 단위가 왔습니다. 이 디코더는 더 쓰지 않으므로 새로 만드십시오. */
        fun onError(error: RuntimeException)
    }

    init {
        val config = Sps.parse(sps)
        val format = MediaFormat.createVideoFormat(MIME, config.width, config.height).apply {
            setByteBuffer("csd-0", ByteBuffer.wrap(START_CODE + sps))
            setByteBuffer("csd-1", ByteBuffer.wrap(START_CODE + pps))
            setInteger(MediaFormat.KEY_PRIORITY, 0)

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R &&
                info.getCapabilitiesForType(MIME).isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency)
            ) {
                setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            }

            // Qualcomm 디코더는 표준 저지연 기능을 알리지 않지만, 제조사 키로 출력 지연(재정렬 대기)을 없앨 수 있습니다.
            // 서버 스트림에는 B 프레임이 없으므로 디코딩 순서대로 내보내도 화면에는 차이가 없습니다.
            if (QUALCOMM_PREFIXES.any { info.name.startsWith(it) }) {
                setInteger("vendor.qti-ext-dec-low-latency.enable", 1)
                setInteger("vendor.qti-ext-dec-picture-order.enable", 1)
            }
        }

        codec.setCallback(Callbacks(), handler)
        codec.configure(format, surface, null, 0)
        codec.start()
    }

    /**
     * 접근 단위 하나를 넣습니다. PTS는 출력 순서에만 씁니다.
     *
     * - 디코더가 입력을 받지 못해 [BACKLOG]개가 쌓여 있으면 쌓인 것을 버리고 `false`를 돌려줍니다. 키프레임부터 다시 넣으십시오.
     */
    fun queue(unit: AccessUnit, ptsUs: Long): Boolean {
        if (units.size >= BACKLOG) {
            units.clear()
            return false
        }

        units.addLast(unit to ptsUs)
        feed()
        return true
    }

    fun release() {
        released = true
        runCatching { codec.stop() }
        codec.release()
    }

    // 쌓인 접근 단위를 빈 입력 버퍼에 차례로 넣습니다. 버퍼를 받지 못한 자리는 버리고 단위는 다음 자리에 넣습니다.
    private fun feed() = guard {
        while (!released && units.isNotEmpty() && inputs.isNotEmpty()) {
            val index = inputs.removeFirst()
            val buffer = codec.getInputBuffer(index) ?: continue
            val (unit, ptsUs) = units.removeFirst()
            buffer.clear()

            for (nal in unit.nals) {
                buffer.put(START_CODE).put(nal)
            }

            val flags = if (unit.keyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0
            codec.queueInputBuffer(index, 0, buffer.position(), ptsUs, flags)
        }
    }

    // 코덱이 오류 상태이거나 Surface를 잃으면 호출이 예외를 던집니다. 입력 버퍼보다 큰 단위는 버퍼에 넣다가 넘칩니다.
    // 어느 쪽이든 이 디코더를 그만 쓰고 알립니다.
    private fun guard(block: () -> Unit) {
        try {
            block()
        } catch (error: IllegalStateException) {
            fail(error)
        } catch (error: BufferOverflowException) {
            fail(error)
        }
    }

    private fun fail(error: RuntimeException) {
        if (!released) {
            released = true
            listener.onError(error)
        }
    }

    private inner class Callbacks : MediaCodec.Callback() {
        override fun onInputBufferAvailable(codec: MediaCodec, index: Int) {
            inputs.addLast(index)
            feed()
        }

        override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) = guard {
            if (!released) {
                codec.releaseOutputBuffer(index, true)
                listener.onFrame()
            }
        }

        override fun onError(codec: MediaCodec, error: MediaCodec.CodecException) = fail(error)

        override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) = Unit
    }

    companion object {
        const val MIME = MediaFormat.MIMETYPE_VIDEO_AVC

        // 입력 버퍼를 기다리며 쌓아 두는 접근 단위의 한도. 60fps에서 약 0.5초입니다.
        // 코덱은 시작한 뒤 입력 버퍼를 내주기까지 시간이 걸리므로, 너무 작으면 첫 키프레임부터 버리게 됩니다.
        const val BACKLOG = 30

        private val START_CODE = byteArrayOf(0, 0, 0, 1)

        private val QUALCOMM_PREFIXES = listOf("c2.qti.", "OMX.qcom.")

        // 하드웨어 가속이고 소프트웨어 전용이 아닌 AVC 디코더를 고릅니다. 앱에 소프트웨어 디코더를 넣지 않습니다.
        private fun hardwareDecoder(): MediaCodecInfo? = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.firstOrNull {
            !it.isEncoder && it.isHardwareAccelerated && !it.isSoftwareOnly &&
                it.supportedTypes.any { type -> type.equals(MIME, ignoreCase = true) }
        }
    }
}
