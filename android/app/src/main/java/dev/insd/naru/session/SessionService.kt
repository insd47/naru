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
import dev.insd.naru.home.Host
import dev.insd.naru.naru
import dev.insd.naru.protocol.Config
import dev.insd.naru.protocol.DisplayMode
import dev.insd.naru.protocol.Event
import dev.insd.naru.protocol.Failure
import dev.insd.naru.protocol.Stage

/**
 * Mac 한 대와의 세션을 화면 밖에서도 이어 갑니다.
 *
 * - 세션이 살아 있는 동안만 알림과 함께 포그라운드 서비스로 돌고, 세션이 끝나면 곧바로 멈춥니다.
 * - 목록 화면은 [state]로 접속 단계와 실패를 봅니다. 인증에 성공하면 목록 화면이 세션 화면을 엽니다.
 * - 세션 화면은 바인드해서 Surface와 입력을 넘깁니다. 화면이 떠나도 세션과 소리는 이어지고, 다시 붙으면 마지막 상태를 되감아 줍니다.
 * - 접속 결과를 저장하는 일(Mac 목록, 비밀번호, 서버 키)과 클립보드는 화면이 없어도 일어나므로 여기서 합니다.
 */
class SessionService : Service() {
    private val main = Handler(Looper.getMainLooper())
    private var session: Running? = null
    private var listener: Connection.Listener? = null

    /** 지금 세션의 상태. 세션이 없으면 `null`이고, 실패는 [dismiss]할 때까지 남습니다. */
    var state by mutableStateOf<State?>(null)
        private set

    /** 살아 있는 세션의 연결. 없으면 `null`입니다. */
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

        val host = session?.host

        if (host == null) {
            stopSelf()
            return START_NOT_STICKY
        }

        ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(host), ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        return START_NOT_STICKY
    }

    // 최근 앱 목록에서 앱을 밀어 지우면 세션도 닫습니다.
    override fun onTaskRemoved(rootIntent: Intent?) {
        close()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        session?.connection?.disconnect()
        super.onDestroy()
    }

    /** 이 Mac과의 세션을 엽니다. 이미 열린 세션이 있으면 닫고 새로 엽니다. */
    fun open(host: Host, credentials: Credentials, display: DisplayMode?) {
        session?.connection?.disconnect()
        state = State.Connecting(host, Stage.WARMUP)

        val config = Config(
            host = host.address,
            port = host.port.toUShort(),
            username = credentials.username,
            password = credentials.password,
            knownKey = naru.serverKeys.find(host),
            display = display,
            audio = Player.supported(),
            priority = naru.preferences.priority,
        )

        val relay = Relay()
        val connection = Connection(config, relay)
        session = Running(host, credentials, connection, Clipboard(this, connection)).also { relay.owner = it }
        ContextCompat.startForegroundService(this, Intent(this, SessionService::class.java))
    }

    /** 실패를 보여 준 뒤 지웁니다. */
    fun dismiss() {
        if (state is State.Failed) state = null
    }

    /** 화면을 붙입니다. 마지막 레이아웃과 커서를 곧바로 되감아 줍니다. */
    fun attach(next: Connection.Listener) {
        listener = next
        val running = session ?: return
        listOfNotNull(running.layout, running.cursor).forEach(next::onEvent)
    }

    fun detach(previous: Connection.Listener) {
        if (listener === previous) listener = null
    }

    /** 알림을 다시 올립니다. 알림 권한을 접속 뒤에 허용하면 처음 올린 알림은 버려졌으므로 부릅니다. */
    fun notifyAgain() {
        val host = session?.host ?: return
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification(host))
    }

    /** 다른 앱에서 복사한 텍스트를 Mac에 보냅니다. 화면이 포커스를 얻을 때 부릅니다. */
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
            is Event.Clipboard -> running.clipboard.onServer(event.text)
            is Event.Layout -> running.layout = event
            is Event.Cursor -> running.cursor = event
            is Event.Progress -> if (state is State.Connecting) state = State.Connecting(running.host, event.stage)
            is Event.Closed -> end(running, event)
            else -> Unit
        }

        listener?.onEvent(event)
    }

    // 인증에 성공한 Mac을 목록에 저장하고, 원하면 비밀번호도 저장합니다. 비밀번호는 이 뒤로 들고 있지 않습니다.
    private fun remember(running: Running) {
        val credentials = running.credentials ?: return
        running.credentials = null
        running.host = latest(running).copy(username = credentials.username, lastConnected = System.currentTimeMillis())
        naru.hosts.save(running.host)

        if (credentials.remember) {
            naru.keychain.save(running.host.id, credentials.password)
        }
    }

    // 이름 없이 추가한 Mac은 Mac이 알려 준 컴퓨터 이름으로 채웁니다. 사용자가 붙인 이름은 그대로 둡니다.
    private fun rename(running: Running, name: String) {
        val host = latest(running)

        if (name.isEmpty() || host.name.isNotEmpty()) {
            return
        }

        running.host = host.copy(name = name)
        naru.hosts.save(running.host)
        if (state is State.Connected) state = State.Connected(running.host)
    }

    // 접속하는 동안 목록 화면에서 고친 내용을 덮어쓰지 않도록, 저장할 때는 저장소의 최신 값에 더합니다.
    private fun latest(running: Running): Host = naru.hosts.find(running.host.id) ?: running.host

    // 비밀번호가 틀렸으면 저장한 비밀번호를 지웁니다. 실패는 목록 화면이 보여 줄 때까지 [state]에 남깁니다.
    private fun end(running: Running, event: Event.Closed) {
        Log.i(TAG, "세션이 끝났습니다")
        session = null
        state = event.failure?.let { State.Failed(running.host, it) }

        if (event.failure is Failure.Rejected) {
            naru.keychain.remove(running.host.id)
        }

        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun notification(host: Host): Notification {
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
            .setContentTitle(host.title)
            .setContentText("Sharing this Mac's screen")
            .setContentIntent(open)
            .addAction(Notification.Action.Builder(null, "Disconnect", disconnect).build())
            .setOngoing(true)
            .build()
    }

    /** 세션의 단계. 모두 어느 Mac과의 세션인지 [host]로 알려 줍니다. */
    sealed interface State {
        val host: Host

        data class Connecting(override val host: Host, val stage: Stage) : State

        /** 인증에 성공했습니다. 화면은 아직 준비 중일 수 있습니다. */
        data class Connected(override val host: Host) : State

        data class Failed(override val host: Host, val failure: Failure) : State
    }

    /** 접속에 쓸 사용자 이름과 비밀번호. 인증에 성공하면 [remember]가 `true`일 때 비밀번호를 저장합니다. */
    data class Credentials(val username: String, val password: String, val remember: Boolean)

    // 열려 있는 세션 하나. 자격 증명은 인증할 때까지만 들고, 화면에 되감아 줄 레이아웃과 커서는 마지막 것만 남깁니다.
    private class Running(var host: Host, var credentials: Credentials?, val connection: Connection, val clipboard: Clipboard) {
        var layout: Event.Layout? = null
        var cursor: Event.Cursor? = null
    }

    // 세션의 사건과 첫 화면 알림을 메인 스레드로 넘깁니다. 새 세션으로 바뀐 뒤 늦게 도착한 이전 세션의 사건은 버립니다.
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
    }
}
