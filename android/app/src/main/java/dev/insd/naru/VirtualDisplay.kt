package dev.insd.naru

import android.app.Activity
import android.os.Build
import android.view.WindowInsets
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.TopAppBarDefaults
import dev.insd.naru.core.DisplayMode
import kotlin.math.roundToInt

/** A virtual display matching this pixel size. Scale 2 makes the Mac render at Retina while the pixel size stays as given. */
fun virtualDisplay(widthPx: Int, heightPx: Int): DisplayMode =
    DisplayMode(width = (widthPx / 2).toUInt(), height = (heightPx / 2).toUInt(), scale = 2.0)

/**
 * A virtual display sized to the session screen this window will show, used before the session screen opens.
 *
 * - Where the title bar stays, the session screen sits below it and the status bar. Sizing for the whole window would
 *   have the Mac resize its display as soon as the screen appears.
 * - Before Android 11 the window's insets are unknown, so the session screen measures the area it draws and resizes
 *   again.
 */
@OptIn(ExperimentalMaterial3Api::class)
fun Activity.virtualDisplay(): DisplayMode {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
        return resources.displayMetrics.let { virtualDisplay(it.widthPixels, it.heightPixels) }
    }

    val metrics = windowManager.currentWindowMetrics
    var height = metrics.bounds.height()

    if (!titleBarHides()) {
        val bar = TopAppBarDefaults.TopAppBarExpandedHeight.value * resources.displayMetrics.density
        height -= metrics.windowInsets.getInsetsIgnoringVisibility(WindowInsets.Type.statusBars()).top + bar.roundToInt()
    }

    return virtualDisplay(metrics.bounds.width(), height)
}
