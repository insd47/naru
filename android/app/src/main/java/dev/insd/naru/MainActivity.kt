package dev.insd.naru

import android.content.ComponentName
import android.content.Intent
import android.content.ServiceConnection
import android.content.res.Configuration
import android.os.Bundle
import android.os.IBinder
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
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
import dev.insd.naru.core.Failure
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
import dev.insd.naru.session.SessionService

/**
 * The first screen, showing saved Macs, Macs found on the local network, and settings shared by every Mac.
 *
 * - Tapping an item asks for the username and password on this screen and connects. A saved password skips the prompt.
 * - As on the Mac, the session screen ([SessionActivity]) opens only after authentication succeeds. This screen reports failures.
 * - [SessionService] owns the session, so it continues while this screen is shown. That Mac's item always shows a Disconnect button.
 * - [NavigationSuiteScaffold] places the navigation and the new connection button by window size (a rail when wide, a bottom bar when narrow).
 * - The network tab discovers only while visible.
 */
class MainActivity : ComponentActivity() {
    private lateinit var discovery: Discovery

    private var tab by mutableStateOf(Tab.SAVED)
    private var adding by mutableStateOf(false)
    private var editing by mutableStateOf<Host?>(null)
    private var signingIn by mutableStateOf<Host?>(null)
    private var loginMessage by mutableStateOf<String?>(null)

    // ID of the Mac whose session screen opens once authentication succeeds. Set only when the connection started from this screen.
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

    // Switching dark mode does not recreate the activity, so the system bar icon colors are updated to match.
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
            // On tablets the rail centers its items vertically and puts the new connection button at the top.
            navigationItemVerticalArrangement = Arrangement.Center,
            // The rail (96dp) header aligns content to the start, so the FAB (56dp) is padded to center it. A FAB inside the rail has no shadow.
            // A new connection button does not fit the settings tab, so it is omitted there.
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

    // Discovery runs as long as the screen is visible, so an indeterminate progress bar sits on top. It stops on leaving the app and resumes on return.
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

    // While connecting, the dialog stays open with input disabled, and [react] closes it once authentication succeeds.
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

    // Returns to the session screen for a Mac in session; otherwise connects with the saved password or opens the login dialog.
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

    // Resolves a found Mac's address and connects, using a saved Mac with the same address if there is one.
    private fun connect(found: Found) {
        discovery.resolve(found) { address ->
            address ?: return@resolve
            val saved = naru.hosts.list.find { it.address == address }
            connect(saved ?: Host(name = found.name, address = address))
        }
    }

    // The list is disabled while connecting so a second connection to another Mac cannot start.
    private fun connecting(): Boolean = service?.state is SessionService.State.Connecting

    private fun open(host: Host, credentials: SessionService.Credentials) {
        val bound = service ?: return

        bound.dismiss()
        opening = host.id
        bound.open(host, credentials, if (host.virtualDisplay) virtualDisplay() else null)
    }

    // Opens the session screen on successful authentication and shows a wrong password in the login dialog. [Failure] shows other failures.
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

    // Only the edited fields are applied to the latest entry, so a connection time or name the session saved while the dialog was open is not overwritten.
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

    // Server keys are stored per address, so the key stays while another saved Mac uses the same address.
    private fun delete(host: Host) {
        naru.hosts.remove(host)
        naru.keychain.remove(host.id)

        if (naru.hosts.list.none { it.address == host.address }) {
            naru.serverKeys.forget(host)
        }
    }

    private fun message(failure: Failure, host: Host): String = when (failure) {
        is Failure.Rejected -> "Incorrect name or password"
        is Failure.KeyChanged -> "This Mac's identity is different from the last time you connected. Reconnect only if you trust this Mac."
        is Failure.OtherUser -> "${failure.name} is logged in to this Mac."
        is Failure.Unreachable -> "Can't reach ${host.address}. Check the address and that Screen Sharing is turned on."
        is Failure.Unsupported -> "This Mac doesn't support High Performance screen sharing."
        is Failure.NoMedia -> "The Mac couldn't start a High Performance connection. A slow or unreliable network can cause this."
        is Failure.Other -> "The connection ended unexpectedly."
    }

    private enum class Tab(val title: String, val icon: ImageVector) {
        SAVED("Saved", Icons.Outlined.Bookmarks),
        NETWORK("Network", Icons.Outlined.Lan),
        SETTINGS("Settings", Icons.Outlined.Settings),
    }
}
