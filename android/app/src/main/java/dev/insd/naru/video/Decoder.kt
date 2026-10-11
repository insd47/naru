package dev.insd.naru.video

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.view.Surface
import dev.insd.naru.protocol.AccessUnit

/**
 * 하드웨어 H.264 디코더 하나를 감쌉니다. 접근 단위를 넣으면 Surface에 그립니다.
 *
 * - OS 내장 하드웨어 디코더만 씁니다. 하드웨어 디코더가 없으면 생성할 때 실패합니다.
 * - 동기 모드이며, 한 스레드에서만 부르십시오.
 * - 새 SPS(해상도 변경, 새 SSRC)가 오면 [release] 후 새로 만듭니다. 이전 세대의 참조와 섞으면 깨지기 때문입니다.
 */
class Decoder(sps: ByteArray, pps: ByteArray, surface: Surface) {
    val info: MediaCodecInfo = hardwareDecoder() ?: error("하드웨어 H.264 디코더가 없습니다")
    val config: Sps = Sps.parse(sps)

    /** Qualcomm 제조사 저지연 키를 넣었는지 여부 */
    val vendorLowLatency: Boolean = QUALCOMM_PREFIXES.any { info.name.startsWith(it) }

    private val codec = MediaCodec.createByCodecName(info.name)
    private val output = MediaCodec.BufferInfo()

    // 넣은 시각을 PTS로 찾아 디코딩 지연을 잽니다.
    private val queuedAt = HashMap<Long, Long>()

    init {
        val format = MediaFormat.createVideoFormat(MIME, config.width, config.height).apply {
            setByteBuffer("csd-0", java.nio.ByteBuffer.wrap(START_CODE + sps))
            setByteBuffer("csd-1", java.nio.ByteBuffer.wrap(START_CODE + pps))
            setInteger(MediaFormat.KEY_PRIORITY, 0)

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R && lowLatency) {
                setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            }

            // Qualcomm 디코더는 표준 저지연 기능을 알리지 않지만, 제조사 키로 출력 지연(재정렬 대기)을 없앨 수 있습니다.
            // 서버 스트림에는 B 프레임이 없으므로 디코딩 순서대로 내보내도 화면에는 차이가 없습니다.
            if (vendorLowLatency) {
                setInteger("vendor.qti-ext-dec-low-latency.enable", 1)
                setInteger("vendor.qti-ext-dec-picture-order.enable", 1)
            }
        }

        codec.configure(format, surface, null, 0)
        codec.start()
    }

    /** 이 디코더가 저지연 모드를 지원하는지 여부 */
    val lowLatency: Boolean
        get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.R &&
            info.getCapabilitiesForType(MIME).isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency)

    /** 접근 단위 하나를 넣습니다. 입력 버퍼를 기다리다 시간이 지나면 `false`입니다. */
    fun queue(unit: AccessUnit, ptsUs: Long, timeoutUs: Long = 50_000): Boolean {
        val index = codec.dequeueInputBuffer(timeoutUs)

        if (index < 0) {
            return false
        }

        val buffer = codec.getInputBuffer(index) ?: return false
        buffer.clear()

        for (nal in unit.nals) {
            buffer.put(START_CODE).put(nal)
        }

        val flags = if (unit.keyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0
        queuedAt[ptsUs] = System.nanoTime()
        codec.queueInputBuffer(index, 0, buffer.position(), ptsUs, flags)
        return true
    }

    /** 나온 프레임을 모두 Surface에 그리고, 프레임마다 [onFrame]을 부릅니다. */
    fun drain(onFrame: (Frame) -> Unit, onFormat: (MediaFormat) -> Unit) {
        while (true) {
            when (val index = codec.dequeueOutputBuffer(output, 0)) {
                MediaCodec.INFO_TRY_AGAIN_LATER -> return
                MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> onFormat(codec.outputFormat)
                else -> if (index >= 0) {
                    val queued = queuedAt.remove(output.presentationTimeUs)
                    val latencyUs = queued?.let { (System.nanoTime() - it) / 1_000 }
                    codec.releaseOutputBuffer(index, true)
                    onFrame(Frame(output.presentationTimeUs, latencyUs))
                }
            }
        }
    }

    fun release() {
        runCatching { codec.stop() }
        codec.release()
    }

    /** 디코더가 내놓은 프레임 하나 */
    data class Frame(val ptsUs: Long, val latencyUs: Long?)

    companion object {
        const val MIME = MediaFormat.MIMETYPE_VIDEO_AVC

        private val START_CODE = byteArrayOf(0, 0, 0, 1)

        private val QUALCOMM_PREFIXES = listOf("c2.qti.", "OMX.qcom.")

        // 하드웨어 가속이고 소프트웨어 전용이 아닌 AVC 디코더를 고릅니다. 앱에 소프트웨어 디코더를 넣지 않습니다.
        private fun hardwareDecoder(): MediaCodecInfo? = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.firstOrNull {
            !it.isEncoder && it.isHardwareAccelerated && !it.isSoftwareOnly &&
                it.supportedTypes.any { type -> type.equals(MIME, ignoreCase = true) }
        }
    }
}
