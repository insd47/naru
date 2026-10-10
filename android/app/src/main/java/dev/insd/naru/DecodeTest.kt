package dev.insd.naru

import android.media.MediaCodec
import android.media.MediaFormat
import android.util.Log
import android.view.Surface
import dev.insd.naru.video.AccessUnit
import dev.insd.naru.video.Decoder
import dev.insd.naru.video.Sps
import dev.insd.naru.video.type
import java.io.File

/**
 * 캡처한 H.264 파일을 하드웨어 디코더로 재생하며, 서버 스트림이 디코더에서 겪을 수 있는 문제를 기록합니다.
 *
 * - IDR이 아닌 키프레임: 키프레임의 종류를 세고, I 슬라이스 키프레임 뒤에 출력이 나오는지 봅니다.
 * - 참조 프레임 초과: SPS의 참조 수를 기록합니다. 번짐은 화면으로 확인합니다.
 * - POC 넘김: IDR 뒤 POC 하위 비트가 넘어가는 지점을 지나도 출력이 이어지는지 봅니다.
 * - SSRC 교체: 새 SPS가 오면 디코더를 새로 만들고 출력이 다시 나오는지 봅니다.
 *
 * 결과는 화면과 logcat(`NaruM3`), 그리고 캡처 옆의 `<캡처>.report.txt`에 남깁니다.
 */
class DecodeTest(
    private val file: File,
    private val surface: Surface,
    private val realtime: Boolean,
    private val onLine: (String) -> Unit,
) {
    private val report = StringBuilder()

    private var decoder: Decoder? = null
    private var queued = 0
    private var rendered = 0
    private var errors = 0
    private var rebuilds = 0
    private var framesSinceIdr = 0
    private var pocWraps = 0

    // 디코더를 새로 만든 뒤에는 키프레임이 올 때까지 넣지 않습니다. 참조가 없는 프레임은 오류만 되풀이합니다.
    private var awaitingKeyframe = true
    private val kinds = mutableMapOf<AccessUnit.Kind, Int>()
    private val latenciesUs = mutableListOf<Long>()

    fun run() {
        log("파일: ${file.name} (${file.length() / 1024} KiB)")

        val units = AccessUnit.split(file.readBytes())
        log("접근 단위 ${units.size}개")

        val hardware = Decoder.hardwareDecoder()
        log("하드웨어 디코더: ${hardware?.name ?: "없음"}")

        if (hardware == null) {
            finish()
            return
        }

        val startedNs = System.nanoTime()
        var sps: ByteArray? = null
        var pps: ByteArray? = null

        for ((index, unit) in units.withIndex()) {
            if (Thread.currentThread().isInterrupted) {
                break
            }

            unit.nals.firstOrNull { it.type == AccessUnit.NAL_SPS }?.let { next ->
                pps = unit.nals.firstOrNull { it.type == AccessUnit.NAL_PPS } ?: pps

                if (sps?.contentEquals(next) != true) {
                    sps = next
                    rebuild(next, pps ?: return@let)
                }
            }

            val current = decoder ?: continue
            count(unit, current.config)

            val keyframe = unit.kind == AccessUnit.Kind.IDR || unit.kind == AccessUnit.Kind.INTRA

            if (awaitingKeyframe && !keyframe) {
                continue
            }

            awaitingKeyframe = false

            val ptsUs = index * FRAME_US

            if (realtime) {
                val dueNs = startedNs + ptsUs * 1_000
                val waitMs = (dueNs - System.nanoTime()) / 1_000_000
                if (waitMs > 0) Thread.sleep(waitMs)
            }

            try {
                while (!current.queue(unit, ptsUs)) {
                    drain(current)
                }

                queued += 1
                drain(current)
            } catch (error: MediaCodec.CodecException) {
                errors += 1
                log("디코더 오류(${unit.kind}, ${index}번째): ${error.diagnosticInfo}, 복구 가능 ${error.isRecoverable}, 일시적 ${error.isTransient}")
                rebuild(sps ?: break, pps ?: break)
            } catch (error: IllegalStateException) {
                errors += 1
                log("디코더 상태 오류(${index}번째): ${error.message}")
                rebuild(sps ?: break, pps ?: break)
            }

            if ((index + 1) % PROGRESS_INTERVAL == 0) {
                log("진행: 넣음 $queued, 나옴 $rendered, 지연 ${latencySummary()}")
            }
        }

        // 디코더에 남은 프레임을 마저 꺼냅니다.
        decoder?.let { current ->
            repeat(20) {
                drain(current)
                Thread.sleep(10)
            }
        }

        finish()
    }

    private fun rebuild(sps: ByteArray, pps: ByteArray) {
        decoder?.release()

        val next = Decoder(sps, pps, surface)
        val config = next.config
        decoder = next
        awaitingKeyframe = true
        rebuilds += 1
        framesSinceIdr = 0

        log(
            "디코더 생성(${rebuilds}번째): ${config.width}×${config.height}, 프로파일 ${config.profile}, 레벨 ${config.level}, " +
                "참조 ${config.maxRefFrames}장, POC 하위 비트 주기 ${config.maxPocLsb ?: "없음"}, 저지연 ${next.lowLatency}",
        )
    }

    private fun count(unit: AccessUnit, config: Sps) {
        kinds[unit.kind] = (kinds[unit.kind] ?: 0) + 1

        when (unit.kind) {
            AccessUnit.Kind.IDR -> framesSinceIdr = 0
            AccessUnit.Kind.INTRA -> log("IDR이 아닌 키프레임(${queued}번째 입력)")
            else -> framesSinceIdr += 1
        }

        // 보통 프레임마다 POC가 2씩 늘어나므로, 하위 비트 주기의 절반 프레임마다 넘김이 일어납니다.
        val wrapEvery = config.maxPocLsb?.let { it / 2 } ?: return

        if (framesSinceIdr > 0 && framesSinceIdr % wrapEvery == 0) {
            pocWraps += 1
            log("POC 넘김 지점 통과(IDR 뒤 ${framesSinceIdr}프레임). 이 시점의 출력 ${rendered}개")
        }
    }

    private fun drain(current: Decoder) {
        current.drain(
            onFrame = { frame ->
                rendered += 1
                frame.latencyUs?.let { latenciesUs += it }
            },
            onFormat = { format -> log("출력 형식: ${describe(format)}") },
        )
    }

    private fun finish() {
        decoder?.release()
        decoder = null

        log("결과: 넣음 $queued, 나옴 $rendered, 디코더 생성 ${rebuilds}회, 오류 ${errors}회, POC 넘김 ${pocWraps}회")
        log("키프레임: IDR ${kinds[AccessUnit.Kind.IDR] ?: 0}, I 슬라이스 ${kinds[AccessUnit.Kind.INTRA] ?: 0}, 나머지 ${kinds[AccessUnit.Kind.INTER] ?: 0}")
        log("지연: ${latencySummary()}")

        val output = File(file.parentFile, "${file.name}.report.txt")
        output.writeText(report.toString())
        log("보고서: ${output.absolutePath}")
    }

    private fun latencySummary(): String {
        if (latenciesUs.isEmpty()) {
            return "없음"
        }

        val sorted = latenciesUs.sorted()
        val percentile = { ratio: Double -> sorted[((sorted.size - 1) * ratio).toInt()] / 1000.0 }
        return "중앙값 %.1fms, p99 %.1fms, 최대 %.1fms".format(percentile(0.5), percentile(0.99), sorted.last() / 1000.0)
    }

    private fun describe(format: MediaFormat): String {
        val keys = listOf(MediaFormat.KEY_WIDTH, MediaFormat.KEY_HEIGHT, MediaFormat.KEY_COLOR_FORMAT, MediaFormat.KEY_COLOR_RANGE)
        return keys.filter { format.containsKey(it) }.joinToString { "$it=${format.getInteger(it)}" }
    }

    private fun log(line: String) {
        Log.i(TAG, line)
        report.appendLine(line)
        onLine(line)
    }

    companion object {
        const val TAG = "NaruM3"

        // 캡처에는 RTP 타임스탬프가 없으므로 60fps로 간주합니다.
        private const val FRAME_US = 16_667L
        private const val PROGRESS_INTERVAL = 600
    }
}
