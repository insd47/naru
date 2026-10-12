package dev.insd.naru

import android.Manifest
import android.content.ComponentName
import android.content.Intent
import android.content.ServiceConnection
import android.content.res.Configuration
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Bookmarks
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.DesktopMac
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Lan
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.ScreenShare
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.WifiFind
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.FloatingActionButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.OutlinedSecureTextField
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteItem
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteScaffold
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleStartEffect
import dev.insd.naru.home.Action
import dev.insd.naru.home.AddDialog
import dev.insd.naru.home.Discovery
import dev.insd.naru.home.EditDialog
import dev.insd.naru.home.Empty
import dev.insd.naru.home.Entry
import dev.insd.naru.home.Found
import dev.insd.naru.home.Group
import dev.insd.naru.home.Host
import dev.insd.naru.home.formatDate
import dev.insd.naru.protocol.DisplayMode
import dev.insd.naru.protocol.Failure
import dev.insd.naru.session.SessionService

/**
 * 저장한 Mac과 같은 네트워크에서 찾은 Mac, 그리고 모든 Mac에 함께 쓰는 설정을 보여 주는 첫 화면입니다.
 *
 * - 항목을 누르면 이 화면에서 사용자 이름과 비밀번호를 묻고 접속합니다. 저장한 비밀번호가 있으면 묻지 않습니다.
 * - Mac처럼 인증에 성공한 뒤에야 세션 화면([SessionActivity])을 엽니다. 실패는 이 화면이 알립니다.
 * - 세션은 [SessionService]가 소유하므로 이 화면에 있는 동안에도 이어집니다. 그 Mac의 항목에는 Disconnect 버튼을 늘 보여 줍니다.
 * - 내비게이션과 새 연결 버튼의 자리는 창 크기에 따라 [NavigationSuiteScaffold]가 고릅니다(넓으면 레일, 좁으면 아래 막대).
 * - 네트워크 탭은 보이는 동안만 찾습니다.
 */
class MainActivity : ComponentActivity() {
    private lateinit var discovery: Discovery

    private var tab by mutableStateOf(Tab.SAVED)
    private var adding by mutableStateOf(false)
    private var editing by mutableStateOf<Host?>(null)
    private var signingIn by mutableStateOf<Host?>(null)
    private var loginMessage by mutableStateOf<String?>(null)

    // 인증에 성공하면 세션 화면을 열 Mac의 ID. 이 화면에서 접속을 시작했을 때만 둡니다.
    private var opening: String? = null

    private var service by mutableStateOf<SessionService?>(null)

    private val binding = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName, binder: IBinder) {
            service = (binder as SessionService.Local).service
        }

        override fun onServiceDisconnected(name: ComponentName) {
            service = null
        }
    }

    // 알림을 못 띄우면 세션은 이어지지만 알림의 연결 끊기 버튼이 보이지 않습니다. 거절해도 접속은 그대로 합니다.
    private val notificationPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) service?.notifyAgain()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        discovery = Discovery(this)

        setContent {
            NaruTheme {
                val state = service?.state
                LaunchedEffect(state) { react(state) }

                Home()
                if (adding) AddDialog(onConnect = { adding = false; connect(Host(address = it)) }, onDismiss = { adding = false })
                editing?.let { EditDialog(it, onSave = { edited -> save(edited); editing = null }, onDismiss = { editing = null }) }
                signingIn?.let { Login(it) }
                (state as? SessionService.State.Failed)?.takeIf { it.failure !is Failure.Rejected }?.let { Failure(it) }
            }
        }

        bindService(Intent(this, SessionService::class.java), binding, BIND_AUTO_CREATE)
    }

    override fun onDestroy() {
        unbindService(binding)
        super.onDestroy()
    }

    // 다크 모드를 바꿔도 화면을 다시 만들지 않으므로, 시스템 막대의 아이콘 색을 새 모드에 맞춥니다.
    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        enableEdgeToEdge()
    }

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    private fun Home() {
        NavigationSuiteScaffold(
            navigationItems = {
                for (destination in Tab.entries) {
                    NavigationSuiteItem(
                        selected = tab == destination,
                        onClick = { tab = destination },
                        icon = { Icon(destination.icon, contentDescription = null) },
                        label = { Text(destination.title) },
                    )
                }
            },
            // 태블릿의 레일은 항목을 세로 가운데에 모으고, 새 연결 버튼은 위에 둡니다.
            navigationItemVerticalArrangement = Arrangement.Center,
            // 레일(96dp) 머리는 내용을 왼쪽에 붙이므로 FAB(56dp)를 가운데에 오게 띄웁니다. 레일 안의 FAB는 그림자가 없습니다.
            // 설정 탭에는 새 연결이 어울리지 않으므로 두지 않습니다.
            primaryActionContent = {
                if (tab != Tab.SETTINGS) {
                    FloatingActionButton(
                        onClick = { if (!connecting()) adding = true },
                        modifier = Modifier.padding(horizontal = 20.dp),
                        elevation = FloatingActionButtonDefaults.elevation(defaultElevation = 0.dp),
                    ) { Icon(Icons.Outlined.Add, contentDescription = "New Connection") }
                }
            },
        ) {
            Scaffold(topBar = { TopAppBar(title = { Text(tab.title) }) }) { padding ->
                Box(Modifier.padding(padding).fillMaxSize()) {
                    when (tab) {
                        Tab.SAVED -> Saved()
                        Tab.NETWORK -> Network()
                        Tab.SETTINGS -> Settings(naru.preferences)
                    }
                }
            }
        }
    }

    @Composable
    private fun Saved() {
        val hosts = naru.hosts.list

        if (hosts.isEmpty()) {
            return Empty(Icons.Outlined.DesktopMac, "No Connections")
        }

        val state = service?.state?.takeIf { it !is SessionService.State.Failed }

        Group {
            itemsIndexed(hosts, key = { _, host -> host.id }) { index, host ->
                val active = state?.takeIf { it.host.id == host.id }
                val disconnect: (() -> Unit)? = if (active != null) ({ service?.close() }) else null
                val primary = when (disconnect) {
                    null -> Action("Connect", Icons.Outlined.ScreenShare) { connect(host) }
                    else -> Action("Disconnect", Icons.Outlined.Close, disconnect)
                }

                Entry(
                    index = index,
                    count = hosts.size,
                    title = host.title,
                    subtitle = host.address,
                    detail = host.lastConnected?.let(::formatDate),
                    onConnect = { connect(host) },
                    menu = listOf(
                        listOf(primary, Action("Edit", Icons.Outlined.Edit) { editing = host }),
                        listOf(Action("Delete", Icons.Outlined.Delete) { delete(host) }),
                    ),
                    enabled = !connecting() || active != null,
                    connecting = active is SessionService.State.Connecting,
                    onDisconnect = disconnect,
                    onInfo = { editing = host },
                )
            }
        }
    }

    // 찾기는 화면이 보이는 동안 계속되므로, 끝이 없는 진행 막대를 위에 둡니다. 앱을 떠나면 멈추고 돌아오면 다시 찾습니다.
    @Composable
    private fun Network() {
        LifecycleStartEffect(Unit) {
            discovery.start()
            onStopOrDispose { discovery.stop() }
        }

        Column {
            LinearProgressIndicator(Modifier.fillMaxWidth())

            if (discovery.found.isEmpty()) {
                return@Column Empty(Icons.Outlined.WifiFind, "Searching for Macs")
            }

            Group {
                itemsIndexed(discovery.found, key = { _, found -> found.name }) { index, found ->
                    Entry(
                        index = index,
                        count = discovery.found.size,
                        title = found.name,
                        subtitle = null,
                        detail = null,
                        onConnect = { connect(found) },
                        menu = listOf(listOf(Action("Connect", Icons.Outlined.ScreenShare) { connect(found) })),
                        enabled = !connecting(),
                    )
                }
            }
        }
    }

    // 접속하는 동안에는 창을 둔 채 입력을 막고, 인증에 성공하면 [react]가 창을 닫습니다.
    @Composable
    private fun Login(host: Host) {
        var username by remember(host.id) { mutableStateOf(host.username) }
        val password = rememberTextFieldState()
        var rememberPassword by remember(host.id) { mutableStateOf(true) }
        val busy = (service?.state as? SessionService.State.Connecting)?.host?.id == host.id
        val ready = !busy && username.isNotEmpty() && password.text.isNotEmpty()

        val submit = {
            if (ready) {
                loginMessage = null
                open(host, SessionService.Credentials(username, password.text.toString(), rememberPassword))
                password.clearText()
            }
        }
        val cancel = {
            if (busy) service?.close()
            opening = null
            signingIn = null
        }

        AlertDialog(
            onDismissRequest = cancel,
            icon = { Icon(Icons.Outlined.Lock, contentDescription = null) },
            title = { Text("Sign In to “${host.title}”") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    Text("Enter your name and password to share this Mac's screen.")
                    OutlinedTextField(
                        username,
                        { username = it.trim() },
                        Modifier.fillMaxWidth(),
                        enabled = !busy,
                        label = { Text("Name") },
                        singleLine = true,
                    )
                    OutlinedSecureTextField(
                        password,
                        Modifier.fillMaxWidth(),
                        enabled = !busy,
                        label = { Text("Password") },
                        isError = loginMessage != null,
                        supportingText = loginMessage?.let { { Text(it) } },
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Go),
                        onKeyboardAction = { submit() },
                    )

                    Row(
                        Modifier.toggleable(rememberPassword, enabled = !busy, role = Role.Checkbox) { rememberPassword = it },
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Checkbox(rememberPassword, onCheckedChange = null, enabled = !busy)
                        Text("Remember this password", Modifier.padding(start = 12.dp))
                    }
                }
            },
            confirmButton = { TextButton(onClick = submit, enabled = ready) { Text(if (busy) "Signing In…" else "Sign In") } },
            dismissButton = { TextButton(onClick = cancel) { Text("Cancel") } },
        )
    }

    @Composable
    private fun Failure(failed: SessionService.State.Failed) {
        val host = failed.host
        val keyChanged = failed.failure is Failure.KeyChanged
        val close: () -> Unit = { service?.dismiss() }

        AlertDialog(
            onDismissRequest = close,
            icon = { Icon(if (keyChanged) Icons.Outlined.Key else Icons.Outlined.ErrorOutline, contentDescription = null) },
            title = { Text(if (keyChanged) "Server Identity Changed" else "Connection Failed") },
            text = { Text(message(failed.failure, host)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        if (keyChanged) naru.serverKeys.forget(host)
                        close()
                        connect(host)
                    },
                ) { Text(if (keyChanged) "Trust and Reconnect" else "Reconnect") }
            },
            dismissButton = { TextButton(onClick = close) { Text("Close") } },
        )
    }

    // 세션 중인 Mac이면 세션 화면으로 돌아가고, 아니면 저장한 비밀번호로 접속하거나 로그인 창을 엽니다.
    private fun connect(host: Host) {
        val state = service?.state?.takeIf { it.host.id == host.id && it !is SessionService.State.Failed }

        if (state != null) {
            if (state is SessionService.State.Connected) startActivity(Intent(this, SessionActivity::class.java))
            return
        }

        val password = naru.keychain.find(host.id)?.takeIf { host.username.isNotEmpty() } ?: run {
            loginMessage = null
            signingIn = host
            return
        }

        open(host, SessionService.Credentials(host.username, password, remember = false))
    }

    // 찾은 Mac의 주소를 풀어 접속합니다. 같은 주소로 저장한 Mac이 있으면 그것으로 접속합니다.
    private fun connect(found: Found) {
        discovery.resolve(found) { address ->
            address ?: return@resolve
            val saved = naru.hosts.list.find { it.address == address }
            connect(saved ?: Host(name = found.name, address = address))
        }
    }

    // 접속하는 동안에는 목록을 막아 다른 Mac으로 접속을 겹쳐 시작하지 않게 합니다.
    private fun connecting(): Boolean = service?.state is SessionService.State.Connecting

    private fun open(host: Host, credentials: SessionService.Credentials) {
        val bound = service ?: return

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }

        bound.dismiss()
        opening = host.id
        bound.open(host, credentials, if (host.virtualDisplay) displayMode() else null)
    }

    // 인증에 성공하면 세션 화면을 열고, 비밀번호가 틀리면 로그인 창에 표시합니다. 그 밖의 실패는 [Failure]가 보여 줍니다.
    private fun react(state: SessionService.State?) {
        when (state) {
            is SessionService.State.Connected -> if (opening == state.host.id) {
                opening = null
                signingIn = null
                startActivity(Intent(this, SessionActivity::class.java))
            }
            is SessionService.State.Failed -> {
                opening = null

                if (state.failure is Failure.Rejected) {
                    loginMessage = message(state.failure, state.host)
                    signingIn = signingIn ?: state.host
                    service?.dismiss()
                } else {
                    signingIn = null
                }
            }
            else -> Unit
        }
    }

    // 세션 화면이 가로채기 전에 이 창의 크기로 가상 디스플레이를 정합니다. 배율 2로 두어 Mac이 Retina로 그리게 합니다.
    private fun displayMode(): DisplayMode {
        val (width, height) = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            windowManager.currentWindowMetrics.bounds.let { it.width() to it.height() }
        } else {
            resources.displayMetrics.let { it.widthPixels to it.heightPixels }
        }

        return DisplayMode(width = (width / 2).toUInt(), height = (height / 2).toUInt(), scale = 2.0)
    }

    // 다이얼로그를 여는 사이 세션이 저장한 접속 시각이나 이름을 덮어쓰지 않도록, 고친 항목만 최신 값에 더합니다.
    private fun save(edited: Host) {
        val latest = naru.hosts.find(edited.id) ?: edited
        val merged = latest.copy(
            name = edited.name,
            address = edited.address,
            username = edited.username,
            port = edited.port,
            virtualDisplay = edited.virtualDisplay,
        )
        naru.hosts.save(merged)
    }

    private fun delete(host: Host) {
        naru.hosts.remove(host)
        naru.keychain.remove(host.id)
    }

    private fun message(failure: Failure, host: Host): String = when (failure) {
        is Failure.Rejected -> "Incorrect name or password"
        is Failure.KeyChanged -> "This Mac's identity is different from the last time you connected. Reconnect only if you trust this Mac."
        is Failure.OtherUser -> "${failure.name} is logged in to this Mac."
        is Failure.Unreachable -> "Can't reach ${host.address}. Check the address and that Screen Sharing is turned on."
        is Failure.Unsupported -> "This Mac doesn't support High Performance screen sharing."
        is Failure.NoMedia -> "The Mac didn't start sending its screen."
        is Failure.Other -> "The connection ended unexpectedly."
    }

    private enum class Tab(val title: String, val icon: ImageVector) {
        SAVED("Saved", Icons.Outlined.Bookmarks),
        NETWORK("Network", Icons.Outlined.Lan),
        SETTINGS("Settings", Icons.Outlined.Settings),
    }
}
