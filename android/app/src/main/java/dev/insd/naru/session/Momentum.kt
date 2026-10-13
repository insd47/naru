package dev.insd.naru.session

import android.content.Context
import android.view.Choreographer
import android.view.ViewConfiguration
import android.widget.OverScroller
import dev.insd.naru.core.MomentumPhase
import kotlin.math.abs

/**
 * Continues a scroll after the fingers lift with the platform's fling, and sends it to the Mac as momentum.
 *
 * - Android reports nothing after a touchpad or touch scroll ends. Views fling it themselves with [OverScroller] from the
 *   release velocity, stepped once per frame, and so does this, with the same thresholds and friction.
 * - Only one momentum runs at a time. Any new touch, scroll, or click stops it first.
 */
class Momentum(context: Context, private val send: (MomentumPhase, Float, Float) -> Unit) {
    private val scroller = OverScroller(context)
    private val choreographer = Choreographer.getInstance()
    private val minimumVelocity = ViewConfiguration.get(context).scaledMinimumFlingVelocity
    private val frame = Choreographer.FrameCallback { step() }

    private var running = false
    private var began = false
    private var lastX = 0
    private var lastY = 0
    private var pointsPerPixel = 1f

    /** Starts from a release velocity in pixels per second. [pointsPerPixel] converts the distance to Mac points. */
    fun start(velocityX: Float, velocityY: Float, pointsPerPixel: Float) {
        stop()

        if (abs(velocityX) < minimumVelocity && abs(velocityY) < minimumVelocity) {
            return
        }

        this.pointsPerPixel = pointsPerPixel
        lastX = 0
        lastY = 0
        scroller.fling(0, 0, velocityX.toInt(), velocityY.toInt(), Int.MIN_VALUE, Int.MAX_VALUE, Int.MIN_VALUE, Int.MAX_VALUE)
        running = true
        choreographer.postFrameCallback(frame)
    }

    /** Ends momentum in progress. The Mac hears the end only if momentum began. */
    fun stop() {
        if (!running) {
            return
        }

        scroller.abortAnimation()
        choreographer.removeFrameCallback(frame)
        finish()
    }

    // The scroller reads the frame's time, so each step moves by what this frame shows.
    private fun step() {
        if (!scroller.computeScrollOffset()) {
            finish()
            return
        }

        val dx = (scroller.currX - lastX) * pointsPerPixel
        val dy = (scroller.currY - lastY) * pointsPerPixel
        lastX = scroller.currX
        lastY = scroller.currY

        send(if (began) MomentumPhase.CHANGED else MomentumPhase.BEGAN, dx, dy)
        began = true
        choreographer.postFrameCallback(frame)
    }

    private fun finish() {
        if (began) send(MomentumPhase.ENDED, 0f, 0f)
        running = false
        began = false
    }
}
