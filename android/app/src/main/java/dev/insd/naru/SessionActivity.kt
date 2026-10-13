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
import dev.insd.naru.core.inputSource
import dev.insd.naru.session.Connection
import dev.insd.naru.session.ImeLanguage
import dev.insd.naru.session.Keys
import dev.insd.naru.session.MetaKey
import dev.insd.naru.session.Momentum
import dev.insd.naru.session.ScreenView
import dev.insd.naru.session.SessionService
import dev.insd.naru.session.Touchpad

/**
 * The screen of a live session, handling only display and input.
 *
 * - The list screen ([MainActivity]) handles connecting, login, and failure messages. This screen opens only for an authenticated session.
 * - [SessionService] owns the session. This screen binds to the service and hands over its Surface and input, so the session continues after leaving and reattaches on return.
 * - Immersive mode, key capture, the Meta key request, and keep-screen-on apply only to this window, so they are released on leaving.
 * - During a session every hardware keyboard key goes to the Mac. Back returns to the list and keeps the session.
 * - The virtual display matches the actual size of the area drawing the video, which reflects rotation, window resizes, and system bars that cannot be hidden.
 */
class SessionActivity : ComponentActivity() {
    private var service: SessionService? = null
    private var connection by mutableStateOf<Connection?>(null)
    private lateinit var keys: Keys
    private var language: ImeLanguage? = null
    private var touchpad: Touchpad? = null
    private var momentum: Momentum? = null

    // The Mac input source last selected for the tablet's language, with Sync Keyboard Language on
    private var selectedSource: String? = null

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

    // The screen view is created on the frame after the connection attaches, so a cursor that arrives earlier is kept and applied on attach.
    private var cursor: Event.Cursor? = null

    // Each resize makes the Mac open a new stream, taking 1.5 to 2 s, so changes during a window drag are collected until it stops and sent once.
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

    // Closes this screen if it reappears from the back stack after the session switched to another Mac.
    override fun onStart() {
        super.onStart()
        val current = service ?: return
        if (connection != null && current.connection !== connection) finish()
    }

    // No key stays held on the Mac after leaving the screen.
    // On return the Surface and decoder are recreated and stay blank until the first frame, so the loading indicator shows again.
    override fun onStop() {
        loading = true
        momentum?.stop()
        connection?.let { current -> keys.releaseAll().forEach { (keysym, down) -> current.key(keysym, down) } }
        super.onStop()
    }

    // Only the screen goes away; the session stays in the service.
    override fun onDestroy() {
        main.removeCallbacks(resize)
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

        if (translated.any { (_, down) -> down }) syncSource(current, tabletLanguage)

        for ((keysym, down) in translated) {
            current.key(keysym, down)
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

    // The source goes out as soon as the tablet's language changes, so the Mac has switched before the user types.
    // Selecting before a key covers a change that was missed; the core holds that key until the Mac has switched.
    private fun syncSource(current: Connection, tabletLanguage: String) {
        if (!naru.preferences.syncKeyboardLanguage || tabletLanguage.isEmpty()) {
            selectedSource = null
            return
        }

        val source = inputSource(tabletLanguage, current.source) ?: return

        if (source != selectedSource) {
            current.selectInputSource(source)
            selectedSource = source
        }
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

            // A centered progress indicator shows until the first frame is drawn.
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
        view.momentum = momentum
        cursor?.let(view::setCursor)
        view.onKey = { handleKey(it) }
        view.requestFocus()
    }

    // Attaches this screen's input to the session's connection.
    private fun adopt(next: Connection) {
        connection = next
        selectedSource = null
        language = ImeLanguage(this) { changed -> syncSource(next, changed) }.also { it.sync(delayMs = 0) }
        momentum = Momentum(this) { phase, dx, dy -> next.momentum(phase, dx, dy) }.also { touchpad = Touchpad(this, next, it) }
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
