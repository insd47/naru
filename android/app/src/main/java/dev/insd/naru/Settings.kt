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
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.unit.dp
import dev.insd.naru.home.Group
import kotlinx.coroutines.launch

/** Settings screen shared by every Mac */
@Composable
fun Settings(preferences: Preferences) {
    val context = LocalContext.current
    val uriHandler = LocalUriHandler.current
    val scope = rememberCoroutineScope()

    Group {
        item { Section("Keyboard") }

        item {
            SwitchItem(
                "Swap Alt and Command",
                "Alt sends Command and the Windows key sends Option, matching the keys on a Mac keyboard.",
                preferences.swapCommandOption,
                index = 0,
                count = 2,
            ) { preferences.swapCommandOption = it }
        }

        item {
            SwitchItem(
                "Sync Keyboard Language",
                "The Mac switches its input source with the tablet's language. Works best with one language besides English on the Mac.",
                preferences.syncKeyboardLanguage,
                index = 1,
                count = 2,
            ) { preferences.syncKeyboardLanguage = it }
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

// A list item that turns a setting on or off, which accessibility services read as a switch
@Composable
private fun SwitchItem(title: String, summary: String, checked: Boolean, index: Int, count: Int, onChange: (Boolean) -> Unit) {
    SegmentedListItem(
        onClick = { onChange(!checked) },
        shapes = ListItemDefaults.segmentedShapes(index, count),
        colors = ListItemDefaults.segmentedColors(),
        modifier = Modifier.semantics {
            role = Role.Switch
            toggleableState = ToggleableState(checked)
        },
        supportingContent = { Text(summary) },
        trailingContent = { Switch(checked = checked, onCheckedChange = null) },
    ) { Text(title) }
}
