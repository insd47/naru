package dev.insd.naru

import android.app.Application
import android.content.Context
import androidx.compose.material3.ComposeMaterial3Flags
import androidx.compose.material3.ExperimentalMaterial3Api
import dev.insd.naru.home.Hosts
import dev.insd.naru.home.Keychain
import dev.insd.naru.home.ServerKeys

/** 로그 태그. 앱의 모든 로그를 이 이름으로 모읍니다. */
const val TAG = "Naru"

/**
 * 앱 프로세스 하나에 하나씩 두는 저장소를 들고 있습니다.
 *
 * - 목록 화면과 세션 서비스가 같은 저장소를 보므로, 한쪽이 저장하면 다른 쪽이 다시 읽지 않아도 바뀐 값을 봅니다. 모두 메인 스레드에서만 고칩니다.
 * - 마우스나 터치패드를 쓰면 Material 컴포넌트를 더 촘촘한 크기로 그리게 합니다. 이 앱은 포인터로 쓰는 일이 많습니다.
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

/** 이 앱의 [NaruApplication] */
val Context.naru: NaruApplication
    get() = applicationContext as NaruApplication
