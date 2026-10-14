package dev.insd.naru.video

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.view.Surface
import dev.insd.naru.core.AccessUnit
import dev.insd.naru.core.DecoderConfig
import java.nio.BufferOverflowException
import java.nio.ByteBuffer

/**
 * Wraps one hardware H.264 decoder that renders queued access units to a Surface.
 *
 * - Uses only the OS's built-in hardware decoders. Construction fails if there is none.
 * - Runs in async mode: input is queued as buffers free up and output is rendered as it comes. Codec callbacks arrive on [handler]'s thread, so call this object only on that thread.
 * - On a new config, [release] and create a new one, since mixing with the previous generation's references corrupts output.
 */
class Decoder(config: DecoderConfig, surface: Surface, handler: Handler, private val listener: Listener) {
    val info: MediaCodecInfo = hardwareDecoder() ?: error("no hardware H.264 decoder")

    private val codec = MediaCodec.createByCodecName(info.name)
    private val units = ArrayDeque<Pair<AccessUnit, Long>>()
    private val inputs = ArrayDeque<Int>()

    // Callbacks already posted may still run after release or an error, so they must not touch the codec.
    private var released = false

    interface Listener {
        /** A frame was rendered to the Surface. */
        fun onFrame()

        /** The codec failed, the Surface was lost, or a unit larger than the input buffer arrived. This decoder is done; create a new one. */
        fun onError(error: RuntimeException)
    }

    init {
        val format = MediaFormat.createVideoFormat(MIME, config.width.toInt(), config.height.toInt()).apply {
            setByteBuffer("csd-0", ByteBuffer.wrap(START_CODE + config.sps.first()))
            setByteBuffer("csd-1", ByteBuffer.wrap(START_CODE + config.pps.first()))
            setInteger(MediaFormat.KEY_PRIORITY, 0)

            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R &&
                info.getCapabilitiesForType(MIME).isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_LowLatency)
            ) {
                setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            }

            // Qualcomm decoders do not advertise the standard low-latency feature, but a vendor key removes the output delay (reorder wait).
            // The server stream has no B-frames, so emitting in decode order makes no visible difference.
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
     * Queues one access unit. The PTS only orders output.
     *
     * - If the decoder is not taking input and [BACKLOG] units are pending, drops them and returns `false`. Resume from a keyframe.
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

    // Feeds pending access units into free input buffers in order. An index whose buffer is unavailable is dropped and the unit waits for the next one.
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

    // Calls throw when the codec is in an error state or the Surface is lost, and a unit larger than the input buffer overflows it.
    // Either way, this decoder is abandoned and reported.
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
        private const val MIME = MediaFormat.MIMETYPE_VIDEO_AVC

        // Limit on access units held while waiting for input buffers, about 0.5 s at 60 fps.
        // A codec takes a while after start to hand out input buffers, so a smaller limit would drop the first keyframe.
        const val BACKLOG = 30

        private val START_CODE = byteArrayOf(0, 0, 0, 1)

        private val QUALCOMM_PREFIXES = listOf("c2.qti.", "OMX.qcom.")

        // Picks an AVC decoder that is hardware accelerated and not software only. The app bundles no software decoder.
        private fun hardwareDecoder(): MediaCodecInfo? = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.firstOrNull {
            !it.isEncoder && it.isHardwareAccelerated && !it.isSoftwareOnly &&
                it.supportedTypes.any { type -> type.equals(MIME, ignoreCase = true) }
        }
    }
}
