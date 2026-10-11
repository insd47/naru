package dev.insd.naru.session

import android.media.MediaCodec
import android.util.Log
import android.view.Surface
import dev.insd.naru.protocol.ConnectConfig
import dev.insd.naru.protocol.NaruSession
import dev.insd.naru.protocol.Phase
import dev.insd.naru.protocol.SessionEvent
import dev.insd.naru.video.AccessUnit
import dev.insd.naru.video.Decoder
import java.util.concurrent.atomic.AtomicReference

/**
 * 세션 하나를 열고, 전용 스레드에서 코어의 사건을 꺼내 디코더에 넣습니다.
 *
 * - 디코더는 이 스레드만 만지고, 화면 쪽은 [attach]로 Surface만 넘깁니다.
 * - 사건을 짧은 시간 제한으로 꺼내므로, 화면이 멈춰 프레임이 오지 않아도 디코더의 남은 출력이 그려집니다.
 * - UI에 알릴 일은 [listener]로 부릅니다. 이 스레드에서 부르므로 UI 쪽에서 메인 스레드로 넘깁니다.
 */
class Connection(config: ConnectConfig, private val listener: Listener) {
    private val session = NaruSession.connect(config)
    private val surface = AtomicReference<Surface?>()
    private val thread = Thread(::run, "naru-session")

    @Volatile private var surfaceChanged = false

    private var decoder: Decoder? = null
    private var sps: ByteArray? = null
    private var pps: ByteArray? = null

    // 1초마다 남기는 수신 기록입니다.
    private var frames = 0
    private var keyframes = 0
    private var dropped = 0
    private var rendered = 0
    private var lastStats: SessionEvent.Stats? = null
    private var reportedAt = System.nanoTime()

    private var shown = false

    // 디코더를 새로 만든 뒤에는 키프레임이 올 때까지 넣지 않습니다.
    private var awaitingKeyframe = true

    interface Listener {
        fun onEvent(event: SessionEvent)

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

    fun inputSource(id: String) {
        runCatching { session.inputSource(id) }
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
                    is SessionEvent.DecoderConfig -> configure(event)
                    is SessionEvent.Frame -> queue(event)
                    is SessionEvent.ResetDecoder -> rebuild()
                    else -> Unit
                }

                if (event is SessionEvent.Stats) {
                    lastStats = event
                    continue
                }

                if (event !is SessionEvent.Frame) {
                    Log.i(TAG, describe(event))
                    listener.onEvent(event)
                }

                if (event is SessionEvent.Closed) {
                    break
                }
            }
        } finally {
            decoder?.release()
            decoder = null
            session.close()
        }
    }

    // 서버는 키프레임마다 같은 설정을 다시 보냅니다. 같은 설정이면 디코더를 그대로 두고, 바뀌었을 때만 새로 만듭니다.
    // 새로 만들 때는 뒤따르는 키프레임이 곧 오므로 따로 요청하지 않습니다.
    private fun configure(config: SessionEvent.DecoderConfig) {
        val nextSps = config.sps.firstOrNull() ?: return
        val nextPps = config.pps.firstOrNull() ?: return

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

    private fun report() {
        val now = System.nanoTime()

        if (now - reportedAt < 1_000_000_000L) {
            return
        }

        reportedAt = now
        val stats = lastStats?.let {
            "데이터그램 ${it.datagrams}, 인증 ${it.authenticated}, SSRC ${it.ssrc?.toString(16)}, 순번 ${it.highest}, 후보 ${it.candidates}, 오퍼 ${it.offers}, NACK ${it.nacked}, 메움 ${it.filled}, 손실 ${it.losses}"
        }
        Log.i(TAG, "1초: 프레임 $frames (키 $keyframes, 버림 $dropped), 출력 $rendered | $stats")
        frames = 0
        keyframes = 0
        dropped = 0
        rendered = 0
    }

    private fun queue(frame: SessionEvent.Frame) {
        frames += 1
        if (frame.keyframe) keyframes += 1

        val current = decoder ?: run { dropped += 1; return }

        if (awaitingKeyframe && !frame.keyframe) {
            dropped += 1
            return
        }

        awaitingKeyframe = false

        // RTP 타임스탬프는 90 kHz입니다. 출력 순서와 지연 측정에만 쓰므로 넘어감은 신경 쓰지 않습니다.
        val ptsUs = frame.timestamp.toLong() * 1_000 / 90

        try {
            if (!current.queue(AccessUnit(frame.nals), ptsUs, INPUT_TIMEOUT_US)) {
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
    private fun describe(event: SessionEvent): String = when (event) {
        is SessionEvent.Progress -> "단계: ${event.stage}"
        is SessionEvent.ServerKey -> "서버 키 ${event.der.size}바이트"
        is SessionEvent.Authenticated -> "인증 성공 (서버 증명 ${event.serverProof})"
        is SessionEvent.Layout -> "레이아웃: 논리 ${event.logicalWidth}×${event.logicalHeight}, 픽셀 ${event.pixelWidth}×${event.pixelHeight}"
        is SessionEvent.DecoderConfig -> "디코더 설정: SPS ${event.sps.size}개, PPS ${event.pps.size}개, 디코더 ${decoder?.info?.name ?: "없음(Surface 대기)"}"
        is SessionEvent.Cursor -> "커서 ${event.width}×${event.height}"
        is SessionEvent.ServerName -> "Mac 이름: ${event.name}"
        is SessionEvent.InputSource -> "입력 소스: ${event.id}"
        is SessionEvent.Clipboard -> "클립보드 ${event.text.length}자"
        is SessionEvent.ResetDecoder -> "디코더 재생성 요청"
        is SessionEvent.Closed -> "세션 종료: ${event.detail ?: "정상"}"
        is SessionEvent.Stats -> "통계"
        is SessionEvent.Frame -> "프레임"
    }

    companion object {
        private const val TAG = "Naru"
        private const val POLL_MS = 5u
        private const val INPUT_TIMEOUT_US = 20_000L
    }
}
