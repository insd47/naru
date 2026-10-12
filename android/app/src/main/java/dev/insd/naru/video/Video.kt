package dev.insd.naru.video

import android.media.MediaCodec
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.view.Surface
import dev.insd.naru.TAG
import dev.insd.naru.protocol.AccessUnit
import dev.insd.naru.protocol.DecoderConfig
import dev.insd.naru.protocol.MediaStats
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * 세션의 영상을 Surface에 그립니다.
 *
 * - 디코더와 그 콜백은 전용 스레드 하나에서만 다룹니다. 어느 스레드에서 불러도 그 스레드로 넘겨 차례로 처리합니다.
 * - 디코더는 설정(SPS, PPS)과 Surface가 모두 있을 때만 있습니다. 둘 중 하나가 바뀌면 새로 만들고 키프레임부터 넣습니다.
 * - 키프레임은 Surface가 바뀌었거나, 디코더가 오류를 냈거나, 입력이 밀렸을 때 [requestKeyframe]으로 요청합니다. 새 설정과 멈춤 복구에는 세션이 보냅니다.
 * - 디코더를 새로 만들 때마다 첫 프레임을 그리면 [onFirstFrame]을 부릅니다. 그때까지 화면은 비어 있습니다.
 * - [release] 뒤에 들어온 일은 하지 않습니다.
 */
class Video(private val requestKeyframe: () -> Unit, private val onFirstFrame: () -> Unit) {
    private val thread = HandlerThread("naru-video").apply { start() }
    private val handler = Handler(thread.looper)

    private var surface: Surface? = null
    private var sps: ByteArray? = null
    private var pps: ByteArray? = null
    private var decoder: Decoder? = null
    private var shown = false
    private var released = false

    // 디코더를 새로 만든 뒤에는 키프레임이 올 때까지 넣지 않습니다.
    // 요청이 서버에 닿지 않았을 수 있으므로, 기다리는 동안 프레임이 오면 일정 간격으로 다시 요청합니다.
    private var awaitingKeyframe = true
    private var requestedAt = 0L

    // 통계 사건마다 남기는 수신 기록입니다.
    private var frames = 0
    private var keyframes = 0
    private var dropped = 0
    private var rendered = 0

    private val listener = object : Decoder.Listener {
        override fun onFrame() {
            rendered += 1

            if (!shown) {
                shown = true
                onFirstFrame()
            }
        }

        // 코덱의 콜백 안에서 그 코덱을 해제하지 않도록 콜백이 끝난 뒤에 새로 만듭니다.
        override fun onError(error: RuntimeException) {
            Log.w(TAG, "디코더 오류: ${(error as? MediaCodec.CodecException)?.diagnosticInfo ?: error.message}")
            handler.post { rebuild(keyframe = true) }
        }
    }

    /**
     * 영상을 그릴 Surface. 사라지면 `null`을 넘깁니다.
     *
     * - Surface가 사라진 뒤에는 그리면 안 되므로, `null`이면 디코더를 해제할 때까지 기다립니다.
     */
    fun attach(next: Surface?) {
        val detached = CountDownLatch(1)

        val posted = handler.post {
            if (next == null || next !== surface || decoder == null) {
                Log.i(TAG, "Surface ${if (next == null) "사라짐" else "연결"}")
                surface = next
                rebuild(keyframe = true)
            }

            detached.countDown()
        }

        if (next == null && posted) {
            detached.await(DETACH_TIMEOUT_MS, TimeUnit.MILLISECONDS)
        }
    }

    // 서버는 키프레임마다 같은 설정을 다시 보냅니다. 같은 설정이면 디코더를 그대로 두고, 바뀌었을 때만 새로 만듭니다.
    // 새로 만들 때는 뒤따르는 키프레임이 곧 오므로 따로 요청하지 않습니다.
    fun configure(config: DecoderConfig) {
        handler.post {
            val nextSps = config.sps.firstOrNull() ?: return@post
            val nextPps = config.pps.firstOrNull() ?: return@post

            if (decoder == null || !sps.contentEquals(nextSps) || !pps.contentEquals(nextPps)) {
                sps = nextSps
                pps = nextPps
                rebuild(keyframe = false)
            }
        }
    }

    fun queue(unit: AccessUnit) {
        handler.post { decode(unit) }
    }

    /** 프레임이 오래 나오지 않았습니다. 디코더를 새로 만듭니다. 키프레임은 세션이 요청합니다. */
    fun reset() {
        handler.post { rebuild(keyframe = false) }
    }

    fun report(stats: MediaStats) {
        handler.post {
            Log.i(
                TAG,
                "수신(0.5초): 프레임 $frames (키 $keyframes, 버림 $dropped), 출력 $rendered | 데이터그램 ${stats.datagrams}, " +
                    "인증 ${stats.authenticated}, SSRC ${stats.ssrc?.toString(16)}, 순번 ${stats.highest}, 후보 ${stats.candidates}, " +
                    "오퍼 ${stats.offers}, NACK ${stats.nacked}, 메움 ${stats.filled}, 손실 ${stats.losses}",
            )
            frames = 0
            keyframes = 0
            dropped = 0
            rendered = 0
        }
    }

    fun release() {
        handler.post {
            released = true
            decoder?.release()
            decoder = null
            thread.quitSafely()
        }
    }

    private fun decode(unit: AccessUnit) {
        if (released) return
        frames += 1
        if (unit.keyframe) keyframes += 1

        val current = decoder

        if (current == null) {
            dropped += 1
            return
        }

        if (awaitingKeyframe && !unit.keyframe) {
            dropped += 1
            if (System.nanoTime() - requestedAt >= KEYFRAME_RETRY_NS) askKeyframe()
            return
        }

        awaitingKeyframe = false

        // RTP 타임스탬프는 90 kHz입니다. 출력 순서에만 쓰므로 넘어감은 신경 쓰지 않습니다.
        val ptsUs = unit.timestamp.toLong() * 1_000 / 90

        if (!current.queue(unit, ptsUs)) {
            Log.w(TAG, "디코더 입력이 밀려 쌓인 프레임을 버립니다")
            dropped += Decoder.BACKLOG + 1
            awaitingKeyframe = true
            askKeyframe()
        }
    }

    // 디코더를 지금의 설정과 Surface로 다시 만듭니다. 둘 중 하나가 없으면 디코더 없이 기다립니다.
    private fun rebuild(keyframe: Boolean) {
        decoder?.release()
        decoder = null
        awaitingKeyframe = true
        shown = false

        if (released) return

        val target = surface?.takeIf { it.isValid } ?: return
        val sps = sps ?: return
        val pps = pps ?: return

        decoder = runCatching { Decoder(sps, pps, target, handler, listener) }
            .onSuccess { Log.i(TAG, "디코더: ${it.info.name}") }
            .onFailure { Log.e(TAG, "디코더를 만들 수 없습니다", it) }
            .getOrNull()

        if (keyframe) {
            askKeyframe()
        }
    }

    private fun askKeyframe() {
        requestedAt = System.nanoTime()
        requestKeyframe()
    }

    companion object {

        // Surface가 사라질 때 디코더 해제를 기다리는 한도. 디코더 해제는 보통 수십 ms 안에 끝납니다.
        private const val DETACH_TIMEOUT_MS = 500L

        // 키프레임을 기다리며 다시 요청하는 간격. 세션도 요청 사이에 최소 간격을 둡니다.
        private const val KEYFRAME_RETRY_NS = 1_000_000_000L
    }
}
