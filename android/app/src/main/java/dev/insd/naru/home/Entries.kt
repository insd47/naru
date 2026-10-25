package dev.insd.naru.home

import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DesktopMac
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material3.DropdownMenuGroup
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenuPopup
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.SegmentedListItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
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
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.util.Locale
import kotlin.math.roundToInt

/** One menu item for a row */
class Action(val label: String, val icon: ImageVector, val run: () -> Unit)

/**
 * A row for one Mac. Tapping connects, and hovering shows a Connect button in place of the last connection time.
 *
 * - While connecting to another Mac, [enabled] is off so the row cannot be tapped.
 * - A Mac in session always shows a Disconnect button through [onDisconnect], with a progress indicator while connecting ([connecting]).
 * - A long press or right click opens [menu] at the pressed point, with spacing between groups.
 */
@Composable
fun Entry(
    index: Int,
    count: Int,
    title: String,
    subtitle: String?,
    detail: String?,
    onConnect: () -> Unit,
    menu: List<List<Action>>,
    enabled: Boolean = true,
    connecting: Boolean = false,
    onDisconnect: (() -> Unit)? = null,
    onInfo: (() -> Unit)? = null,
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
            colors = itemColors(),
            contentPadding = ItemPadding,
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
            // The item puts 16dp after the icon; 8dp more matches the item's side padding.
            leadingContent = { Icon(Icons.Outlined.DesktopMac, contentDescription = null, Modifier.padding(end = 8.dp)) },
            supportingContent = subtitle?.let { { Text(it) } },
            trailingContent = {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    when {
                        onDisconnect != null -> {
                            if (connecting) {
                                @OptIn(ExperimentalMaterial3ExpressiveApi::class)
                                LoadingIndicator(Modifier.size(32.dp))
                            }

                            FilledTonalButton(onClick = onDisconnect) { Text("Disconnect") }
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

/** Colors of a grouped item: the brightest surface, which stands out from the page in light and dark themes alike, as in the system Settings. */
@Composable
fun itemColors() = ListItemDefaults.segmentedColors(containerColor = MaterialTheme.colorScheme.surfaceBright)

/** Padding inside a grouped item: the default top and bottom, with wider sides as in the system Settings. */
val ItemPadding = PaddingValues(
    start = 24.dp,
    top = ListItemDefaults.ContentPadding.calculateTopPadding(),
    end = 24.dp,
    bottom = ListItemDefaults.ContentPadding.calculateBottomPadding(),
)

/** Shows items as one group with joined corners. */
@Composable
fun Group(content: LazyListScope.() -> Unit) {
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(ListItemDefaults.SegmentedGap),
        content = content,
    )
}

/** Centered message shown when there are no items */
@Composable
fun Empty(icon: ImageVector, message: String) {
    Column(
        Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(icon, contentDescription = null, Modifier.size(64.dp), tint = MaterialTheme.colorScheme.outline)
        Text(message, style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.outline)
    }
}

/** Formats as "Today at 20:48" or "Aug 29, 2026 at 20:03", as the Screen Sharing app does. */
fun formatDate(epochMs: Long): String {
    val time = Instant.ofEpochMilli(epochMs).atZone(ZoneId.systemDefault())
    val today = LocalDate.now()
    val clock = time.format(DateTimeFormatter.ofPattern("HH:mm"))

    return when (time.toLocalDate()) {
        today -> "Today at $clock"
        today.minusDays(1) -> "Yesterday at $clock"
        else -> "${time.format(DateTimeFormatter.ofPattern("MMM d, yyyy", Locale.ENGLISH))} at $clock"
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
