package dev.insd.naru.session

import android.view.MotionEvent
import dev.insd.naru.protocol.Phase

/**
 * 터치패드의 두 손가락 스와이프와 핀치를 Mac의 스크롤과 확대로 옮깁니다.
 *
 * - Android는 이 제스처를 분류가 붙은 터치로 보내며 이동량은 픽셀입니다. dp로 바꿔 Mac의 포인트로 씁니다.
 * - 관성과 포인터 위치는 세션이 처리하므로 여기서는 단계와 이동량만 옮깁니다.
 */
class Touchpad(private val connection: Connection, private val density: Float) {
    // 핀치를 시작한 뒤의 누적 배율. Android는 이벤트마다의 비율을 줍니다.
    private var scale = 1f

    /** 터치패드 제스처면 Mac에 보내고 `true`를 반환합니다. */
    fun handle(event: MotionEvent): Boolean {
        when (event.classification) {
            MotionEvent.CLASSIFICATION_TWO_FINGER_SWIPE -> swipe(event)
            MotionEvent.CLASSIFICATION_PINCH -> pinch(event)
            else -> return false
        }

        return true
    }

    // Android의 스와이프 거리는 아래로 스크롤하는 쪽이 양수이고, Mac은 위로 스크롤하는 쪽이 양수라 부호를 바꿉니다.
    private fun swipe(event: MotionEvent) {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> connection.scroll(Phase.BEGAN, 0f, 0f)
            MotionEvent.ACTION_MOVE -> {
                val dx = -sum(event, MotionEvent.AXIS_GESTURE_SCROLL_X_DISTANCE) / density
                val dy = -sum(event, MotionEvent.AXIS_GESTURE_SCROLL_Y_DISTANCE) / density
                connection.scroll(Phase.CHANGED, dx, dy)
            }
            MotionEvent.ACTION_UP -> connection.scroll(Phase.ENDED, 0f, 0f)
            MotionEvent.ACTION_CANCEL -> connection.scroll(Phase.CANCELLED, 0f, 0f)
        }
    }

    private fun pinch(event: MotionEvent) {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                scale = 1f
                connection.magnify(Phase.BEGAN, scale)
            }
            MotionEvent.ACTION_MOVE -> {
                for (sample in 0..event.historySize) {
                    scale *= axis(event, MotionEvent.AXIS_GESTURE_PINCH_SCALE_FACTOR, sample)
                }

                connection.magnify(Phase.CHANGED, scale)
            }
            MotionEvent.ACTION_UP -> connection.magnify(Phase.ENDED, scale)
            MotionEvent.ACTION_CANCEL -> connection.magnify(Phase.CANCELLED, scale)
        }
    }

    // 한 이벤트에 묶여 온 이전 표본까지 더합니다. 마지막 값만 읽으면 빠르게 움직일 때 이동을 잃습니다.
    private fun sum(event: MotionEvent, axis: Int): Float = (0..event.historySize).sumOf { sample ->
        axis(event, axis, sample).toDouble()
    }.toFloat()

    // `sample`이 historySize와 같으면 현재 값입니다.
    private fun axis(event: MotionEvent, axis: Int, sample: Int): Float =
        if (sample < event.historySize) event.getHistoricalAxisValue(axis, sample) else event.getAxisValue(axis)
}
