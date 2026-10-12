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
import dev.insd.naru.video.Video

/**
 * 세션 하나를 열고, 코어가 밀어 주는 사건을 영상, 소리, 화면 쪽으로 나눠 보냅니다.
 *
 * - 사건은 세션의 사건 스레드에서 옵니다. 영상은 [Video]가 자기 스레드로 넘기고, 소리는 이 스레드에서 바로 재생합니다.
 * - 영상과 소리를 뺀 사건은 [listener]로 넘깁니다. 이 스레드에서 부르므로 UI 쪽에서 메인 스레드로 넘깁니다.
 * - 세션이 끝난 뒤에 보낸 입력은 코어가 버립니다.
 */
class Connection(config: Config, private val listener: Listener) {
    // 세션은 연결하면서 사건 스레드를 시작하므로, 사건을 받을 영상과 소리가 먼저 있어야 합니다.
    // 영상이 부르는 세션은 Surface가 붙은 뒤에야 쓰이므로 아직 없을 때 불리지 않습니다.
    private val video = Video(requestKeyframe = { session.requestKeyframe() }, onFirstFrame = { listener.onFirstFrame() })
    private var audio: Player? = null
    private val session = Session.connect(config, Events())

    interface Listener {
        fun onEvent(event: Event)

        /** 디코더를 새로 만든 뒤 첫 화면을 그렸습니다. 로딩 표시를 거둘 때 씁니다. 영상 스레드에서 부릅니다. */
        fun onFirstFrame()
    }

    /** 영상을 그릴 Surface. 사라지면 `null`을 넘깁니다. */
    fun attach(next: Surface?) = video.attach(next)

    fun key(keysym: Int, down: Boolean) = session.key(keysym.toUInt(), down)

    fun pointer(buttons: Int, x: Int, y: Int) = session.pointer(buttons.toUByte(), x.toUShort(), y.toUShort())

    fun scroll(phase: Phase, dx: Float, dy: Float) = session.scroll(phase, dx, dy)

    fun wheel(dx: Float, dy: Float) = session.wheel(dx, dy)

    fun magnify(phase: Phase, scale: Float) = session.magnify(phase, scale)

    /** 보냈으면 `true`. 크기 한도를 넘으면 보내지 않고 `false`입니다. */
    fun clipboard(text: String): Boolean = session.clipboard(text)

    fun language(language: String) = session.language(language)

    fun resize(mode: DisplayMode) = session.resize(mode)

    fun disconnect() = session.disconnect()

    // 영상과 소리는 초당 수십 번 오므로 기록하지 않고 화면 쪽에도 넘기지 않습니다.
    // 오디오 코덱이 예외를 던지면 다음 프레임에서도 같은 예외가 나므로 소리를 끕니다. 영상과 나머지 사건은 계속 받습니다.
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
            Log.e(TAG, "소리를 재생하지 못해 끕니다", error)
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

        Log.i(TAG, describe(event))
        listener.onEvent(event)

        if (event is Event.Closed) {
            video.release()
            audio?.release()
            audio = null
        }
    }

    // 소리를 낼 수 없으면 영상만으로 계속합니다.
    private fun startAudio(config: ByteArray) {
        audio?.release()
        audio = runCatching { Player(config) }
            .onFailure { Log.e(TAG, "오디오 디코더를 만들 수 없습니다", it) }
            .getOrNull()
    }

    // 로그에는 사건의 종류와 크기만 남깁니다. 클립보드 내용이나 키 바이트는 남기지 않습니다.
    private fun describe(event: Event): String = when (event) {
        is Event.Progress -> "단계: ${event.stage}"
        is Event.ServerKey -> "서버 키 ${event.der.size}바이트"
        is Event.Authenticated -> "인증 성공 (서버 증명 ${event.serverProof})"
        is Event.Layout -> "레이아웃: 논리 ${event.logicalWidth}×${event.logicalHeight}, 픽셀 ${event.pixelWidth}×${event.pixelHeight}"
        is Event.Decoder -> "디코더 설정: SPS ${event.config.sps.size}개, PPS ${event.config.pps.size}개"
        is Event.Cursor -> "커서 ${event.width}×${event.height}"
        is Event.ServerName -> "Mac 이름: ${event.name}"
        is Event.Source -> "입력 소스: ${event.source.id} (보안 입력 ${event.source.secure})"
        is Event.Clipboard -> "클립보드 ${event.text.length}자"
        is Event.ResetDecoder -> "디코더 재생성 요청"
        is Event.Closed -> "세션 종료: ${event.detail ?: "정상"}"
        is Event.AudioConfig -> "오디오 설정 ${event.config.size}바이트, 디코더 ${if (audio != null) "있음" else "없음"}"
        else -> event.javaClass.simpleName
    }

}
