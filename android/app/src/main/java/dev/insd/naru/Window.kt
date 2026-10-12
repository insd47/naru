package dev.insd.naru

import android.app.Activity
import android.os.Build
import dev.insd.naru.core.DisplayMode

/** A virtual display matching this pixel size. Scale 2 makes the Mac render at Retina while the pixel size stays as given. */
fun virtualDisplay(widthPx: Int, heightPx: Int): DisplayMode =
    DisplayMode(width = (widthPx / 2).toUInt(), height = (heightPx / 2).toUInt(), scale = 2.0)

/**
 * A virtual display matching this window's full size, used before the session screen opens.
 *
 * - In windows that cannot hide the system bars, such as split view, this is larger than the drawable area. The session screen measures the area it draws and resizes again.
 */
fun Activity.virtualDisplay(): DisplayMode {
    val (width, height) = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
        windowManager.currentWindowMetrics.bounds.let { it.width() to it.height() }
    } else {
        resources.displayMetrics.let { it.widthPixels to it.heightPixels }
    }

    return virtualDisplay(width, height)
}
