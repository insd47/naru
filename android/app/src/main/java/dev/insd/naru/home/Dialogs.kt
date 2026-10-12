package dev.insd.naru.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ToggleButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp

/** 새 연결은 주소만 묻습니다. 사용자 이름과 비밀번호는 접속하면서 묻고, 성공하면 목록에 저장합니다. */
@Composable
fun AddDialog(onConnect: (address: String) -> Unit, onDismiss: () -> Unit) {
    var address by remember { mutableStateOf("") }
    val submit = { if (address.isNotEmpty()) onConnect(address) }

    AlertDialog(
        onDismissRequest = onDismiss,
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
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** 저장한 Mac의 이름, 주소, 사용자 이름, 화면을 고칩니다. 이름을 비우면 다음 접속 때 Mac의 컴퓨터 이름으로 채웁니다. */
@Composable
fun EditDialog(original: Host, onSave: (Host) -> Unit, onDismiss: () -> Unit) {
    var name by remember(original.id) { mutableStateOf(original.name) }
    var address by remember(original.id) { mutableStateOf(original.address) }
    var username by remember(original.id) { mutableStateOf(original.username) }
    var port by remember(original.id) { mutableStateOf(original.port.toString()) }
    var virtualDisplay by remember(original.id) { mutableStateOf(original.virtualDisplay) }
    val valid = address.isNotEmpty() && port.toIntOrNull() in 1..65535

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Edit “${original.title}”") },
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
                    val edited = original.copy(
                        name = name.trim(),
                        address = address,
                        username = username,
                        port = port.toInt(),
                        virtualDisplay = virtualDisplay,
                    )
                    onSave(edited)
                },
            ) { Text("Save") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
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
