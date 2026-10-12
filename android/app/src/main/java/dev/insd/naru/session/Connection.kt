package dev.insd.naru.session

import android.util.Log
import android.view.Surface
import dev.insd.naru.TAG
import dev.insd.naru.audio.Player
import dev.insd.naru.core.Config
import dev.insd.naru.core.DisplayMode
import dev.insd.naru.core.Event
import dev.insd.naru.core.Phase
import dev.insd.naru.core.Session
import dev.insd.naru.core.SessionListener
import dev.insd.naru.core.TouchPhase
import dev.insd.naru.video.Video

/**
 * Opens one session and routes the events the core pushes to video, audio, and the UI.
 *
 * - Events arrive on the session's event thread. [Video] hands video to its own thread, and audio plays directly on this one.
 * - Events other than video and audio go to [listener], called on this thread, so the UI side posts them to the main thread.
 * - The core discards input sent after the session ends.
 */
class Connection(config: Config, private val listener: Listener) {
    // The session starts its event thread while connecting, so video and audio must exist first to receive events.
    // Video uses the session only after a Surface attaches, so it is never called before the session exists.
    private val video = Video(requestKeyframe = { session.requestKeyframe() }, onFirstFrame = { listener.onFirstFrame() })
    private var audio: Player? = null
    private val session = Session.connect(config, Events())

    interface Listener {
        fun onEvent(event: Event)

        /** The first frame was drawn after a new decoder was created, used to hide the loading indicator. Called on the video thread. */
        fun onFirstFrame()
    }

    /** Surface to draw video on, or `null` once it is gone */
    fun attach(next: Surface?) = video.attach(next)

    fun key(keysym: Int, down: Boolean) = session.key(keysym.toUInt(), down)

    fun pointer(buttons: Int, x: Int, y: Int) = session.pointer(buttons.toUByte(), x.toUShort(), y.toUShort())

    fun scroll(phase: Phase, dx: Float, dy: Float) = session.scroll(phase, dx, dy)

    fun wheel(dx: Float, dy: Float) = session.wheel(dx, dy)

    fun magnify(phase: Phase, scale: Float) = session.magnify(phase, scale)

    fun touch(id: Int, phase: TouchPhase, x: Float, y: Float) = session.touch(id.toUInt(), phase, x, y)

    /** `true` if sent; `false` without sending if it exceeds the size limit */
    fun clipboard(text: String): Boolean = session.clipboard(text)

    fun language(language: String) = session.language(language)

    fun resize(mode: DisplayMode) = session.resize(mode)

    fun disconnect() = session.disconnect()

    // Video and audio arrive dozens of times per second, so they are neither logged nor passed to the UI.
    // If the audio codec throws, the next frame throws the same way, so audio is turned off. Video and other events keep flowing.
    private inner class Events : SessionListener {
        override fun onEvent(event: Event) {
            when (event) {
                is Event.Frame -> video.queue(event.unit)
                is Event.Audio -> play(event)
                is Event.Stats -> video.report(event.stats)
                else -> announce(event)
            }
        }
    }

    private fun play(event: Event.Audio) {
        try {
            audio?.play(event.frame)
        } catch (error: RuntimeException) {
            Log.e(TAG, "Audio playback failed; turning audio off", error)
            audio?.release()
            audio = null
        }
    }

    private fun announce(event: Event) {
        when (event) {
            is Event.Decoder -> video.configure(event.config)
            is Event.ResetDecoder -> video.reset()
            is Event.AudioConfig -> startAudio(event.config)
            else -> Unit
        }

        // The touch cursor position arrives on every finger movement, so it is not logged.
        if (event !is Event.Pointer) Log.i(TAG, describe(event))
        listener.onEvent(event)

        if (event is Event.Closed) {
            video.release()
            audio?.release()
            audio = null
        }
    }

    // Without audio, the session continues with video only.
    private fun startAudio(config: ByteArray) {
        audio?.release()
        audio = runCatching { Player(config) }
            .onFailure { Log.e(TAG, "Cannot create the audio decoder", it) }
            .getOrNull()
    }

    // Logs record only the event kind and size, never clipboard content or key bytes.
    private fun describe(event: Event): String = when (event) {
        is Event.Progress -> "Stage: ${event.stage}"
        is Event.ServerKey -> "Server key, ${event.der.size} bytes"
        is Event.Authenticated -> "Authenticated (server proof ${event.serverProof})"
        is Event.Layout -> "Layout: logical ${event.logicalWidth}×${event.logicalHeight}, pixels ${event.pixelWidth}×${event.pixelHeight}"
        is Event.Decoder -> "Decoder config: ${event.config.sps.size} SPS, ${event.config.pps.size} PPS"
        is Event.Cursor -> "Cursor ${event.width}×${event.height}"
        is Event.ServerName -> "Mac name: ${event.name}"
        is Event.Source -> "Input source: ${event.source.id} (secure input ${event.source.secure})"
        is Event.Clipboard -> "Clipboard, ${event.text.length} chars"
        is Event.ResetDecoder -> "Decoder reset requested"
        is Event.Closed -> "Session closed: ${event.detail ?: "normally"}"
        is Event.AudioConfig -> "Audio config, ${event.config.size} bytes, decoder ${if (audio != null) "available" else "unavailable"}"
        else -> event.javaClass.simpleName
    }

}
