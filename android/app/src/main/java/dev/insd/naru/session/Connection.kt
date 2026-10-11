package dev.insd.naru.session

import android.media.MediaCodec
import android.util.Log
import android.view.Surface
import dev.insd.naru.audio.Player
import dev.insd.naru.protocol.Config
import dev.insd.naru.protocol.Event
import dev.insd.naru.protocol.Language
import dev.insd.naru.protocol.Phase
import dev.insd.naru.protocol.Session
import dev.insd.naru.video.Decoder
import java.util.concurrent.atomic.AtomicReference

/**
 * 세션 하나를 열고, 전용 스레드에서 코어의 사건을 꺼내 디코더에 넣습니다.
 *
 * - 디코더는 이 스레드만 만지고, 화면 쪽은 [attach]로 Surface만 넘깁니다.
 * - 사건을 짧은 시간 제한으로 꺼내므로, 화면이 멈춰 프레임이 오지 않아도 디코더의 남은 출력이 그려집니다.
 * - UI에 알릴 일은 [listener]로 부릅니다. 이 스레드에서 부르므로 UI 쪽에서 메인 스레드로 넘깁니다.
 */
class Connection(config: Config, private val listener: Listener) {
    private val session = Session.connect(config)
    private val surface = AtomicReference<Surface?>()
    private val thread = Thread(::run, "naru-session")

    @Volatile private var surfaceChanged = false

    private var decoder: Decoder? = null
    private var audio: Player? = null
    private var sps: ByteArray? = null
    private var pps: ByteArray? = null

    // 1초마다 남기는 수신 기록입니다.
    private var frames = 0
    private var keyframes = 0
    private var dropped = 0
    private var rendered = 0
    private var lastStats: Event.Stats? = null
    private var reportedAt = System.nanoTime()

    private var shown = false

    // 디코더를 새로 만든 뒤에는 키프레임이 올 때까지 넣지 않습니다.
    private var awaitingKeyframe = true

    interface Listener {
        fun onEvent(event: Event)

        /** 첫 화면이 그려졌습니다. 접속 중 표시를 거둘 때 씁니다. */
        fun onFirstFrame()
    }

    init {
        thread.start()
    }

    /** 영상을 그릴 Surface. 사라지면 `null`을 넘깁니다. */
    fun attach(next: Surface?) {
        Log.i(TAG, "Surface ${if (next == null) "사라짐" else "연결"}")
        surface.set(next)
        surfaceChanged = true
    }

    // 세션 스레드가 끝나며 네이티브 객체를 해제한 뒤에 들어온 입력은 버립니다.
    fun key(keysym: Int, down: Boolean) {
        runCatching { session.key(keysym.toUInt(), down) }
    }

    fun pointer(buttons: Int, x: Int, y: Int) {
        runCatching { session.pointer(buttons.toUByte(), x.toUShort(), y.toUShort()) }
    }

    fun scroll(phase: Phase, dx: Float, dy: Float) {
        runCatching { session.scroll(phase, dx, dy) }
    }

    fun wheel(dx: Float, dy: Float) {
        runCatching { session.wheel(dx, dy) }
    }

    fun magnify(phase: Phase, scale: Float) {
        runCatching { session.magnify(phase, scale) }
    }

    fun clipboard(text: String) {
        runCatching { session.clipboard(text) }
    }

    fun language(language: Language) {
        runCatching { session.language(language) }
    }

    fun disconnect() {
        runCatching { session.disconnect() }
    }

    private fun run() {
        try {
            while (true) {
                if (surfaceChanged) {
                    surfaceChanged = false
                    rebuild()
                }

                val event = session.nextEvent(POLL_MS)
                decoder?.let(::drain)
                report()

                event ?: continue

                when (event) {
                    is Event.Decoder -> configure(event)
                    is Event.Frame -> queue(event)
                    is Event.ResetDecoder -> rebuild()
                    is Event.AudioConfig -> startAudio(event.config)
                    is Event.Audio -> audio?.play(event.frame)
                    else -> Unit
                }

                if (event is Event.Stats) {
                    lastStats = event
                    continue
                }

                // 영상과 소리는 초당 수십 번 오므로 기록하지 않고 화면 쪽에도 넘기지 않습니다.
                if (event !is Event.Frame && event !is Event.Audio) {
                    Log.i(TAG, describe(event))
                    listener.onEvent(event)
                }

                if (event is Event.Closed) {
                    break
                }
            }
        } finally {
            decoder?.release()
            decoder = null
            audio?.release()
            audio = null
            session.close()
        }
    }

    // 서버는 키프레임마다 같은 설정을 다시 보냅니다. 같은 설정이면 디코더를 그대로 두고, 바뀌었을 때만 새로 만듭니다.
    // 새로 만들 때는 뒤따르는 키프레임이 곧 오므로 따로 요청하지 않습니다.
    private fun configure(event: Event.Decoder) {
        val nextSps = event.config.sps.firstOrNull() ?: return
        val nextPps = event.config.pps.firstOrNull() ?: return

        if (decoder != null && sps.contentEquals(nextSps) && pps.contentEquals(nextPps)) {
            return
        }

        sps = nextSps
        pps = nextPps
        rebuild(requestKeyframe = false)
    }

    private fun firstFrame() {
        if (!shown) {
            shown = true
            listener.onFirstFrame()
        }
    }

    // 소리를 낼 수 없으면 영상만으로 계속합니다.
    private fun startAudio(config: ByteArray) {
        audio?.release()
        audio = runCatching { Player(config) }
            .onFailure { Log.e(TAG, "오디오 디코더를 만들 수 없습니다", it) }
            .getOrNull()
    }

    private fun report() {
        val now = System.nanoTime()

        if (now - reportedAt < 1_000_000_000L) {
            return
        }

        reportedAt = now
        val stats = lastStats?.stats?.let {
            "데이터그램 ${it.datagrams}, 인증 ${it.authenticated}, SSRC ${it.ssrc?.toString(16)}, 순번 ${it.highest}, 후보 ${it.candidates}, 오퍼 ${it.offers}, NACK ${it.nacked}, 메움 ${it.filled}, 손실 ${it.losses}"
        }
        Log.i(TAG, "1초: 프레임 $frames (키 $keyframes, 버림 $dropped), 출력 $rendered | $stats")
        frames = 0
        keyframes = 0
        dropped = 0
        rendered = 0
    }

    private fun queue(frame: Event.Frame) {
        frames += 1
        if (frame.unit.keyframe) keyframes += 1

        val current = decoder ?: run { dropped += 1; return }

        if (awaitingKeyframe && !frame.unit.keyframe) {
            dropped += 1
            return
        }

        awaitingKeyframe = false

        // RTP 타임스탬프는 90 kHz입니다. 출력 순서와 지연 측정에만 쓰므로 넘어감은 신경 쓰지 않습니다.
        val ptsUs = frame.unit.timestamp.toLong() * 1_000 / 90

        try {
            if (!current.queue(frame.unit, ptsUs, INPUT_TIMEOUT_US)) {
                // 입력 버퍼가 없으면 이 프레임을 버리고 키프레임부터 다시 받습니다.
                Log.w(TAG, "디코더 입력 버퍼가 없어 프레임을 버립니다")
                awaitingKeyframe = true
                session.requestKeyframe()
            }
        } catch (error: MediaCodec.CodecException) {
            Log.w(TAG, "디코더 오류: ${error.diagnosticInfo}")
            rebuild()
        } catch (error: IllegalStateException) {
            Log.w(TAG, "디코더 상태 오류: ${error.message}")
            rebuild()
        }
    }

    private fun drain(current: Decoder) {
        try {
            current.drain(onFrame = { rendered += 1; firstFrame() }, onFormat = {})
        } catch (error: IllegalStateException) {
            Log.w(TAG, "디코더 출력 오류: ${error.message}")
            rebuild()
        }
    }

    // 디코더를 지금의 설정과 Surface로 다시 만들고 키프레임을 요청합니다. 둘 중 하나가 없으면 디코더 없이 기다립니다.
    private fun rebuild(requestKeyframe: Boolean = true) {
        decoder?.release()
        decoder = null
        awaitingKeyframe = true

        val target = surface.get()?.takeIf { it.isValid } ?: return
        val sps = sps ?: return
        val pps = pps ?: return

        decoder = runCatching { Decoder(sps, pps, target) }
            .onFailure { Log.e(TAG, "디코더를 만들 수 없습니다", it) }
            .getOrNull()

        if (requestKeyframe) {
            session.requestKeyframe()
        }
    }

    // 로그에는 사건의 종류와 크기만 남깁니다. 클립보드 내용이나 키 바이트는 남기지 않습니다.
    private fun describe(event: Event): String = when (event) {
        is Event.Progress -> "단계: ${event.stage}"
        is Event.ServerKey -> "서버 키 ${event.der.size}바이트"
        is Event.Authenticated -> "인증 성공 (서버 증명 ${event.serverProof})"
        is Event.Layout -> "레이아웃: 논리 ${event.logicalWidth}×${event.logicalHeight}, 픽셀 ${event.pixelWidth}×${event.pixelHeight}"
        is Event.Decoder -> "디코더 설정: SPS ${event.config.sps.size}개, PPS ${event.config.pps.size}개, 디코더 ${decoder?.info?.name ?: "없음(Surface 대기)"}"
        is Event.Cursor -> "커서 ${event.width}×${event.height}"
        is Event.ServerName -> "Mac 이름: ${event.name}"
        is Event.Source -> "입력 소스: ${event.source.id} (보안 입력 ${event.source.secure})"
        is Event.Clipboard -> "클립보드 ${event.text.length}자"
        is Event.ResetDecoder -> "디코더 재생성 요청"
        is Event.Closed -> "세션 종료: ${event.detail ?: "정상"}"
        is Event.Stats -> "통계"
        is Event.AudioConfig -> "오디오 설정 ${event.config.size}바이트, 디코더 ${if (audio != null) "있음" else "없음"}"
        is Event.Audio -> "오디오"
        is Event.Frame -> "프레임"
    }

    companion object {
        private const val TAG = "Naru"
        private const val POLL_MS = 5u
        private const val INPUT_TIMEOUT_US = 20_000L
    }
}
