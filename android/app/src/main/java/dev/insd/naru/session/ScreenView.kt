package dev.insd.naru.session

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.PointerIcon
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewConfiguration
import dev.insd.naru.core.Event
import dev.insd.naru.core.TouchPhase
import kotlin.math.roundToInt

/**
 * Draws the Mac screen and sends pointer input converted to Mac coordinates.
 *
 * - The view has the video's aspect ratio, so a position in the view scales proportionally to layout pixels.
 * - A mouse sends its button state as is. A pen touching is the left button, touching with the side button held is the right button, and hovering only moves the pointer.
 * - For finger touches the core uses the whole screen as a trackpad and moves the cursor relatively. [Touchpad] handles touchpad gestures.
 * - With a mouse or pen, the cursor shape becomes the system pointer icon so it draws without lag. Finger touches have no system pointer, so
 *   the view draws it at the position the core reports ([setTouchCursor]).
 */
@SuppressLint("ViewConstructor", "ClickableViewAccessibility")
class ScreenView(context: Context) : SurfaceView(context) {
    var connection: Connection? = null
        set(value) {
            field = value
            value?.attach(holder.surface.takeIf { it.isValid })
        }

    /** Pixel size of the layout. No pointer events are sent until it arrives. */
    var layoutSize: Pair<Int, Int>? = null

    /** Logical width of the layout. Cursor images arrive at logical size, so they are scaled relative to this width. */
    var logicalWidth: Int? = null

    /** Key input handler. It runs before the IME so the One UI keyboard cannot intercept keys such as Korean/English. */
    var onKey: ((KeyEvent) -> Boolean)? = null

    private var buttons = 0

    // Cursor bitmap and hotspot to draw (view pixels), and the cursor position moved by finger touches (view pixels). Using a mouse or pen clears the position.
    private var cursorBitmap: Bitmap? = null
    private var cursorHotspot = 0f to 0f
    private var touchCursor: Pair<Float, Float>? = null

    override fun dispatchKeyEventPreIme(event: KeyEvent): Boolean =
        onKey?.invoke(event) ?: super.dispatchKeyEventPreIme(event)

    init {
        isFocusable = true
        isFocusableInTouchMode = true
        setWillNotDraw(false)

        // A key press exits touch mode and draws a white focus highlight on the focused view. Turned off so it does not cover the screen.
        defaultFocusHighlightEnabled = false

        holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) {
                connection?.attach(holder.surface)
            }

            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) = Unit

            override fun surfaceDestroyed(holder: SurfaceHolder) {
                connection?.attach(null)
            }
        })

        setOnTouchListener { _, event -> touch(event) }
        setOnGenericMotionListener { _, event -> motion(event) }
    }

    /** Uses the cursor shape as the system pointer icon, scaled to the screen. The image and hotspot are in the Mac's logical size. */
    fun setCursor(cursor: Event.Cursor) {
        val width = cursor.width.toInt()
        val height = cursor.height.toInt()
        val rgba = cursor.rgba
        val pixels = IntArray(width * height) { index ->
            val at = index * 4
            val channel = { offset: Int -> rgba[at + offset].toInt() and 0xff }
            (channel(3) shl 24) or (channel(0) shl 16) or (channel(1) shl 8) or channel(2)
        }

        // setPixels takes non-premultiplied colors.
        val source = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        source.setPixels(pixels, 0, width, 0, 0, width, height)

        val scale = logicalWidth?.let { this.width.toFloat() / it }?.takeIf { it > 0 } ?: 1f
        val scaledWidth = (width * scale).roundToInt().coerceAtLeast(1)
        val scaledHeight = (height * scale).roundToInt().coerceAtLeast(1)

        // The cursor only comes as a logical-size bitmap, so it must be scaled up. Like Apple's client, it is always filtered so it scales smoothly without jaggies.
        val bitmap = Bitmap.createScaledBitmap(source, scaledWidth, scaledHeight, true)

        pointerIcon = PointerIcon.create(bitmap, cursor.hotspotX.toFloat() * scale, cursor.hotspotY.toFloat() * scale)
        cursorBitmap = bitmap
        cursorHotspot = cursor.hotspotX.toFloat() * scale to cursor.hotspotY.toFloat() * scale
        invalidate()
    }

    /** Cursor position moved by finger touches (layout pixels), where the cursor is drawn. */
    fun setTouchCursor(x: Int, y: Int) {
        val (layoutWidth, layoutHeight) = layoutSize ?: return
        touchCursor = x.toFloat() / layoutWidth * width to y.toFloat() / layoutHeight * height
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val bitmap = cursorBitmap ?: return
        val (x, y) = touchCursor ?: return
        canvas.drawBitmap(bitmap, x - cursorHotspot.first, y - cursorHotspot.second, null)
    }

    private fun touch(event: MotionEvent): Boolean {
        requestFocus()

        if (event.getToolType(0) == MotionEvent.TOOL_TYPE_FINGER && !event.isFromSource(InputDevice.SOURCE_MOUSE)) {
            finger(event)
            return true
        }

        clearTouchCursor()

        val touching = event.actionMasked == MotionEvent.ACTION_DOWN || event.actionMasked == MotionEvent.ACTION_MOVE

        buttons = when {
            event.isFromSource(InputDevice.SOURCE_MOUSE) -> mouseButtons(event.buttonState)
            !touching -> 0
            event.buttonState and MotionEvent.BUTTON_STYLUS_PRIMARY != 0 -> BUTTON_RIGHT
            else -> BUTTON_LEFT
        }

        send(event)
        return true
    }

    // Converts each finger's position to layout pixels for the core. Positions off screen are not clamped, since movement is relative.
    private fun finger(event: MotionEvent) {
        val current = connection ?: return
        val (layoutWidth, layoutHeight) = layoutSize ?: return

        if (width == 0 || height == 0) {
            return
        }

        val send = { index: Int, phase: TouchPhase ->
            val x = event.getX(index) / width * layoutWidth
            val y = event.getY(index) / height * layoutHeight
            current.touch(event.getPointerId(index), phase, x, y)
        }

        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> send(event.actionIndex, TouchPhase.BEGAN)
            MotionEvent.ACTION_MOVE -> for (index in 0 until event.pointerCount) send(index, TouchPhase.MOVED)
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> send(event.actionIndex, TouchPhase.ENDED)
            MotionEvent.ACTION_CANCEL -> for (index in 0 until event.pointerCount) send(index, TouchPhase.CANCELLED)
        }
    }

    // With a mouse or pen the system pointer draws the cursor, so the self-drawn cursor is cleared.
    private fun clearTouchCursor() {
        if (touchCursor != null) {
            touchCursor = null
            invalidate()
        }
    }

    private fun motion(event: MotionEvent): Boolean {
        clearTouchCursor()

        when (event.actionMasked) {
            MotionEvent.ACTION_HOVER_MOVE, MotionEvent.ACTION_HOVER_ENTER -> {
                buttons = mouseButtons(event.buttonState)
                send(event)
            }
            MotionEvent.ACTION_SCROLL -> wheel(event)
            MotionEvent.ACTION_BUTTON_PRESS, MotionEvent.ACTION_BUTTON_RELEASE -> {
                buttons = mouseButtons(event.buttonState)
                send(event)
            }
            else -> return false
        }

        return true
    }

    // A notch scrolls ViewConfiguration's scroll factor, which goes to the Mac as a distance in its points. Android has no
    // line count, and that factor is about three text lines, so three lines go with it. There is no acceleration.
    // Android is positive to the right and the Mac to the left. Vertically, both are positive upward.
    private fun wheel(event: MotionEvent) {
        val notchesX = -event.getAxisValue(MotionEvent.AXIS_HSCROLL)
        val notchesY = event.getAxisValue(MotionEvent.AXIS_VSCROLL)
        val configuration = ViewConfiguration.get(context)
        val points = logicalWidth?.takeIf { width > 0 }?.let { it.toFloat() / width } ?: 1f

        connection?.wheel(
            notchesX * configuration.scaledHorizontalScrollFactor * points,
            notchesY * configuration.scaledVerticalScrollFactor * points,
            notchesX * LINES_PER_NOTCH,
            notchesY * LINES_PER_NOTCH,
        )
    }

    private fun send(event: MotionEvent) {
        val current = connection ?: return
        val (layoutWidth, layoutHeight) = layoutSize ?: return

        if (width == 0 || height == 0) {
            return
        }

        val x = (event.x / width * layoutWidth).toInt().coerceIn(0, layoutWidth - 1)
        val y = (event.y / height * layoutHeight).toInt().coerceIn(0, layoutHeight - 1)
        current.pointer(buttons, x, y)
    }

    private fun mouseButtons(state: Int): Int {
        var mask = 0
        if (state and MotionEvent.BUTTON_PRIMARY != 0) mask = mask or BUTTON_LEFT
        if (state and MotionEvent.BUTTON_SECONDARY != 0) mask = mask or BUTTON_RIGHT
        if (state and MotionEvent.BUTTON_TERTIARY != 0) mask = mask or BUTTON_MIDDLE
        return mask
    }

    companion object {
        // Unlike standard RFB, the server's button bits are 1 for right and 2 for middle.
        private const val BUTTON_LEFT = 1 shl 0
        private const val BUTTON_RIGHT = 1 shl 1
        private const val BUTTON_MIDDLE = 1 shl 2

        private const val LINES_PER_NOTCH = 3f
    }
}
