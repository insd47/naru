package dev.insd.naru.session

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Binder
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import dev.insd.naru.R
import dev.insd.naru.SessionActivity
import dev.insd.naru.TAG
import dev.insd.naru.audio.Player
import dev.insd.naru.core.Config
import dev.insd.naru.core.DisplayMode
import dev.insd.naru.core.Event
import dev.insd.naru.core.Failure
import dev.insd.naru.core.VideoCodec
import dev.insd.naru.home.Host
import dev.insd.naru.naru

/**
 * Keeps a session with one Mac running off screen.
 *
 * - Runs as a foreground service with a notification only while the session is alive, and stops as soon as it ends.
 * - The list screen watches the connection stage and failures through [state], and opens the session screen once authentication succeeds.
 * - The session screen binds to hand over its Surface and input. The session and audio continue when it leaves, and the last state is replayed when it reattaches.
 * - Saving connection results (the Mac list, password, server key) and the clipboard happen without a screen, so they live here.
 */
class SessionService : Service() {
    private val main = Handler(Looper.getMainLooper())
    private var session: Running? = null
    private var listener: Connection.Listener? = null

    /** State of the current session, or `null` without one. A failure stays until [dismiss]. */
    var state by mutableStateOf<State?>(null)
        private set

    /** Connection of the live session, or `null` */
    val connection: Connection? get() = session?.connection

    override fun onCreate() {
        super.onCreate()
        val channel = NotificationChannel(CHANNEL, "Screen Sharing", NotificationManager.IMPORTANCE_LOW)
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    override fun onBind(intent: Intent): IBinder = Local()

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_DISCONNECT) {
            close()
            return START_NOT_STICKY
        }

        // A service started for the foreground must enter it even when its session already ended, or the system stops the app.
        val title = intent?.getStringExtra(EXTRA_TITLE).orEmpty()
        ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(title), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)

        if (session == null) {
            stopSelf()
        }

        return START_NOT_STICKY
    }

    // Swiping the app away from recents also closes the session.
    override fun onTaskRemoved(rootIntent: Intent?) {
        close()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        session?.connection?.disconnect()
        super.onDestroy()
    }

    /** Opens a session with this Mac, closing any session already open. */
    fun open(host: Host, credentials: Credentials, display: DisplayMode?) {
        session?.connection?.disconnect()
        state = State.Connecting(host)

        val config = Config(
            host = host.address,
            port = host.port.toUShort(),
            username = credentials.username,
            password = credentials.password,
            knownKey = naru.serverKeys.find(host),
            display = display,
            audio = Player.supported(),
            // The server's HEVC is 4:4:4 only, which most Android hardware decoders cannot decode, and there is no profile constant to query support.
            codec = VideoCodec.H264,
        )

        val relay = Relay()
        val connection = Connection(config, relay)
        session = Running(host, credentials, display, connection, Clipboard(this, connection)).also { relay.owner = it }
        ContextCompat.startForegroundService(this, Intent(this, SessionService::class.java).putExtra(EXTRA_TITLE, host.title))
    }

    /** Clears a failure after it has been shown. */
    fun dismiss() {
        if (state is State.Failed) state = null
    }

    /** Attaches a screen and immediately replays the last layout and cursor. */
    fun attach(next: Connection.Listener) {
        listener = next
        val running = session ?: return
        listOfNotNull(running.layout, running.cursor).forEach(next::onEvent)
    }

    fun detach(previous: Connection.Listener) {
        if (listener === previous) listener = null
    }

    /**
     * Resizes the virtual display to this size. Called after rotation or a window resize.
     *
     * - Does nothing when connected to the Mac's real display or when the size is unchanged.
     */
    fun resize(mode: DisplayMode) {
        val running = session ?: return
        val current = running.display ?: return

        if (mode == current) {
            return
        }

        Log.i(TAG, "Virtual display: ${mode.width}×${mode.height}")
        running.display = mode
        running.connection.resize(mode)
    }

    /** Sends text copied in another app to the Mac. Called when the screen gains focus. */
    fun syncClipboard() {
        session?.clipboard?.sync()
    }

    fun close() {
        session?.connection?.disconnect()
    }

    private fun handle(running: Running, event: Event) {
        when (event) {
            is Event.ServerKey -> naru.serverKeys.remember(running.host, event.der)
            is Event.Authenticated -> {
                remember(running)
                state = State.Connected(running.host)
            }
            is Event.ServerName -> rename(running, event.name)
            is Event.ClipboardOffer -> running.clipboard.onOffer(event.kinds)
            is Event.Clipboard -> running.clipboard.onServer(event.items)
            is Event.Layout -> running.layout = event
            is Event.Cursor -> running.cursor = event
            is Event.Closed -> end(running, event)
            else -> Unit
        }

        listener?.onEvent(event)
    }

    // Saves the authenticated Mac to the list, and the password if requested. The password is not held after this.
    private fun remember(running: Running) {
        val credentials = running.credentials ?: return
        running.credentials = null
        running.host = latest(running).copy(username = credentials.username, lastConnected = System.currentTimeMillis())
        naru.hosts.save(running.host)

        if (credentials.remember) {
            naru.keychain.save(running.host.id, credentials.password)
        }
    }

    // A Mac added without a name gets the computer name it reports. A name given by the user is kept.
    private fun rename(running: Running, name: String) {
        val host = latest(running)

        if (name.isEmpty() || host.name.isNotEmpty()) {
            return
        }

        running.host = host.copy(name = name)
        naru.hosts.save(running.host)
        if (state is State.Connected) state = State.Connected(running.host)
    }

    // Saves apply to the store's latest entry so edits made on the list screen while connecting are not overwritten.
    private fun latest(running: Running): Host = naru.hosts.find(running.host.id) ?: running.host

    // A wrong password deletes the saved one. The failure stays in [state] until the list screen shows it.
    private fun end(running: Running, event: Event.Closed) {
        Log.i(TAG, "Session ended")
        session = null
        state = event.failure?.let { State.Failed(running.host, it) }

        if (event.failure is Failure.Rejected) {
            naru.keychain.remove(running.host.id)
        }

        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun notification(title: String): Notification {
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, SessionActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val disconnect = PendingIntent.getService(
            this,
            0,
            Intent(this, SessionService::class.java).setAction(ACTION_DISCONNECT),
            PendingIntent.FLAG_IMMUTABLE,
        )

        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentText("Sharing this Mac's screen")
            .setContentIntent(open)
            .addAction(Notification.Action.Builder(null, "Disconnect", disconnect).build())
            .setOngoing(true)
            .build()
    }

    /** Stage of a session. Each one names its Mac through [host]. */
    sealed interface State {
        val host: Host

        data class Connecting(override val host: Host) : State

        /** Authentication succeeded. The video may still be getting ready. */
        data class Connected(override val host: Host) : State

        data class Failed(override val host: Host, val failure: Failure) : State
    }

    /** Username and password for connecting. On successful authentication, the password is saved if [remember] is `true`. */
    data class Credentials(val username: String, val password: String, val remember: Boolean)

    // The open session. Credentials are held only until authentication, and only the last layout and cursor are kept for replay to the screen.
    // `display` is absent without a virtual display (the Mac's real display).
    private class Running(
        var host: Host,
        var credentials: Credentials?,
        var display: DisplayMode?,
        val connection: Connection,
        val clipboard: Clipboard,
    ) {
        var layout: Event.Layout? = null
        var cursor: Event.Cursor? = null
    }

    // Posts session events and the first-frame notice to the main thread. Late events from a previous session are dropped after switching to a new one.
    private inner class Relay : Connection.Listener {
        var owner: Running? = null

        override fun onEvent(event: Event) {
            main.post { owner?.takeIf { it === session }?.let { handle(it, event) } }
        }

        override fun onFirstFrame() {
            main.post { owner?.takeIf { it === session }?.let { listener?.onFirstFrame() } }
        }
    }

    inner class Local : Binder() {
        val service: SessionService get() = this@SessionService
    }

    companion object {
        private const val CHANNEL = "session"
        private const val NOTIFICATION_ID = 1
        private const val ACTION_DISCONNECT = "dev.insd.naru.DISCONNECT"
        private const val EXTRA_TITLE = "title"
    }
}
