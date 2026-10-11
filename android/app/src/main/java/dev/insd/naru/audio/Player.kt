package dev.insd.naru.audio

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import dev.insd.naru.protocol.AudioFrame
import java.nio.ByteBuffer

/**
 * Mac의 소리를 재생합니다. AAC-ELD를 시스템 디코더로 풀어 저지연 AudioTrack으로 내보냅니다.
 *
 * - 세션 스레드에서만 부르십시오. 넣을 때마다 나온 출력을 꺼내 쓰고, 버퍼가 차 있으면 기다리지 않고 버립니다.
 * - 영상과 시각을 맞추지 않고 받는 대로 재생합니다.
 */
class Player(config: ByteArray) {
    private val format = format(config)
    private val codec = MediaCodec.createByCodecName(decoderName(format) ?: error("AAC-ELD 디코더가 없습니다"))
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

            // RTP 타임스탬프는 48 kHz입니다. 출력 순서에만 쓰므로 넘어감은 신경 쓰지 않습니다.
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

        /** 이 기기에 AAC-ELD 디코더가 있는지 여부. 없으면 소리를 받지 않습니다. */
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
