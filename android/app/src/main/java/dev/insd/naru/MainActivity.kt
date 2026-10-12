package dev.insd.naru

import android.Manifest
import android.content.ComponentName
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.clearText
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Bookmarks
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.DesktopMac
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Key
import androidx.compose.material.icons.outlined.Lan
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.ScreenShare
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.WifiFind
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenuGroup
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenuPopup
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.FloatingActionButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.OutlinedSecureTextField
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteItem
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteScaffold
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.isSecondaryPressed
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import dev.insd.naru.home.Discovery
import dev.insd.naru.home.Found
import dev.insd.naru.home.Host
import dev.insd.naru.home.Hosts
import dev.insd.naru.home.Keychain
import dev.insd.naru.home.ServerKeys
import dev.insd.naru.protocol.DisplayMode
import dev.insd.naru.protocol.Failure
import dev.insd.naru.protocol.Priority
import dev.insd.naru.session.SessionService
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlin.math.roundToInt

/**
 * 저장한 Mac과 같은 네트워크에서 찾은 Mac, 그리고 모든 Mac에 함께 쓰는 설정을 보여 주는 첫 화면입니다.
 *
 * - 항목을 누르면 이 화면에서 사용자 이름과 비밀번호를 묻고 접속합니다. 저장한 비밀번호가 있으면 묻지 않습니다.
 * - Mac처럼 인증에 성공한 뒤에야 세션 화면([SessionActivity])을 엽니다. 실패는 이 화면이 알립니다.
 * - 세션은 [SessionService]가 소유하므로 이 화면에 있는 동안에도 이어집니다. 그 Mac의 항목에는 Disconnect 버튼을 늘 보여 줍니다.
 * - 마우스를 올리면 Connect 버튼이 나오고, 길게 누르거나 오른쪽 클릭하면 그 자리에 메뉴가 열립니다.
 * - 내비게이션과 새 연결 버튼의 자리는 창 크기에 따라 [NavigationSuiteScaffold]가 고릅니다(넓으면 레일, 좁으면 아래 막대).
 * - 네트워크 탭은 보이는 동안만 찾습니다.
 */
class MainActivity : ComponentActivity() {
    private lateinit var hosts: Hosts
    private lateinit var keychain: Keychain
    private lateinit var serverKeys: ServerKeys
    private lateinit var preferences: Preferences
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
        hosts = Hosts(this)
        keychain = Keychain(this)
        serverKeys = ServerKeys(this)
        preferences = Preferences(this)
        discovery = Discovery(this)

        setContent {
            NaruTheme {
                val state = service?.state
                LaunchedEffect(state) { react(state) }

                Home()
                if (adding) Add()
                editing?.let { Editor(it) }
                signingIn?.let { Login(it) }
                (state as? SessionService.State.Failed)?.takeIf { it.failure !is Failure.Rejected }?.let { Failure(it) }
            }
        }

        bindService(Intent(this, SessionService::class.java), binding, BIND_AUTO_CREATE)
    }

    // 서비스가 접속에 성공하며 저장한 내용을 다시 읽습니다.
    override fun onStart() {
        super.onStart()
        hosts.reload()
    }

    override fun onDestroy() {
        unbindService(binding)
        super.onDestroy()
    }

    override fun onStop() {
        discovery.stop()
        super.onStop()
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
                        Tab.SETTINGS -> Settings()
                    }
                }
            }
        }
    }

    @Composable
    private fun Saved() {
        if (hosts.list.isEmpty()) {
            return Empty(Icons.Outlined.DesktopMac, "No Connections")
        }

        val state = service?.state?.takeIf { it !is SessionService.State.Failed }

        Group {
            itemsIndexed(hosts.list, key = { _, host -> host.id }) { index, host ->
                val active = state?.takeIf { it.host.id == host.id }
                val primary = when (active) {
                    null -> Action("Connect", Icons.Outlined.ScreenShare) { connect(host) }
                    else -> Action("Disconnect", Icons.Outlined.Close) { service?.close() }
                }

                Entry(
                    index = index,
                    count = hosts.list.size,
                    enabled = !connecting() || active != null,
                    title = host.name.ifEmpty { host.address },
                    subtitle = host.address,
                    detail = host.lastConnected?.let(::formatDate),
                    active = active,
                    onConnect = { connect(host) },
                    onInfo = { editing = host },
                    menu = listOf(
                        listOf(primary, Action("Edit", Icons.Outlined.Edit) { editing = host }),
                        listOf(Action("Delete", Icons.Outlined.Delete) { delete(host) }),
                    ),
                )
            }
        }
    }

    // 찾기는 화면이 보이는 동안 계속되므로, 끝이 없는 진행 막대를 위에 둡니다.
    @Composable
    private fun Network() {
        DisposableEffect(Unit) {
            discovery.start()
            onDispose { discovery.stop() }
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
                        enabled = !connecting(),
                        title = found.name,
                        subtitle = null,
                        detail = null,
                        active = null,
                        onConnect = { connect(found) },
                        onInfo = null,
                        menu = listOf(listOf(Action("Connect", Icons.Outlined.ScreenShare) { connect(found) })),
                    )
                }
            }
        }
    }

    // 키 배치는 Parsec처럼 켜고 끄고, 화질은 Screen Sharing 앱처럼 두 가지 중 하나를 고릅니다.
    @Composable
    private fun Settings() {
        val qualities = listOf(
            Priority.LATENCY to ("Adapt to network conditions" to "Keeps the screen moving when packets are lost. Parts of it may look broken briefly."),
            Priority.QUALITY to ("Show the screen at full quality" to "Pauses on the last clean picture until the Mac sends a fresh one."),
        )

        Group {
            item { Section("Keyboard") }

            item {
                SegmentedListItem(
                    onClick = { preferences.swapCommandOption = !preferences.swapCommandOption },
                    shapes = ListItemDefaults.segmentedShapes(0, 1),
                    colors = ListItemDefaults.segmentedColors(),
                    modifier = Modifier.semantics {
                        role = Role.Switch
                        toggleableState = ToggleableState(preferences.swapCommandOption)
                    },
                    supportingContent = { Text("Alt sends Command and the Windows key sends Option, matching the keys on a Mac keyboard.") },
                    trailingContent = { Switch(checked = preferences.swapCommandOption, onCheckedChange = null) },
                ) { Text("Swap Alt and Command") }
            }

            item { Section("Quality") }

            itemsIndexed(qualities) { index, (priority, text) ->
                val (title, detail) = text
                val selected = preferences.priority == priority

                SegmentedListItem(
                    onClick = { preferences.priority = priority },
                    shapes = ListItemDefaults.segmentedShapes(index, qualities.size),
                    colors = ListItemDefaults.segmentedColors(),
                    modifier = Modifier.semantics {
                        role = Role.RadioButton
                        this.selected = selected
                    },
                    leadingContent = { RadioButton(selected = selected, onClick = null) },
                    supportingContent = { Text(detail) },
                ) { Text(title) }
            }
        }
    }

    @Composable
    private fun Section(title: String) {
        Text(
            title,
            Modifier.padding(start = 16.dp, top = 16.dp, bottom = 8.dp),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
        )
    }

    /**
     * Mac 하나의 행. 누르면 접속하고, 마우스를 올리면 마지막 접속 시각 대신 Connect 버튼이 나옵니다.
     *
     * - 다른 Mac에 접속하는 동안에는 [enabled]를 꺼 누를 수 없게 합니다.
     * - 세션 중인 Mac([active])은 Disconnect 버튼을 늘 보여 주고, 누르면 세션 화면으로 돌아갑니다. 접속 중이면 진행 표시를 곁들입니다.
     * - 길게 누르거나 오른쪽 클릭하면 누른 자리에 [menu]가 열립니다. 묶음 사이에는 간격을 둡니다.
     */
    @Composable
    private fun Entry(
        index: Int,
        count: Int,
        enabled: Boolean,
        title: String,
        subtitle: String?,
        detail: String?,
        active: SessionService.State?,
        onConnect: () -> Unit,
        onInfo: (() -> Unit)?,
        menu: List<List<Action>>,
    ) {
        val interaction = remember { MutableInteractionSource() }
        val hovered by interaction.collectIsHoveredAsState()
        var menuAt by remember { mutableStateOf<Offset?>(null) }
        var pressedAt by remember { mutableStateOf(Offset.Zero) }

        Box {
            SegmentedListItem(
                onClick = onConnect,
                enabled = enabled,
                onLongClick = { menuAt = pressedAt },
                onLongClickLabel = "Show actions",
                shapes = ListItemDefaults.segmentedShapes(index, count),
                colors = ListItemDefaults.segmentedColors(),
                interactionSource = interaction,
                modifier = Modifier.pointerInput(Unit) {
                    awaitPointerEventScope {
                        while (true) {
                            val event = awaitPointerEvent(PointerEventPass.Initial)

                            if (event.type == PointerEventType.Press) {
                                pressedAt = event.changes.first().position
                                if (event.buttons.isSecondaryPressed) menuAt = pressedAt
                            }
                        }
                    }
                },
                leadingContent = { Icon(Icons.Outlined.DesktopMac, contentDescription = null, Modifier.size(32.dp)) },
                supportingContent = subtitle?.let { { Text(it) } },
                trailingContent = {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        when {
                            active != null -> {
                                if (active is SessionService.State.Connecting) {
                                    @OptIn(ExperimentalMaterial3ExpressiveApi::class)
                                    LoadingIndicator(Modifier.size(32.dp))
                                }

                                FilledTonalButton(onClick = { service?.close() }) { Text("Disconnect") }
                            }
                            hovered -> FilledTonalButton(onClick = onConnect) { Text("Connect") }
                            detail != null -> Text(detail, style = MaterialTheme.typography.bodyMedium)
                        }

                        onInfo?.let {
                            IconButton(onClick = it) { Icon(Icons.Outlined.Info, contentDescription = "Edit") }
                        }
                    }
                },
            ) { Text(title) }

            menuAt?.let { at ->
                Box(Modifier.offset { IntOffset(at.x.roundToInt(), at.y.roundToInt()) }) {
                    Menu(menu, onDismiss = { menuAt = null })
                }
            }
        }
    }

    @Composable
    private fun Menu(groups: List<List<Action>>, onDismiss: () -> Unit) {
        DropdownMenuPopup(expanded = true, onDismissRequest = onDismiss) {
            Column(verticalArrangement = Arrangement.spacedBy(MenuDefaults.GroupSpacing)) {
                groups.forEachIndexed { groupIndex, actions ->
                    DropdownMenuGroup(shapes = MenuDefaults.groupShape(groupIndex, groups.size)) {
                        actions.forEachIndexed { index, action ->
                            DropdownMenuItem(
                                onClick = { onDismiss(); action.run() },
                                text = { Text(action.label) },
                                shape = MenuDefaults.itemShape(index, actions.size).shape,
                                leadingIcon = { Icon(action.icon, contentDescription = null) },
                            )
                        }
                    }
                }
            }
        }
    }

    // 항목을 모서리가 이어지는 한 묶음으로 보여 줍니다.
    @Composable
    private fun Group(content: LazyListScope.() -> Unit) {
        LazyColumn(
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap),
            content = content,
        )
    }

    @Composable
    private fun Empty(icon: ImageVector, message: String) {
        Column(
            Modifier.fillMaxSize(),
            verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Icon(icon, contentDescription = null, Modifier.size(64.dp), tint = MaterialTheme.colorScheme.outline)
            Text(message, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.outline)
        }
    }

    // 새 연결은 주소만 묻습니다. 사용자 이름과 비밀번호는 접속하면서 묻고, 성공하면 목록에 저장합니다.
    @Composable
    private fun Add() {
        var address by remember { mutableStateOf("") }
        val submit = {
            if (address.isNotEmpty()) {
                adding = false
                connect(Host(name = address, address = address))
            }
        }

        AlertDialog(
            onDismissRequest = { adding = false },
            title = { Text("Connect to a Mac") },
            text = {
                OutlinedTextField(
                    address,
                    { address = it.trim() },
                    Modifier.fillMaxWidth(),
                    label = { Text("Server Address") },
                    placeholder = { Text("Hostname or IP address") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
                    keyboardActions = KeyboardActions(onGo = { submit() }),
                )
            },
            confirmButton = { TextButton(onClick = submit, enabled = address.isNotEmpty()) { Text("Connect") } },
            dismissButton = { TextButton(onClick = { adding = false }) { Text("Cancel") } },
        )
    }

    @Composable
    private fun Editor(original: Host) {
        var name by remember(original.id) { mutableStateOf(original.name) }
        var address by remember(original.id) { mutableStateOf(original.address) }
        var username by remember(original.id) { mutableStateOf(original.username) }
        var port by remember(original.id) { mutableStateOf(original.port.toString()) }
        var virtualDisplay by remember(original.id) { mutableStateOf(original.virtualDisplay) }
        val valid = address.isNotEmpty() && port.toIntOrNull() in 1..65535

        AlertDialog(
            onDismissRequest = { editing = null },
            title = { Text("Edit “${original.name.ifEmpty { original.address }}”") },
            text = {
                // 입력란은 다이얼로그 너비를 채우고, 포트처럼 짧은 값은 주소 옆에 좁게 둡니다.
                Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    OutlinedTextField(name, { name = it }, Modifier.fillMaxWidth(), label = { Text("Name") }, singleLine = true)

                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        OutlinedTextField(
                            address,
                            { address = it.trim() },
                            Modifier.weight(1f),
                            label = { Text("Server Address") },
                            singleLine = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                        )
                        OutlinedTextField(
                            port,
                            { port = it.filter(Char::isDigit).take(5) },
                            Modifier.width(112.dp),
                            label = { Text("Port") },
                            singleLine = true,
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        )
                    }

                    OutlinedTextField(username, { username = it.trim() }, Modifier.fillMaxWidth(), label = { Text("Username") }, singleLine = true)

                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text("Display", style = MaterialTheme.typography.labelLarge)
                        DisplayChoice(virtualDisplay) { virtualDisplay = it }
                    }

                    original.lastConnected?.let {
                        Text(
                            "Last connected ${formatDate(it)}",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            },
            confirmButton = {
                TextButton(
                    enabled = valid,
                    onClick = {
                        hosts.save(
                            original.copy(
                                name = name.trim(),
                                address = address,
                                username = username,
                                port = port.toInt(),
                                virtualDisplay = virtualDisplay,
                            ),
                        )
                        editing = null
                    },
                ) { Text("Save") }
            },
            dismissButton = { TextButton(onClick = { editing = null }) { Text("Cancel") } },
        )
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
            title = { Text("Sign In to “${host.name.ifEmpty { host.address }}”") },
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
                        if (keyChanged) serverKeys.forget(host)
                        close()
                        connect(host)
                    },
                ) { Text(if (keyChanged) "Trust and Reconnect" else "Reconnect") }
            },
            dismissButton = { TextButton(onClick = close) { Text("Close") } },
        )
    }

    // 하나만 고르는 연결된 버튼 묶음
    @OptIn(ExperimentalMaterial3ExpressiveApi::class)
    @Composable
    private fun DisplayChoice(virtualDisplay: Boolean, onChange: (Boolean) -> Unit) {
        val options = listOf(true to "Virtual Display", false to "Mac's Display")

        Row(horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween)) {
            options.forEachIndexed { index, (value, label) ->
                ToggleButton(
                    checked = virtualDisplay == value,
                    onCheckedChange = { onChange(value) },
                    shapes = when (index) {
                        0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                        else -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                    },
                    modifier = Modifier.weight(1f).semantics { role = Role.RadioButton },
                ) { Text(label) }
            }
        }
    }

    // 세션 중인 Mac이면 세션 화면으로 돌아가고, 아니면 저장한 비밀번호로 접속하거나 로그인 창을 엽니다.
    private fun connect(host: Host) {
        val state = service?.state?.takeIf { it.host.id == host.id && it !is SessionService.State.Failed }

        if (state != null) {
            if (state is SessionService.State.Connected) startActivity(Intent(this, SessionActivity::class.java))
            return
        }

        val password = keychain.find(host.id)?.takeIf { host.username.isNotEmpty() } ?: run {
            loginMessage = null
            signingIn = host
            return
        }

        open(host, SessionService.Credentials(host.username, password, remember = false))
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
        hosts.reload()

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

    // 찾은 Mac의 주소를 풀어 접속합니다. 같은 주소로 저장한 Mac이 있으면 그것으로 접속합니다.
    private fun connect(found: Found) {
        discovery.resolve(found) { address ->
            address ?: return@resolve
            val saved = hosts.list.find { it.address == address }
            connect(saved ?: Host(name = found.name, address = address))
        }
    }

    private fun delete(host: Host) {
        hosts.remove(host)
        keychain.remove(host.id)
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

    // Screen Sharing 앱처럼 "Today at 20:48", "Aug 29, 2026 at 20:03"으로 씁니다.
    private fun formatDate(epochMs: Long): String {
        val time = Instant.ofEpochMilli(epochMs).atZone(ZoneId.systemDefault())
        val today = LocalDate.now()
        val clock = time.format(DateTimeFormatter.ofPattern("HH:mm"))

        return when (time.toLocalDate()) {
            today -> "Today at $clock"
            today.minusDays(1) -> "Yesterday at $clock"
            else -> "${time.format(DateTimeFormatter.ofPattern("MMM d, yyyy", Locale.ENGLISH))} at $clock"
        }
    }

    private class Action(val label: String, val icon: ImageVector, val run: () -> Unit)

    private enum class Tab(val title: String, val icon: ImageVector) {
        SAVED("Saved", Icons.Outlined.Bookmarks),
        NETWORK("Network", Icons.Outlined.Lan),
        SETTINGS("Settings", Icons.Outlined.Settings),
    }
}
