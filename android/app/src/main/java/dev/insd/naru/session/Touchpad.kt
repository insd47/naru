package dev.insd.naru.session

import android.view.MotionEvent
import dev.insd.naru.core.Phase

/**
 * Translates touchpad two-finger swipes and pinches into Mac scroll and magnify.
 *
 * - Android delivers these gestures as classified touches with deltas in pixels. They are converted to dp and used as Mac points.
 * - The session handles momentum and the pointer position, so only the stage and deltas are passed on here.
 */
class Touchpad(private val connection: Connection, private val density: Float) {
    // Cumulative scale since the pinch began. Android gives a per-event ratio.
    private var scale = 1f

    /** Sends a touchpad gesture to the Mac and returns `true`. */
    fun handle(event: MotionEvent): Boolean {
        when (event.classification) {
            MotionEvent.CLASSIFICATION_TWO_FINGER_SWIPE -> swipe(event)
            MotionEvent.CLASSIFICATION_PINCH -> pinch(event)
            else -> return false
        }

        return true
    }

    // Android's swipe distance is positive when scrolling down and the Mac's when scrolling up, so the sign is flipped.
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

    // Also sums the historical samples batched into one event. Reading only the last value loses movement during fast motion.
    private fun sum(event: MotionEvent, axis: Int): Float = (0..event.historySize).sumOf { sample ->
        axis(event, axis, sample).toDouble()
    }.toFloat()

    // `sample` equal to historySize means the current value.
    private fun axis(event: MotionEvent, axis: Int, sample: Int): Float =
        if (sample < event.historySize) event.getHistoricalAxisValue(axis, sample) else event.getAxisValue(axis)
}
