package dev.insd.naru

import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
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
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedSecureTextField
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import dev.insd.naru.audio.Player
import dev.insd.naru.home.Host
import dev.insd.naru.home.Hosts
import dev.insd.naru.home.Keychain
import dev.insd.naru.protocol.Config
import dev.insd.naru.protocol.DisplayMode
import dev.insd.naru.protocol.Event
import dev.insd.naru.protocol.Failure
import dev.insd.naru.protocol.Stage
import dev.insd.naru.session.Clipboard
import dev.insd.naru.session.Connection
import dev.insd.naru.session.InputSource
import dev.insd.naru.session.Keys
import dev.insd.naru.session.MetaKey
import dev.insd.naru.session.ScreenView
import dev.insd.naru.session.Touchpad
import java.io.File

/**
 * Mac 한 대와의 세션 화면입니다. 비밀번호를 물은 뒤 접속하고, 화면과 입력을 처리합니다.
 *
 * - 몰입 모드, 키 가로채기, Meta 키 요청, 화면 켜짐 유지는 이 창에만 걸리므로 화면을 떠나면 함께 풀립니다.
 * - 저장한 비밀번호가 있으면 묻지 않고 접속합니다. 없거나 틀리면 사용자 이름과 비밀번호를 묻습니다.
 * - 인증에 성공하면 Mac을 목록에 저장하고, 원하면 비밀번호를 [Keychain]에 저장합니다.
 * - 서버 공개키는 처음 접속할 때 저장하고, 다음 접속부터 바뀌었는지 확인합니다.
 * - 세션 중에는 하드웨어 키보드의 키를 모두 Mac으로 보냅니다. 뒤로 가기만 세션을 닫습니다.
 */
class SessionActivity : ComponentActivity() {
    private lateinit var host: Host
    private lateinit var hosts: Hosts
    private lateinit var keychain: Keychain

    private var connection by mutableStateOf<Connection?>(null)
    private val keys = Keys()
    private var inputSource: InputSource? = null
    private var touchpad: Touchpad? = null
    private var clipboard: Clipboard? = null

    // 인증 결과를 기다리는 사용자 이름과 비밀번호. 인증에 성공하면 저장하고 지웁니다.
    private var pending: Credentials? = null

    private var status by mutableStateOf<String?>(null)
    private var failure by mutableStateOf<Event.Closed?>(null)
    private var loginMessage by mutableStateOf<String?>(null)
    private var layout by mutableStateOf<Event.Layout?>(null)
    private var screen: ScreenView? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        host = Host(
            id = intent.getStringExtra(EXTRA_ID) ?: return finish(),
            name = intent.getStringExtra(EXTRA_NAME) ?: "",
            address = intent.getStringExtra(EXTRA_ADDRESS) ?: return finish(),
            username = intent.getStringExtra(EXTRA_USERNAME) ?: "",
            port = intent.getIntExtra(EXTRA_PORT, Host.DEFAULT_PORT),
            virtualDisplay = intent.getBooleanExtra(EXTRA_VIRTUAL_DISPLAY, true),
        )
        hosts = Hosts(this)
        keychain = Keychain(this)

        setContent {
            NaruTheme(dark = true) {
                Box(Modifier.fillMaxSize().background(Color.Black), contentAlignment = Alignment.Center) {
                    if (connection != null) Screen()

                    when {
                        failure != null -> Failure()
                        connection == null -> Login()
                    }
                }
            }
        }

        start()
    }

    override fun onDestroy() {
        disconnect()
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
        if (hasFocus) clipboard?.sync()
    }

    // 세션 중이면 키를 Mac으로 보내고 `true`를 반환합니다. 화면 뷰가 IME보다 먼저 부르고, 포커스가 다른 곳이면 액티비티가 부릅니다.
    private fun handleKey(event: KeyEvent, afterIme: Boolean = false): Boolean {
        val current = connection ?: return false

        if (event.keyCode == KeyEvent.KEYCODE_BACK) {
            if (event.action == KeyEvent.ACTION_UP) finish()
            return true
        }

        val translated = keys.translate(event)

        for ((keysym, down) in translated) {
            current.key(keysym, down)
        }

        // Mac으로 보내지 않는 키는 시스템과 IME에 그대로 넘깁니다. 한/영 키는 IME가 언어를 바꾸고, 볼륨 키는 볼륨을 바꿉니다.
        if (translated.isEmpty() && !KeyEvent.isModifierKey(event.keyCode)) {
            // 화면 뷰(IME 전)와 액티비티(IME 후)가 같은 키를 두 번 볼 수 있으므로 IME 뒤에서만 언어를 다시 읽습니다.
            if (event.keyCode == KeyEvent.KEYCODE_LANGUAGE_SWITCH && event.action == KeyEvent.ACTION_UP && afterIme) {
                inputSource?.sync()
            }

            return false
        }

        return true
    }

    @Composable
    private fun Login() {
        var username by remember { mutableStateOf(host.username) }
        val password = rememberTextFieldState()
        var rememberPassword by remember { mutableStateOf(true) }
        val ready = username.isNotEmpty() && password.text.isNotEmpty()

        val submit = {
            if (ready) {
                connect(Credentials(username, password.text.toString(), rememberPassword))
                password.clearText()
            }
        }

        AlertDialog(
            onDismissRequest = ::finish,
            icon = { Icon(Icons.Outlined.Lock, contentDescription = null) },
            title = { Text("Sign In to “${host.name.ifEmpty { host.address }}”") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    Text("Enter your name and password to share this Mac's screen.")
                    OutlinedTextField(username, { username = it.trim() }, Modifier.fillMaxWidth(), label = { Text("Name") }, singleLine = true)
                    OutlinedSecureTextField(
                        password,
                        Modifier.fillMaxWidth(),
                        label = { Text("Password") },
                        isError = loginMessage != null,
                        supportingText = loginMessage?.let { { Text(it) } },
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Go),
                        onKeyboardAction = { submit() },
                    )

                    Row(
                        Modifier.toggleable(rememberPassword, role = Role.Checkbox) { rememberPassword = it },
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Checkbox(rememberPassword, onCheckedChange = null)
                        Text("Remember this password", Modifier.padding(start = 12.dp))
                    }
                }
            },
            confirmButton = { TextButton(onClick = submit, enabled = ready) { Text("Sign In") } },
            dismissButton = { TextButton(onClick = ::finish) { Text("Cancel") } },
        )
    }


    @Composable
    private fun Failure() {
        val closed = failure ?: return
        val keyChanged = closed.failure is Failure.KeyChanged

        AlertDialog(
            onDismissRequest = ::finish,
            icon = { Icon(if (keyChanged) Icons.Outlined.Key else Icons.Outlined.ErrorOutline, contentDescription = null) },
            title = { Text(if (keyChanged) "Server Identity Changed" else "Connection Failed") },
            text = { Text(message(closed.failure)) },
            confirmButton = {
                if (keyChanged) {
                    TextButton(onClick = { keyFile().delete(); failure = null; start() }) { Text("Trust and Reconnect") }
                } else {
                    TextButton(onClick = { failure = null; start() }) { Text("Reconnect") }
                }
            },
            dismissButton = { TextButton(onClick = ::finish) { Text("Close") } },
        )
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

            // 첫 화면이 그려질 때까지 가운데에 진행 표시와 지금 단계를 둡니다.
            status?.let {
                Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(24.dp)) {
                    @OptIn(ExperimentalMaterial3ExpressiveApi::class)
                    LoadingIndicator(Modifier.size(96.dp))
                    Text(it, color = Color.White, style = MaterialTheme.typography.titleMedium)
                }
            }
        }
    }

    private fun bind(view: ScreenView) {
        view.connection = connection
        view.onKey = { handleKey(it) }
        view.requestFocus()
    }

    // 저장한 비밀번호가 있으면 곧바로 접속하고, 없으면 로그인 창이 뜬 채로 둡니다.
    private fun start() {
        val password = keychain.find(host.id)?.takeIf { host.username.isNotEmpty() } ?: return
        connect(Credentials(host.username, password, remember = false))
    }

    private fun connect(credentials: Credentials) {
        layout = null
        loginMessage = null
        pending = credentials
        status = stageName(Stage.WARMUP)

        val config = Config(
            host = host.address,
            port = host.port.toUShort(),
            username = credentials.username,
            password = credentials.password,
            knownKey = keyFile().takeIf { it.exists() }?.readBytes(),
            display = if (host.virtualDisplay) displayMode() else null,
            audio = Player.supported(),
        )

        connection = Connection(config, object : Connection.Listener {
            override fun onEvent(event: Event) = runOnUiThread { handle(event) }
            override fun onFirstFrame() = runOnUiThread { status = null }
        }).also { connection ->
            inputSource = InputSource(this, connection) { korean -> keys.korean = korean }
            touchpad = Touchpad(connection, resources.displayMetrics.density)
            clipboard = Clipboard(this, connection)
        }

        immersive(true)
    }

    private fun handle(event: Event) {
        when (event) {
            is Event.Progress -> {
                status = stageName(event.stage)
                if (event.stage == Stage.MEDIA) {
                    inputSource?.sync(delayMs = INPUT_SOURCE_DELAY_MS)
                    clipboard?.sync()
                }
            }
            is Event.Source -> inputSource?.onServer(event.source.id)
            is Event.Clipboard -> clipboard?.onServer(event.text)
            is Event.ServerKey -> {
                val file = keyFile()

                if (!file.exists()) {
                    file.parentFile?.mkdirs()
                    file.writeBytes(event.der)
                }
            }
            is Event.Authenticated -> remember()
            is Event.ServerName -> rename(event.name)
            is Event.Layout -> layout = event
            is Event.Cursor -> screen?.setCursor(event)
            is Event.Closed -> closed(event)
            else -> Unit
        }
    }

    // 인증에 성공한 Mac을 목록에 저장하고, 원하면 비밀번호도 저장합니다.
    private fun remember() {
        val credentials = pending ?: return
        pending = null
        host = host.copy(username = credentials.username, lastConnected = System.currentTimeMillis())
        hosts.save(host)

        if (credentials.remember) {
            keychain.save(host.id, credentials.password)
        }
    }

    // 주소로만 추가한 Mac은 Mac이 알려 준 컴퓨터 이름으로 바꿉니다. 사용자가 붙인 이름은 그대로 둡니다.
    private fun rename(name: String) {
        if (name.isEmpty() || host.name != host.address) {
            return
        }

        host = host.copy(name = name)
        hosts.save(host)
    }

    // 사용자가 닫은 세션은 목록으로 돌아갑니다. 비밀번호가 틀렸으면 저장한 비밀번호를 지우고 다시 묻습니다.
    private fun closed(event: Event.Closed) {
        disconnect()
        pending = null

        when (event.failure) {
            null -> finish()
            is Failure.Rejected -> {
                keychain.remove(host.id)
                loginMessage = message(event.failure)
            }
            else -> failure = event
        }
    }

    private fun disconnect() {
        val current = connection ?: return
        Log.i(TAG, "세션을 닫습니다")
        connection = null
        inputSource?.stop()
        inputSource = null
        touchpad = null
        clipboard = null
        keys.korean = false
        screen?.connection = null
        screen = null

        for ((keysym, down) in keys.releaseAll()) {
            current.key(keysym, down)
        }

        current.disconnect()
        status = null
        layout = null
        immersive(false)
    }

    // 가상 디스플레이를 이 창의 픽셀 크기에 맞춥니다. 배율 2로 두어 Mac이 Retina로 그리게 합니다.
    private fun displayMode(): DisplayMode {
        val (width, height) = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            windowManager.currentWindowMetrics.bounds.let { it.width() to it.height() }
        } else {
            resources.displayMetrics.let { it.widthPixels to it.heightPixels }
        }

        return DisplayMode(width = (width / 2).toUInt(), height = (height / 2).toUInt(), scale = 2.0)
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

    private fun keyFile() = File(filesDir, "server-keys/${host.address.replace(Regex("[^A-Za-z0-9.:-]"), "_")}.der")

    private fun message(failure: Failure?): String = when (failure) {
        null -> ""
        is Failure.Rejected -> "Incorrect name or password"
        is Failure.KeyChanged -> "This Mac's identity is different from the last time you connected. Reconnect only if you trust this Mac."
        is Failure.OtherUser -> "${failure.name} is logged in to this Mac."
        is Failure.Unreachable -> "Can't reach ${host.address}. Check the address and that Screen Sharing is turned on."
        is Failure.Unsupported -> "This Mac doesn't support High Performance screen sharing."
        is Failure.NoMedia -> "The Mac didn't start sending its screen."
        is Failure.Other -> "The connection ended unexpectedly."
    }

    private fun stageName(stage: Stage) = when (stage) {
        Stage.WARMUP, Stage.HANDSHAKE -> "Connecting…"
        Stage.AUTH -> "Signing in…"
        Stage.KEY_EXCHANGE, Stage.OFFER, Stage.MEDIA -> "Preparing screen…"
    }

    private data class Credentials(val username: String, val password: String, val remember: Boolean)

    companion object {
        private const val TAG = "Naru"
        private const val INPUT_SOURCE_DELAY_MS = 1_000L

        private const val EXTRA_ID = "id"
        private const val EXTRA_NAME = "name"
        private const val EXTRA_ADDRESS = "address"
        private const val EXTRA_USERNAME = "username"
        private const val EXTRA_PORT = "port"
        private const val EXTRA_VIRTUAL_DISPLAY = "virtualDisplay"

        fun intent(context: Context, host: Host): Intent = Intent(context, SessionActivity::class.java)
            .putExtra(EXTRA_ID, host.id)
            .putExtra(EXTRA_NAME, host.name)
            .putExtra(EXTRA_ADDRESS, host.address)
            .putExtra(EXTRA_USERNAME, host.username)
            .putExtra(EXTRA_PORT, host.port)
            .putExtra(EXTRA_VIRTUAL_DISPLAY, host.virtualDisplay)
    }
}
