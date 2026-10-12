package dev.insd.naru

import android.content.ComponentName
import android.content.Intent
import android.content.ServiceConnection
import android.os.Bundle
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import dev.insd.naru.core.Event
import dev.insd.naru.session.Connection
import dev.insd.naru.session.ImeLanguage
import dev.insd.naru.session.Keys
import dev.insd.naru.session.MetaKey
import dev.insd.naru.session.ScreenView
import dev.insd.naru.session.SessionService
import dev.insd.naru.session.Touchpad

/**
 * 살아 있는 세션의 화면입니다. 화면과 입력만 다룹니다.
 *
 * - 접속과 로그인, 실패 안내는 목록 화면([MainActivity])이 합니다. 이 화면은 인증에 성공한 세션에만 열립니다.
 * - 세션은 [SessionService]가 소유합니다. 이 화면은 서비스에 붙어 Surface와 입력을 넘기므로, 화면을 떠나도 세션은 이어지고 돌아오면 다시 붙습니다.
 * - 몰입 모드, 키 가로채기, Meta 키 요청, 화면 켜짐 유지는 이 창에만 걸리므로 화면을 떠나면 함께 풀립니다.
 * - 세션 중에는 하드웨어 키보드의 키를 모두 Mac으로 보냅니다. 뒤로 가기는 세션을 둔 채 목록으로 돌아갑니다.
 * - 가상 디스플레이는 영상을 그리는 영역의 실제 크기에 맞춥니다. 화면 회전, 창 크기 변경, 숨길 수 없는 시스템 막대가 모두 이 크기에 드러납니다.
 */
class SessionActivity : ComponentActivity() {
    private var service: SessionService? = null
    private var connection by mutableStateOf<Connection?>(null)
    private lateinit var keys: Keys
    private var language: ImeLanguage? = null
    private var touchpad: Touchpad? = null

    private val listener = object : Connection.Listener {
        override fun onEvent(event: Event) = handle(event)
        override fun onFirstFrame() {
            loading = false
        }
    }

    // 인증 전(접속 중에 알림을 누른 경우)이거나 이미 끝난 세션이면 보여 줄 것이 없으므로 목록 화면으로 돌아갑니다. 로그인과 실패는 목록 화면이 맡습니다.
    private val binding = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) {
            val bound = (binder as SessionService.Local).service
            service = bound

            val connection = bound.connection?.takeIf { bound.state is SessionService.State.Connected } ?: return home()
            adopt(connection)
            bound.attach(listener)
        }

        override fun onServiceDisconnected(name: ComponentName) {
            service = null
        }
    }

    private var loading by mutableStateOf(true)
    private var layout by mutableStateOf<Event.Layout?>(null)
    private var screen: ScreenView? = null

    // 화면 뷰는 연결을 붙인 뒤 다음 프레임에 만들어지므로, 그 전에 온 커서는 남겨 두었다가 붙일 때 씁니다.
    private var cursor: Event.Cursor? = null

    // 크기를 바꿀 때마다 Mac은 새 스트림을 열어 1.5~2초 걸리므로, 창을 끄는 동안의 변경은 멈출 때까지 모았다가 한 번만 보냅니다.
    private val main = Handler(Looper.getMainLooper())
    private var area: IntSize? = null
    private val resize = Runnable { area?.let { service?.resize(virtualDisplay(it.width, it.height)) } }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        keys = Keys(swapCommandOption = naru.preferences.swapCommandOption)

        setContent {
            NaruTheme(dark = true) {
                val measured = Modifier.onSizeChanged {
                    area = it
                    main.removeCallbacks(resize)
                    main.postDelayed(resize, RESIZE_SETTLE_MS)
                }

                Box(Modifier.fillMaxSize().background(Color.Black).then(measured), contentAlignment = Alignment.Center) {
                    if (connection != null) Screen()
                }
            }
        }

        bindService(Intent(this, SessionService::class.java), binding, BIND_AUTO_CREATE)
    }

    // 뒤에 남아 있던 화면이 다른 Mac과의 새 세션으로 바뀐 뒤 다시 보이면 닫습니다.
    override fun onStart() {
        super.onStart()
        val current = service ?: return
        if (connection != null && current.connection !== connection) finish()
    }

    // 화면을 떠날 때 Mac에 눌린 채 남는 키가 없게 합니다.
    // 돌아오면 Surface와 디코더를 새로 만들어 첫 프레임까지 비어 있으므로 로딩 표시를 다시 띄웁니다.
    override fun onStop() {
        loading = true
        connection?.let { current -> keys.releaseAll().forEach { (keysym, down) -> current.key(keysym, down) } }
        super.onStop()
    }

    // 화면만 사라집니다. 세션은 서비스에 남습니다.
    override fun onDestroy() {
        main.removeCallbacks(resize)
        language?.stop()
        service?.detach(listener)
        unbindService(binding)
        super.onDestroy()
    }

    // 세션 중에는 키를 시스템이나 Compose보다 먼저 가로챕니다. 포커스 이동(Tab, 화살표)이 일어나지 않게 합니다.
    override fun dispatchKeyEvent(event: KeyEvent): Boolean = handleKey(event, afterIme = true) || super.dispatchKeyEvent(event)

    // 터치패드 제스처는 Compose가 중간에 취소를 끼워 넣으므로 그보다 먼저 가로챕니다.
    override fun dispatchTouchEvent(event: MotionEvent): Boolean =
        touchpad?.handle(event) == true || super.dispatchTouchEvent(event)

    // 다른 앱에서 복사하고 돌아오면 그 텍스트를 Mac에 보냅니다.
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) service?.syncClipboard()
    }

    // 세션 중이면 키를 Mac으로 보내고 `true`를 반환합니다. 화면 뷰가 IME보다 먼저 부르고, 포커스가 다른 곳이면 액티비티가 부릅니다.
    private fun handleKey(event: KeyEvent, afterIme: Boolean = false): Boolean {
        val current = connection ?: return false
        val translated = keys.translate(event)

        for ((keysym, down) in translated) {
            current.key(keysym, down)
        }

        // Mac으로 보내지 않는 키는 시스템과 IME에 그대로 넘깁니다. 한/영 키는 IME가 언어를 바꾸고, 볼륨 키는 볼륨을, 뒤로 가기는 화면을 바꿉니다.
        if (translated.isEmpty() && !KeyEvent.isModifierKey(event.keyCode) && !keys.isMouseNavigation(event)) {
            // 화면 뷰(IME 전)와 액티비티(IME 후)가 같은 키를 두 번 볼 수 있으므로 IME 뒤에서만 언어를 다시 읽습니다.
            if (event.keyCode == KeyEvent.KEYCODE_LANGUAGE_SWITCH && event.action == KeyEvent.ACTION_UP && afterIme) {
                language?.sync()
            }

            return false
        }

        return true
    }

    @Composable
    private fun Screen() {
        val current = layout
        val aspect = current?.let { it.pixelWidth.toFloat() / it.pixelHeight.toFloat() }

        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            AndroidView(
                modifier = if (aspect != null) Modifier.aspectRatio(aspect) else Modifier.fillMaxSize(),
                factory = { context -> ScreenView(context).also { screen = it; bind(it) } },
                update = { view ->
                    view.layoutSize = current?.let { it.pixelWidth.toInt() to it.pixelHeight.toInt() }
                    view.logicalWidth = current?.logicalWidth?.toInt()
                },
            )

            // 첫 화면이 그려질 때까지 가운데에 진행 표시를 둡니다.
            if (loading) {
                Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(24.dp)) {
                    @OptIn(ExperimentalMaterial3ExpressiveApi::class)
                    LoadingIndicator(Modifier.size(96.dp))
                    Text("Preparing screen…", color = Color.White, style = MaterialTheme.typography.titleMedium)
                }
            }
        }
    }

    private fun bind(view: ScreenView) {
        view.connection = connection
        cursor?.let(view::setCursor)
        view.onKey = { handleKey(it) }
        view.requestFocus()
    }

    // 세션의 연결에 이 화면의 입력을 붙입니다.
    private fun adopt(next: Connection) {
        connection = next
        language = ImeLanguage(this, next).also { it.sync(delayMs = 0) }
        touchpad = Touchpad(next, resources.displayMetrics.density)
        immersive(true)
    }

    private fun handle(event: Event) {
        when (event) {
            is Event.Layout -> layout = event
            is Event.Cursor -> {
                cursor = event
                screen?.setCursor(event)
            }
            is Event.Closed -> closed()
            else -> Unit
        }
    }

    private fun home() {
        startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP))
        finish()
    }

    // 남은 정리는 화면이 사라지며 합니다. 시스템 막대와 Meta 키 요청만 먼저 되돌립니다.
    private fun closed() {
        Log.i(TAG, "세션 화면을 닫습니다")
        immersive(false)
        finish()
    }

    private fun immersive(on: Boolean) {
        val controller = WindowCompat.getInsetsController(window, window.decorView)

        MetaKey.request(this, on)

        if (on) {
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            controller.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller.hide(WindowInsetsCompat.Type.systemBars())
        } else {
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            controller.show(WindowInsetsCompat.Type.systemBars())
        }
    }

    companion object {
        private const val RESIZE_SETTLE_MS = 500L
    }
}
