package dev.insd.naru.session

import android.app.Activity
import android.util.Log

/**
 * Samsung 기기에서 Meta 키를 앱이 받도록 요청합니다.
 *
 * - One UI는 Meta 키를 시스템 단축키로 먼저 가져갑니다. Samsung 프레임워크의 `SemWindowManager.requestMetaKeyEvent`로
 *   요청하면 이 액티비티가 Meta 키를 받습니다. 공개 SDK에 없으므로 리플렉션으로 부르고, 없는 기기에서는 아무것도 하지 않습니다.
 */
object MetaKey {
    fun request(activity: Activity, enabled: Boolean) {
        runCatching {
            val manager = Class.forName("com.samsung.android.view.SemWindowManager")
            val instance = manager.getMethod("getInstance").invoke(null)
            manager.getMethod("requestMetaKeyEvent", android.content.ComponentName::class.java, Boolean::class.javaPrimitiveType)
                .invoke(instance, activity.componentName, enabled)
        }.onSuccess {
            Log.i("Naru", "Meta 키 요청: $enabled")
        }.onFailure {
            Log.i("Naru", "Meta 키를 요청할 수 없습니다: $it")
        }
    }
}
