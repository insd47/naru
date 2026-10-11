package dev.insd.naru

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
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
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.DesktopMac
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Lan
import androidx.compose.material.icons.outlined.ScreenShare
import androidx.compose.material.icons.outlined.WifiFind
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonGroupDefaults
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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteScaffold
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteItem
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
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
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import dev.insd.naru.home.Discovery
import dev.insd.naru.home.Found
import dev.insd.naru.home.Host
import dev.insd.naru.home.Hosts
import dev.insd.naru.home.Keychain
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlin.math.roundToInt

/**
 * 저장한 Mac과 같은 네트워크에서 찾은 Mac을 보여 주는 첫 화면입니다.
 *
 * - 항목을 누르면 세션 화면([SessionActivity])이 열리고, 사용자 이름과 비밀번호는 그 화면이 묻고 저장합니다.
 * - 마우스를 올리면 Connect 버튼이 나오고, 길게 누르거나 오른쪽 클릭하면 그 자리에 메뉴가 열립니다.
 * - 내비게이션과 새 연결 버튼의 자리는 창 크기에 따라 [NavigationSuiteScaffold]가 고릅니다(넓으면 레일, 좁으면 아래 막대).
 * - 네트워크 탭은 보이는 동안만 찾습니다.
 */
class MainActivity : ComponentActivity() {
    private lateinit var hosts: Hosts
    private lateinit var keychain: Keychain
    private lateinit var discovery: Discovery

    private var tab by mutableStateOf(Tab.SAVED)
    private var adding by mutableStateOf(false)
    private var editing by mutableStateOf<Host?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        hosts = Hosts(this)
        keychain = Keychain(this)
        discovery = Discovery(this)

        setContent {
            NaruTheme {
                Home()
                if (adding) Add()
                editing?.let { Editor(it) }
            }
        }
    }

    // 세션 화면이 접속에 성공하며 저장한 내용을 다시 읽습니다.
    override fun onStart() {
        super.onStart()
        hosts.reload()
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
            primaryActionContent = {
                // 레일(96dp) 머리는 내용을 왼쪽에 붙이므로 FAB(56dp)를 가운데에 오게 띄웁니다. 레일 안의 FAB는 그림자가 없습니다.
                FloatingActionButton(
                    onClick = { adding = true },
                    modifier = Modifier.padding(horizontal = 20.dp),
                    elevation = FloatingActionButtonDefaults.elevation(defaultElevation = 0.dp),
                ) { Icon(Icons.Outlined.Add, contentDescription = "New Connection") }
            },
        ) {
            Scaffold(topBar = { TopAppBar(title = { Text(tab.title) }) }) { padding ->
                Box(Modifier.padding(padding).fillMaxSize()) {
                    when (tab) {
                        Tab.SAVED -> Saved()
                        Tab.NETWORK -> Network()
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

        Group {
            itemsIndexed(hosts.list, key = { _, host -> host.id }) { index, host ->
                Entry(
                    index = index,
                    count = hosts.list.size,
                    title = host.name.ifEmpty { host.address },
                    subtitle = host.address,
                    detail = host.lastConnected?.let(::formatDate),
                    onConnect = { connect(host) },
                    onInfo = { editing = host },
                    menu = listOf(
                        listOf(
                            Action("Connect", Icons.Outlined.ScreenShare) { connect(host) },
                            Action("Edit", Icons.Outlined.Edit) { editing = host },
                        ),
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
                        title = found.name,
                        subtitle = null,
                        detail = null,
                        onConnect = { connect(found) },
                        onInfo = null,
                        menu = listOf(listOf(Action("Connect", Icons.Outlined.ScreenShare) { connect(found) })),
                    )
                }
            }
        }
    }

    /**
     * Mac 하나의 행. 누르면 접속하고, 마우스를 올리면 마지막 접속 시각 대신 Connect 버튼이 나옵니다.
     *
     * - 길게 누르거나 오른쪽 클릭하면 누른 자리에 [menu]가 열립니다. 묶음 사이에는 간격을 둡니다.
     */
    @Composable
    private fun Entry(
        index: Int,
        count: Int,
        title: String,
        subtitle: String?,
        detail: String?,
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

    private fun connect(host: Host) = startActivity(SessionActivity.intent(this, host))

    // 찾은 Mac의 주소를 풀어 접속합니다. 같은 주소나 같은 이름으로 저장한 Mac이 있으면 그것으로 접속합니다.
    // Mac은 주소가 여럿일 수 있어, 찾을 때 풀린 주소가 저장한 주소와 다를 수 있습니다.
    private fun connect(found: Found) {
        discovery.resolve(found) { address ->
            address ?: return@resolve
            val saved = hosts.list.find { it.address == address || it.name == found.name }
            connect(saved ?: Host(name = found.name, address = address))
        }
    }

    private fun delete(host: Host) {
        hosts.remove(host)
        keychain.remove(host.id)
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
        SAVED("All Connections", Icons.Outlined.Devices),
        NETWORK("Network", Icons.Outlined.Lan),
    }
}
