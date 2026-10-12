package dev.insd.naru

import android.app.Application
import android.content.Context
import androidx.compose.material3.ComposeMaterial3Flags
import androidx.compose.material3.ExperimentalMaterial3Api
import dev.insd.naru.home.Hosts
import dev.insd.naru.home.Keychain
import dev.insd.naru.home.ServerKeys

/** Log tag shared by every log line in the app */
const val TAG = "Naru"

/**
 * Holds the stores that exist once per app process.
 *
 * - The list screen and the session service share the same stores, so a save on one side is visible to the other without rereading. All of them are modified only on the main thread.
 * - With a mouse or touchpad attached, Material components draw at a denser size, since this app is often used with a pointer.
 */
class NaruApplication : Application() {
    val hosts by lazy { Hosts(this) }
    val keychain by lazy { Keychain(this) }
    val serverKeys by lazy { ServerKeys(this) }
    val preferences by lazy { Preferences(this) }

    @OptIn(ExperimentalMaterial3Api::class)
    override fun onCreate() {
        super.onCreate()
        ComposeMaterial3Flags.isPrecisionPointerComponentSizingEnabled = true
    }
}

/** This app's [NaruApplication] */
val Context.naru: NaruApplication
    get() = applicationContext as NaruApplication
