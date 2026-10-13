package dev.insd.naru

import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Process
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.time.Instant

/**
 * Exports the app's log as a text file for a bug report.
 *
 * - The log is the system log of this process, read with logcat, so it covers this launch only, as far back as the
 *   system's ring buffer keeps it.
 * - Nothing the app logs carries passwords, keys, typed text, or clipboard contents.
 */
object Diagnostics {
    /** Writes the log with a header describing the app and device, and opens the share sheet for it. */
    suspend fun share(context: Context) {
        val file = withContext(Dispatchers.IO) { write(context) }
        val uri = FileProvider.getUriForFile(context, "${context.packageName}.diagnostics", file)
        val send = Intent(Intent.ACTION_SEND)
            .setType("text/plain")
            .putExtra(Intent.EXTRA_STREAM, uri)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)

        context.startActivity(Intent.createChooser(send, "Export Diagnostics").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }

    private fun write(context: Context): File {
        val info = context.packageManager.getPackageInfo(context.packageName, 0)
        val header = listOf(
            "Naru ${info.versionName} (${info.longVersionCode}), Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})",
            "${Build.MANUFACTURER} ${Build.MODEL}",
            "Exported ${Instant.now()}",
            "",
        )
        // The app's own lines, and only warnings and errors from the system libraries in this process (decoders, audio).
        val log = ProcessBuilder("logcat", "-d", "-v", "threadtime", "--pid", Process.myPid().toString(), "$TAG:V", "*:W")
            .redirectErrorStream(true)
            .start()
            .inputStream
            .bufferedReader()
            .use { it.readText() }

        val directory = File(context.cacheDir, "diagnostics").apply { mkdirs() }
        return File(directory, "naru-diagnostics.txt").apply { writeText(header.joinToString("\n") + log) }
    }
}
