package dev.insd.naru

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import org.json.JSONObject

/**
 * Third-party notices: the Gradle dependencies' licenses, collected by the AboutLibraries plugin at build time, then the
 * Rust core's notices, generated with cargo-about.
 */
class LicensesActivity : ComponentActivity() {
    @OptIn(ExperimentalMaterial3Api::class)
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)

        val sections = libraries() + core()

        setContent {
            NaruTheme {
                Scaffold(
                    topBar = {
                        TopAppBar(
                            title = { Text("Open source licenses") },
                            navigationIcon = {
                                IconButton(onClick = ::finish) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") }
                            },
                        )
                    },
                ) { padding ->
                    LazyColumn(contentPadding = padding) {
                        items(sections) { (title, body) ->
                            Text(title, Modifier.padding(start = 16.dp, end = 16.dp, top = 24.dp), style = MaterialTheme.typography.titleMedium)
                            Text(
                                body,
                                Modifier.padding(16.dp),
                                style = MaterialTheme.typography.bodySmall,
                                fontFamily = FontFamily.Monospace,
                            )
                        }
                    }
                }
            }
        }
    }

    // One section per license, listing the libraries under it, then its text.
    private fun libraries(): List<Pair<String, String>> {
        val json = resources.openRawResource(R.raw.aboutlibraries).bufferedReader().use { JSONObject(it.readText()) }
        val licenses = json.getJSONObject("licenses")
        val libraries = json.getJSONArray("libraries").let { array -> (0 until array.length()).map(array::getJSONObject) }

        return licenses.keys().asSequence().map { id ->
            val license = licenses.getJSONObject(id)
            val users = libraries
                .filter { library -> library.optJSONArray("licenses")?.let { ids -> (0 until ids.length()).any { ids.getString(it) == id } } == true }
                .joinToString("\n") { "- ${it.optString("name", it.getString("uniqueId"))} ${it.optString("artifactVersion")}" }
            license.getString("name") to "Used by:\n$users\n\n${license.optString("content")}"
        }.toList()
    }

    private fun core(): List<Pair<String, String>> {
        val text = assets.open("core.txt").bufferedReader().use { it.readText() }
        return listOf("Naru core" to text.trim())
    }
}
