package dev.insd.naru

import android.app.Activity
import android.os.Build
import dev.insd.naru.core.DisplayMode

/** 이 픽셀 크기에 맞춘 가상 디스플레이. 배율 2로 두어 Mac이 Retina로 그리고, 픽셀 크기는 주어진 크기와 같습니다. */
fun virtualDisplay(widthPx: Int, heightPx: Int): DisplayMode =
    DisplayMode(width = (widthPx / 2).toUInt(), height = (heightPx / 2).toUInt(), scale = 2.0)

/**
 * 이 창 전체 크기에 맞춘 가상 디스플레이. 세션 화면을 열기 전에 씁니다.
 *
 * - 스플릿 뷰처럼 시스템 막대를 숨길 수 없는 창에서는 실제로 그릴 영역보다 큽니다. 세션 화면이 그린 영역을 재서 다시 맞춥니다.
 */
fun Activity.virtualDisplay(): DisplayMode {
    val (width, height) = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
        windowManager.currentWindowMetrics.bounds.let { it.width() to it.height() }
    } else {
        resources.displayMetrics.let { it.widthPixels to it.heightPixels }
    }

    return virtualDisplay(width, height)
}
