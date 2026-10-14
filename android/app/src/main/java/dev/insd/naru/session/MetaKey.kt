package dev.insd.naru.session

import android.app.Activity
import android.content.ComponentName
import android.util.Log
import dev.insd.naru.TAG

/**
 * Requests that the app receive the Meta key on Samsung devices.
 *
 * - One UI takes the Meta key for system shortcuts first. Requesting it through the Samsung framework's `SemWindowManager.requestMetaKeyEvent`
 *   delivers it to this activity. It is not in the public SDK, so it is called through reflection and does nothing on devices without it.
 */
object MetaKey {
    fun request(activity: Activity, enabled: Boolean) {
        runCatching {
            val manager = Class.forName("com.samsung.android.view.SemWindowManager")
            val instance = manager.getMethod("getInstance").invoke(null)
            manager.getMethod("requestMetaKeyEvent", ComponentName::class.java, Boolean::class.javaPrimitiveType)
                .invoke(instance, activity.componentName, enabled)
        }.onSuccess {
            Log.i(TAG, "Meta key request: $enabled")
        }.onFailure {
            Log.i(TAG, "Cannot request the Meta key: $it")
        }
    }
}
