package dev.insd.naru

import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.unit.dp
import dev.insd.naru.core.Priority
import dev.insd.naru.home.Group

/** Settings screen shared by every Mac. The key mapping is an on/off toggle as in Parsec, and quality is one of two choices as in the Screen Sharing app. */
@Composable
fun Settings(preferences: Preferences) {
    val qualities = listOf(
        Priority.LATENCY to ("Adapt to network conditions" to "Keeps the screen moving when packets are lost. Parts of it may look broken briefly."),
        Priority.QUALITY to ("Show the screen at full quality" to "Pauses on the last clean picture until the Mac sends a fresh one."),
    )

    Group {
        item { Section("Keyboard") }

        item {
            SegmentedListItem(
                onClick = { preferences.swapCommandOption = !preferences.swapCommandOption },
                shapes = ListItemDefaults.segmentedShapes(0, 2),
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
                shapes = ListItemDefaults.segmentedShapes(1, 2),
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
