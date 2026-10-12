package dev.insd.naru.video

import android.media.MediaCodec
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.view.Surface
import dev.insd.naru.TAG
import dev.insd.naru.core.AccessUnit
import dev.insd.naru.core.DecoderConfig
import dev.insd.naru.core.MediaStats
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Draws the session's video to a Surface.
 *
 * - The decoder and its callbacks live on one dedicated thread. Calls from any thread are posted there and handled in order.
 * - A decoder exists only while both a config (SPS, PPS) and a Surface exist. When either changes, a new one is created and fed from a keyframe.
 * - A keyframe is requested through [requestKeyframe] when the Surface changed, the decoder failed, or input backed up. The session requests one itself for a new config and for frame stall recovery.
 * - Each new decoder calls [onFirstFrame] once it draws its first frame. The screen is blank until then.
 * - Work arriving after [release] is ignored.
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

    // After a new decoder is created, nothing is queued until a keyframe arrives.
    // The request may not have reached the server, so frames arriving while waiting trigger a new request at a fixed interval.
    private var awaitingKeyframe = true
    private var requestedAt = 0L

    // Receive counters logged on each stats event.
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

        // Recreated after the callback returns so the codec is not released inside its own callback.
        override fun onError(error: RuntimeException) {
            Log.w(TAG, "Decoder error: ${(error as? MediaCodec.CodecException)?.diagnosticInfo ?: error.message}")
            handler.post { rebuild(keyframe = true) }
        }
    }

    /**
     * Surface to draw video on, or `null` once it is gone.
     *
     * - Drawing to a Surface after it is gone is not allowed, so `null` blocks until the decoder is released.
     */
    fun attach(next: Surface?) {
        val detached = CountDownLatch(1)

        val posted = handler.post {
            if (next == null || next !== surface || decoder == null) {
                Log.i(TAG, "Surface ${if (next == null) "gone" else "attached"}")
                surface = next
                rebuild(keyframe = true)
            }

            detached.countDown()
        }

        if (next == null && posted) {
            detached.await(DETACH_TIMEOUT_MS, TimeUnit.MILLISECONDS)
        }
    }

    // The server resends the same config with every keyframe. The decoder is kept for an identical config and recreated only when it changes.
    // When recreating, the following keyframe arrives shortly, so none is requested.
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

    /** No frame has come out for a while, so the decoder is recreated. The session requests the keyframe. */
    fun reset() {
        handler.post { rebuild(keyframe = false) }
    }

    fun report(stats: MediaStats) {
        handler.post {
            Log.i(
                TAG,
                "Received (0.5 s): frames $frames (key $keyframes, dropped $dropped), rendered $rendered | datagrams ${stats.datagrams}, " +
                    "authenticated ${stats.authenticated}, SSRC ${stats.ssrc?.toString(16)}, sequence ${stats.highest}, candidates ${stats.candidates}, " +
                    "offers ${stats.offers}, NACK ${stats.nacked}, filled ${stats.filled}, lost ${stats.losses}",
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

        // RTP timestamps are 90 kHz. They only order output, so wraparound does not matter.
        val ptsUs = unit.timestamp.toLong() * 1_000 / 90

        if (!current.queue(unit, ptsUs)) {
            Log.w(TAG, "Decoder input backed up; dropping pending frames")
            dropped += Decoder.BACKLOG + 1
            awaitingKeyframe = true
            askKeyframe()
        }
    }

    // Recreates the decoder with the current config and Surface. If either is missing, waits without a decoder.
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
            .onSuccess { Log.i(TAG, "Decoder: ${it.info.name}") }
            .onFailure { Log.e(TAG, "Cannot create the decoder", it) }
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

        // How long to wait for the decoder release when the Surface goes away. A release usually finishes within tens of ms.
        private const val DETACH_TIMEOUT_MS = 500L

        // Interval between keyframe re-requests while waiting. The session also enforces a minimum interval between requests.
        private const val KEYFRAME_RETRY_NS = 1_000_000_000L
    }
}
