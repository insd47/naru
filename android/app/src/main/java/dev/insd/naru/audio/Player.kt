package dev.insd.naru.audio

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import dev.insd.naru.core.AudioFrame
import java.nio.ByteBuffer

/**
 * Plays the Mac's audio, decoding AAC-ELD with the system decoder into a low-latency AudioTrack.
 *
 * - Call only from the session's event thread. Each input drains the available output, and input is dropped rather than waiting when the buffers are full.
 * - Plays as received, without syncing to video.
 */
class Player(config: ByteArray) {
    private val format = format(config)
    private val codec = MediaCodec.createByCodecName(decoderName(format) ?: error("no AAC-ELD decoder"))
    private val output = MediaCodec.BufferInfo()

    private val track = AudioTrack.Builder()
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build(),
        )
        .setAudioFormat(
            AudioFormat.Builder()
                .setSampleRate(SAMPLE_RATE)
                .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
                .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                .build(),
        )
        .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
        .setBufferSizeInBytes(
            AudioTrack.getMinBufferSize(SAMPLE_RATE, AudioFormat.CHANNEL_OUT_STEREO, AudioFormat.ENCODING_PCM_16BIT) * 2,
        )
        .build()

    init {
        codec.configure(format, null, null, 0)
        codec.start()
        track.play()
    }

    fun play(frame: AudioFrame) {
        val index = codec.dequeueInputBuffer(0)

        if (index >= 0) {
            val buffer = codec.getInputBuffer(index) ?: return
            buffer.clear()
            buffer.put(frame.data)

            // RTP timestamps are 48 kHz. They only order output, so wraparound does not matter.
            val ptsUs = frame.timestamp.toLong() * 1_000 / 48
            codec.queueInputBuffer(index, 0, frame.data.size, ptsUs, 0)
        }

        while (true) {
            val outputIndex = codec.dequeueOutputBuffer(output, 0)

            if (outputIndex == MediaCodec.INFO_TRY_AGAIN_LATER) {
                return
            }

            if (outputIndex < 0) {
                continue
            }

            codec.getOutputBuffer(outputIndex)?.let { pcm ->
                track.write(pcm, output.size, AudioTrack.WRITE_NON_BLOCKING)
            }

            codec.releaseOutputBuffer(outputIndex, false)
        }
    }

    fun release() {
        track.release()
        runCatching { codec.stop() }
        codec.release()
    }

    companion object {
        private const val SAMPLE_RATE = 48_000
        private const val CHANNELS = 2

        /** Whether this device has an AAC-ELD decoder. Without one, audio is not requested. */
        fun supported(): Boolean = decoderName(format(null)) != null

        private fun format(config: ByteArray?) =
            MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, SAMPLE_RATE, CHANNELS).apply {
                setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectELD)
                config?.let { setByteBuffer("csd-0", ByteBuffer.wrap(it)) }
            }

        private fun decoderName(format: MediaFormat): String? =
            MediaCodecList(MediaCodecList.REGULAR_CODECS).findDecoderForFormat(format)
    }
}
