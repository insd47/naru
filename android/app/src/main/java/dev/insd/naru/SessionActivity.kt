package dev.insd.naru

import android.content.ComponentName
import android.content.Intent
import android.content.ServiceConnection
import android.content.res.Configuration
import android.hardware.input.InputManager
import android.os.Bundle
import android.os.IBinder
import android.util.Log
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.LoadingIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.IntOffset
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
import dev.insd.naru.session.Momentum
import dev.insd.naru.session.ScreenView
import dev.insd.naru.session.SessionService
import dev.insd.naru.session.Touchpad
import kotlin.math.roundToInt
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * The screen of a live session, handling only display and input.
 *
 * - The list screen ([MainActivity]) handles connecting, login, and failure messages. This screen opens only for an authenticated session.
 * - [SessionService] owns the session. This screen binds to the service and hands over its Surface and input, so the session continues after leaving and reattaches on return.
 * - Immersive mode, key capture, the Meta key request, and keep-screen-on apply only to this window, so they are released on leaving.
 * - During a session every hardware keyboard key goes to the Mac. Back returns to the list and keeps the session.
 * - The [TitleBar] stays above the Mac's screen in a window (split screen, desktop windowing) and when no mouse or
 *   touchpad is connected. Filling the screen with one, it hides; as in Screen Sharing on the Mac, it comes down with the
 *   status bar when the pointer rests at the top edge, and goes once the pointer leaves it downward. The Mac's screen
 *   moves down with it rather than shrinks, since a new size would resize the Mac's display.
 * - The virtual display matches the actual size of the area drawing the video, which reflects rotation, window resizes, and system bars that cannot be hidden.
 */
class SessionActivity : ComponentActivity() {
    private var service: SessionService? = null
    private var connection by mutableStateOf<Connection?>(null)
    private lateinit var keys: Keys
    private var language: ImeLanguage? = null
    private var touchpad: Touchpad? = null
    private var momentum: Momentum? = null

    private val listener = object : Connection.Listener {
        override fun onEvent(event: Event) = handle(event)
        override fun onFirstFrame() {
            loading = false
        }
    }

    // Before authentication (the notification tapped while connecting) or after the session ended there is nothing to show, so return to the list screen, which handles login and failures.
    private val binding = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) {
            val bound = (binder as SessionService.Local).service
            service = bound

            val connection = bound.connection?.takeIf { bound.state is SessionService.State.Connected } ?: return home()
            title = bound.state?.host?.title.orEmpty()
            adopt(connection)
            bound.attach(listener)
        }

        override fun onServiceDisconnected(name: ComponentName) {
            service = null
        }
    }

    private var loading by mutableStateOf(true)
    private var title by mutableStateOf("")
    private var windowed by mutableStateOf(false)
    // Whether a mouse or touchpad is connected
    private var pointing by mutableStateOf(false)
    // Whether the pointer brought the hidden title bar down
    private var revealed by mutableStateOf(false)
    private var layout by mutableStateOf<Event.Layout?>(null)
    private var cursor by mutableStateOf<Event.Cursor?>(null)

    private val devices = object : InputManager.InputDeviceListener {
        override fun onInputDeviceAdded(deviceId: Int) = recount()
        override fun onInputDeviceRemoved(deviceId: Int) = recount()
        override fun onInputDeviceChanged(deviceId: Int) = recount()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        keys = Keys(swapCommandOption = naru.preferences.swapCommandOption)
        windowed = isInMultiWindowMode
        getSystemService(InputManager::class.java).registerInputDeviceListener(devices, null)
        recount()

        setContent {
            NaruTheme {
                // The status bar sits on the title bar, so its icons follow the theme.
                val dark = isSystemInDarkTheme()
                SideEffect { WindowCompat.getInsetsController(window, window.decorView).isAppearanceLightStatusBars = !dark }

                var area by remember { mutableStateOf<IntSize?>(null) }
                var barHeight by remember { mutableIntStateOf(0) }
                val hides = !windowed && pointing
                val shown by animateFloatAsState(if (!hides || revealed) 1f else 0f, label = "title bar")
                val scope = rememberCoroutineScope()

                // Each resize makes the Mac open a new stream, taking 1.5 to 2 s, so changes during a window drag are
                // collected until it stops and sent once.
                LaunchedEffect(area) {
                    val size = area ?: return@LaunchedEffect
                    delay(RESIZE_SETTLE_MS)
                    service?.resize(virtualDisplay(size.width, size.height))
                }

                // Seen before the Mac's screen and the bar take the pointer. Resting at the top edge brings the bar down
                // after a moment, and moving below it sends the bar back up.
                val edge = Modifier.pointerInput(hides) {
                    if (!hides) return@pointerInput
                    var resting: Job? = null

                    awaitPointerEventScope {
                        while (true) {
                            val change = awaitPointerEvent(PointerEventPass.Initial).changes.first()
                            if (change.type != PointerType.Mouse) continue
                            val y = change.position.y

                            when {
                                revealed && y > barHeight -> reveal(false)
                                !revealed && y < 1.dp.toPx() -> if (resting?.isActive != true) {
                                    resting = scope.launch {
                                        delay(REVEAL_MS)
                                        reveal(true)
                                    }
                                }
                                else -> resting?.cancel()
                            }
                        }
                    }
                }

                Box(Modifier.fillMaxSize().background(Color.Black).then(edge)) {
                    Column(Modifier.fillMaxSize()) {
                        if (!hides && connection != null) Bar()

                        Box(
                            Modifier.fillMaxWidth().weight(1f).onSizeChanged { area = it }
                                .offset { IntOffset(0, if (hides) (shown * barHeight).roundToInt() else 0) },
                            contentAlignment = Alignment.Center,
                        ) {
                            if (connection != null) Screen()
                        }
                    }

                    if (hides && connection != null) {
                        Bar(
                            Modifier.onSizeChanged { barHeight = it.height }
                                .offset { IntOffset(0, ((shown - 1) * barHeight).roundToInt()) }
                                // The pointer on the bar stays off the Mac's screen under it.
                                .pointerInput(Unit) { awaitPointerEventScope { while (true) awaitPointerEvent().changes.forEach { it.consume() } } },
                        )
                    }
                }
            }
        }

        bindService(Intent(this, SessionService::class.java), binding, BIND_AUTO_CREATE)
    }

    // Closes this screen if it reappears from the back stack after the session switched to another Mac.
    override fun onStart() {
        super.onStart()
        val current = service ?: return
        if (connection != null && current.connection !== connection) finish()
        // The setting may have changed while away.
        connection?.setSyncKeyboardLanguage(naru.preferences.syncKeyboardLanguage)
    }

    // No key stays held on the Mac after leaving the screen.
    // On return the Surface and decoder are recreated and stay blank until the first frame, so the loading indicator shows again.
    override fun onStop() {
        loading = true
        momentum?.stop()
        connection?.let { current -> keys.releaseAll().forEach { current.key(it.keysym, it.usage, it.down) } }
        super.onStop()
    }

    // Only the screen goes away; the session stays in the service.
    override fun onDestroy() {
        getSystemService(InputManager::class.java).unregisterInputDeviceListener(devices)
        language?.stop()
        service?.detach(listener)
        unbindService(binding)
        super.onDestroy()
    }

    // During a session keys are captured before the system and Compose so they do not move focus (Tab, arrows).
    override fun dispatchKeyEvent(event: KeyEvent): Boolean = handleKey(event, afterIme = true) || super.dispatchKeyEvent(event)

    // Compose inserts a cancel into touchpad gestures, so they are captured before Compose.
    override fun dispatchTouchEvent(event: MotionEvent): Boolean =
        touchpad?.handle(event) == true || super.dispatchTouchEvent(event)

    // Text copied in another app is sent to the Mac on return.
    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) service?.syncClipboard()
    }

    // During a session, sends the key to the Mac and returns `true`. The screen view calls this before the IME, and the activity calls it when focus is elsewhere.
    private fun handleKey(event: KeyEvent, afterIme: Boolean = false): Boolean {
        val current = connection ?: return false
        val tabletLanguage = language?.current.orEmpty()
        val translated = keys.translate(event, tabletLanguage, current.source)

        if (translated.any { it.down }) report(current, tabletLanguage)

        for (key in translated) {
            current.key(key.keysym, key.usage, key.down)
        }

        // Keys not sent to the Mac pass through to the system and IME: Korean/English switches the IME language, volume keys change volume, and Back changes the screen.
        if (translated.isEmpty() && !KeyEvent.isModifierKey(event.keyCode) && !keys.isMouseNavigation(event) && !keys.isDeadKey(event)) {
            // The screen view (before the IME) and the activity (after it) can both see the same key, so the language is reread only after the IME.
            if (event.keyCode == KeyEvent.KEYCODE_LANGUAGE_SWITCH && event.action == KeyEvent.ACTION_UP && afterIme) {
                language?.sync()
            }

            return false
        }

        return true
    }

    @Composable
    private fun Bar(modifier: Modifier = Modifier) {
        TitleBar(
            title = title,
            onBack = { onBackPressedDispatcher.onBackPressed() },
            onDock = { connection?.toggle(it) },
            onDisconnect = { service?.close() },
            modifier = modifier,
        )
    }

    override fun onMultiWindowModeChanged(isInMultiWindowMode: Boolean, newConfig: Configuration) {
        super.onMultiWindowModeChanged(isInMultiWindowMode, newConfig)
        windowed = isInMultiWindowMode
        revealed = false
        if (connection != null) bars()
    }

    private fun recount() {
        pointing = pointing()
        if (!pointing) revealed = false
        if (connection != null) bars()
    }

    private fun reveal(on: Boolean) {
        revealed = on
        bars()
    }

    // The status bar shows whenever the title bar does, as on the iPad. Filling the screen, the navigation bar stays hidden.
    private fun bars() {
        val controller = WindowCompat.getInsetsController(window, window.decorView)

        when {
            windowed -> controller.show(WindowInsetsCompat.Type.systemBars())
            pointing && !revealed -> controller.hide(WindowInsetsCompat.Type.systemBars())
            else -> {
                controller.hide(WindowInsetsCompat.Type.navigationBars())
                controller.show(WindowInsetsCompat.Type.statusBars())
            }
        }
    }

    // The language goes out as soon as the tablet's language changes, so the Mac has switched before the user types.
    // Reporting it before a key covers a change that was missed; the core sends nothing for a language already reported.
    private fun report(current: Connection, tabletLanguage: String) {
        if (tabletLanguage.isNotEmpty()) current.setInputLanguage(tabletLanguage)
    }

    @Composable
    private fun Screen() {
        val current = layout
        val aspect = current?.let { it.pixelWidth.toFloat() / it.pixelHeight.toFloat() }

        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            AndroidView(
                modifier = if (aspect != null) Modifier.aspectRatio(aspect) else Modifier.fillMaxSize(),
                factory = { context -> ScreenView(context).also(::bind) },
                update = { view ->
                    view.layout = current
                    view.cursor = cursor
                },
            )

            // A centered progress indicator shows until the first frame is drawn.
            if (loading) {
                @OptIn(ExperimentalMaterial3ExpressiveApi::class)
                LoadingIndicator(Modifier.size(96.dp).semantics { contentDescription = "Preparing screen" })
            }
        }
    }

    private fun bind(view: ScreenView) {
        view.connection = connection
        view.momentum = momentum
        view.onKey = { handleKey(it) }
        view.requestFocus()
    }

    // Attaches this screen's input to the session's connection.
    private fun adopt(next: Connection) {
        connection = next
        next.setSyncKeyboardLanguage(naru.preferences.syncKeyboardLanguage)
        language = ImeLanguage(this) { changed -> report(next, changed) }.also { it.sync(delayMs = 0) }
        momentum = Momentum(this) { phase, dx, dy -> next.momentum(phase, dx, dy) }.also { touchpad = Touchpad(this, next, it) }
        immersive(true)
    }

    private fun handle(event: Event) {
        when (event) {
            is Event.Layout -> layout = event
            is Event.Cursor -> cursor = event
            is Event.CursorHidden -> cursor = null
            is Event.Closed -> closed()
            else -> Unit
        }
    }

    private fun home() {
        startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP))
        finish()
    }

    // The rest of the cleanup happens as the screen goes away. Only the system bars and the Meta key request are restored first.
    private fun closed() {
        Log.i(TAG, "Closing the session screen")
        immersive(false)
        finish()
    }

    private fun immersive(on: Boolean) {
        val controller = WindowCompat.getInsetsController(window, window.decorView)

        MetaKey.request(this, on)

        if (on) {
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            controller.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            bars()
        } else {
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            controller.show(WindowInsetsCompat.Type.systemBars())
        }
    }

    companion object {
        private const val RESIZE_SETTLE_MS = 500L
        private const val REVEAL_MS = 500L
    }
}
