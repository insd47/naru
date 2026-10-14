package dev.insd.naru

import android.content.Intent
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import kotlinx.coroutines.launch
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.unit.dp
import dev.insd.naru.home.Group

/** Settings screen shared by every Mac. The key mapping is an on/off toggle as in Parsec. */
@Composable
fun Settings(preferences: Preferences) {
    val context = LocalContext.current
    val uriHandler = LocalUriHandler.current
    val scope = rememberCoroutineScope()

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

        item {
            SegmentedListItem(
                onClick = { preferences.syncKeyboardLanguage = !preferences.syncKeyboardLanguage },
                shapes = ListItemDefaults.segmentedShapes(0, 2),
                colors = ListItemDefaults.segmentedColors(),
                modifier = Modifier.semantics {
                    role = Role.Switch
                    toggleableState = ToggleableState(preferences.syncKeyboardLanguage)
                },
                supportingContent = {
                    Text("The Mac switches its input source with the tablet's language. Works best with one language besides English on the Mac.")
                },
                trailingContent = { Switch(checked = preferences.syncKeyboardLanguage, onCheckedChange = null) },
            ) { Text("Sync Keyboard Language") }
        }

        item { Section("Help") }

        item {
            SegmentedListItem(
                onClick = { scope.launch { Diagnostics.share(context) } },
                shapes = ListItemDefaults.segmentedShapes(0, 1),
                colors = ListItemDefaults.segmentedColors(),
                supportingContent = { Text("Saves this launch's app log as a text file to share with a bug report. It has no passwords, typed text, or clipboard contents.") },
            ) { Text("Export Diagnostics") }
        }

        item { Section("About") }

        item {
            SegmentedListItem(
                onClick = { context.startActivity(Intent(context, LicensesActivity::class.java)) },
                shapes = ListItemDefaults.segmentedShapes(0, 2),
                colors = ListItemDefaults.segmentedColors(),
            ) { Text("Open source licenses") }
        }

        item {
            SegmentedListItem(
                onClick = { uriHandler.openUri("https://github.com/insd47/naru/blob/main/PRIVACY.md") },
                shapes = ListItemDefaults.segmentedShapes(1, 2),
                colors = ListItemDefaults.segmentedColors(),
            ) { Text("Privacy policy") }
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
