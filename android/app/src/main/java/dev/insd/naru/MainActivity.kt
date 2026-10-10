package dev.insd.naru

import android.os.Bundle
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import java.io.File

/**
 * M3 디코딩 시험 화면입니다. 앱 전용 외부 저장소의 캡처를 하드웨어 디코더로 재생합니다.
 *
 * - 파일은 인텐트의 `file`(기본값 `capture.h264`)로 고르고, `realtime`이 `false`면 60fps로 맞추지 않고 최대한 빨리 넣습니다.
 * - 앱 전용 외부 저장소(`adb push`)를 먼저 찾고, 없으면 내부 저장소(`adb shell run-as`로 복사)를 찾습니다.
 */
class MainActivity : ComponentActivity() {
    private val lines = mutableStateListOf<String>()
    private var worker: Thread? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        val name = intent.getStringExtra("file") ?: "capture.h264"
        val realtime = intent.getBooleanExtra("realtime", true)
        val file = listOfNotNull(getExternalFilesDir(null), filesDir).map { File(it, name) }.firstOrNull { it.exists() }
            ?: File(getExternalFilesDir(null), name)

        setContent {
            Box(Modifier.fillMaxSize().background(Color.Black)) {
                AndroidView(
                    modifier = Modifier.fillMaxSize(),
                    factory = { context ->
                        SurfaceView(context).apply {
                            holder.addCallback(object : SurfaceHolder.Callback {
                                override fun surfaceCreated(holder: SurfaceHolder) = start(file, holder, realtime)
                                override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) = Unit
                                override fun surfaceDestroyed(holder: SurfaceHolder) = stop()
                            })
                        }
                    },
                )

                Column(Modifier.align(Alignment.TopStart).background(Color(0x99000000)).padding(12.dp)) {
                    for (line in lines.takeLast(VISIBLE_LINES)) {
                        Text(line, color = Color.White, fontSize = 13.sp)
                    }
                }
            }
        }
    }

    private fun start(file: File, holder: SurfaceHolder, realtime: Boolean) {
        if (!file.exists()) {
            lines += "캡처가 없습니다: ${file.absolutePath}"
            return
        }

        val test = DecodeTest(file, holder.surface, realtime) { line -> runOnUiThread { lines += line } }
        worker = Thread({ test.run() }, "decode-test").apply { start() }
    }

    // Surface가 사라지면 디코더도 더 그릴 곳이 없으므로 시험을 멈춥니다.
    private fun stop() {
        worker?.interrupt()
        worker?.join(1_000)
        worker = null
    }

    companion object {
        private const val VISIBLE_LINES = 24
    }
}
