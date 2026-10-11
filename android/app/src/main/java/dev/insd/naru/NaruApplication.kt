package dev.insd.naru

import android.app.Application
import androidx.compose.material3.ComposeMaterial3Flags
import androidx.compose.material3.ExperimentalMaterial3Api

/** 마우스나 터치패드를 쓰면 Material 컴포넌트를 더 촘촘한 크기로 그리게 합니다. 이 앱은 포인터로 쓰는 일이 많습니다. */
class NaruApplication : Application() {
    @OptIn(ExperimentalMaterial3Api::class)
    override fun onCreate() {
        super.onCreate()
        ComposeMaterial3Flags.isPrecisionPointerComponentSizingEnabled = true
    }
}
