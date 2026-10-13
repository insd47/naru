package dev.insd.naru.session

import android.content.Context
import android.view.MotionEvent
import android.view.VelocityTracker
import android.view.ViewConfiguration
import dev.insd.naru.core.Phase

/**
 * Translates touchpad two-finger swipes and pinches into Mac scroll and magnify.
 *
 * - Android delivers these gestures as classified touches with deltas in pixels. They are converted to dp and used as Mac points.
 * - Fingers touching down are a scroll that may begin, as on a Mac trackpad: lifting without moving cancels it. Android 15
 *   and later also report fingers touching down after a swipe this way, to stop a fling.
 * - Android gives no fling either, so the swipe's release velocity starts [momentum], as views do with these touches.
 */
class Touchpad(context: Context, private val connection: Connection, private val momentum: Momentum) {
    private val density = context.resources.displayMetrics.density
    private val maximumVelocity = ViewConfiguration.get(context).scaledMaximumFlingVelocity.toFloat()
    private var tracker: VelocityTracker? = null

    // Whether the swipe has moved, so the Mac has heard it begin
    private var scrolling = false

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
    // The fingers' tracked position moves the other way from that distance, so its velocity has the Mac's sign.
    private fun swipe(event: MotionEvent) {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                momentum.stop()
                tracker?.recycle()
                tracker = VelocityTracker.obtain().apply { addMovement(event) }
                scrolling = false
                connection.scroll(Phase.MAY_BEGIN, 0f, 0f)
            }
            MotionEvent.ACTION_MOVE -> {
                tracker?.addMovement(event)
                val dx = -sum(event, MotionEvent.AXIS_GESTURE_SCROLL_X_DISTANCE) / density
                val dy = -sum(event, MotionEvent.AXIS_GESTURE_SCROLL_Y_DISTANCE) / density

                if (scrolling || dx != 0f || dy != 0f) {
                    connection.scroll(if (scrolling) Phase.CHANGED else Phase.BEGAN, dx, dy)
                    scrolling = true
                }
            }
            MotionEvent.ACTION_UP -> {
                val tracker = tracker ?: return
                this.tracker = null
                tracker.addMovement(event)
                tracker.computeCurrentVelocity(1000, maximumVelocity)

                if (scrolling) {
                    connection.scroll(Phase.ENDED, 0f, 0f)
                    momentum.start(tracker.xVelocity, tracker.yVelocity, 1 / density)
                } else {
                    connection.scroll(Phase.CANCELLED, 0f, 0f)
                }

                tracker.recycle()
            }
            MotionEvent.ACTION_CANCEL -> {
                tracker?.recycle()
                tracker = null
                connection.scroll(Phase.CANCELLED, 0f, 0f)
            }
        }
    }

    private fun pinch(event: MotionEvent) {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                momentum.stop()
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
