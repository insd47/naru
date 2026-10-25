package dev.insd.naru

import android.app.Activity
import android.view.InputDevice
import androidx.compose.foundation.layout.Box
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.outlined.Apps
import androidx.compose.material.icons.outlined.CallToAction
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.SpaceDashboard
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import dev.insd.naru.core.Dock

/**
 * The session's title bar: back to the connections, the Mac's name, the buttons of Screen Sharing's toolbar, and More.
 *
 * - The toolbar buttons stand in for the three-finger gestures Android keeps for itself. App Windows is left out, as in
 *   Screen Sharing's toolbar, since Mission Control shows the same windows.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TitleBar(
    title: String,
    onBack: () -> Unit,
    onDock: (Dock) -> Unit,
    onDisconnect: () -> Unit,
    modifier: Modifier = Modifier,
) {
    TopAppBar(
        title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        modifier = modifier,
        navigationIcon = {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Connections") }
        },
        actions = {
            IconButton(onClick = { onDock(Dock.APPS) }) { Icon(Icons.Outlined.Apps, "Apps") }
            IconButton(onClick = { onDock(Dock.MISSION_CONTROL) }) { Icon(Icons.Outlined.SpaceDashboard, "Mission Control") }
            IconButton(onClick = { onDock(Dock.DESKTOP) }) { Icon(Icons.Outlined.CallToAction, "Desktop") }

            var more by remember { mutableStateOf(false) }
            Box {
                IconButton(onClick = { more = true }) { Icon(Icons.Filled.MoreVert, "More") }
                DropdownMenu(expanded = more, onDismissRequest = { more = false }) {
                    DropdownMenuItem(
                        text = { Text("Disconnect") },
                        onClick = {
                            more = false
                            onDisconnect()
                        },
                        leadingIcon = { Icon(Icons.Outlined.Close, null) },
                    )
                }
            }
        },
    )
}

/**
 * Whether the title bar hides over a screen it fills: only with a mouse or touchpad to bring it down, and not in a
 * window (split screen, desktop windowing).
 */
fun Activity.titleBarHides(): Boolean = !isInMultiWindowMode && pointing()

/** Whether a mouse or touchpad is connected. A virtual device (the emulator's, or one injecting input) is not one the user points with. */
fun pointing(): Boolean = InputDevice.getDeviceIds().any { id ->
    InputDevice.getDevice(id)?.run { !isVirtual && supportsSource(InputDevice.SOURCE_MOUSE) } == true
}
