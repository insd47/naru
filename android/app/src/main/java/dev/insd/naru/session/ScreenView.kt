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
import dev.insd.naru.core.Event
import dev.insd.naru.core.TouchPhase
import kotlin.math.roundToInt

/**
 * Mac 화면을 그리고 포인터 입력을 Mac 좌표로 바꿔 보냅니다.
 *
 * - 뷰는 영상과 같은 비율로 놓이므로, 뷰 안의 위치를 레이아웃 픽셀 좌표로 비례해 옮기면 됩니다.
 * - 마우스는 버튼 상태를 그대로 보냅니다. 펜은 대면 왼쪽 버튼, 옆 버튼을 누른 채 대면 오른쪽 버튼이고, 띄우면 포인터만 움직입니다.
 * - 손가락 터치는 코어가 화면 전체를 트랙패드로 써서 커서를 상대 이동으로 옮깁니다. 터치패드 제스처는 [Touchpad]가 맡습니다.
 * - 커서 모양은 마우스와 펜이면 시스템 포인터 아이콘으로 바꿔 지연 없이 그립니다. 손가락 터치 중에는 시스템 포인터가 없으므로,
 *   코어가 알려 준 자리([setTouchCursor])에 직접 그립니다.
 */
@SuppressLint("ViewConstructor", "ClickableViewAccessibility")
class ScreenView(context: Context) : SurfaceView(context) {
    var connection: Connection? = null
        set(value) {
            field = value
            value?.attach(holder.surface.takeIf { it.isValid })
        }

    /** 레이아웃의 픽셀 크기. 오기 전에는 포인터를 보내지 않습니다. */
    var layoutSize: Pair<Int, Int>? = null

    /** 레이아웃의 논리 폭. 커서 이미지는 논리 좌표 크기로 오므로 이 폭을 기준으로 키웁니다. */
    var logicalWidth: Int? = null

    /** 키 입력 처리. IME보다 먼저 받아, One UI의 키보드가 한/영 키 등을 가로채지 않게 합니다. */
    var onKey: ((KeyEvent) -> Boolean)? = null

    private var buttons = 0

    // 직접 그릴 커서 비트맵과 핫스팟(뷰 픽셀), 손가락 터치로 옮긴 커서 자리(뷰 픽셀). 마우스나 펜을 쓰면 자리를 지웁니다.
    private var cursorBitmap: Bitmap? = null
    private var cursorHotspot = 0f to 0f
    private var touchCursor: Pair<Float, Float>? = null

    override fun dispatchKeyEventPreIme(event: KeyEvent): Boolean =
        onKey?.invoke(event) ?: super.dispatchKeyEventPreIme(event)

    init {
        isFocusable = true
        isFocusableInTouchMode = true
        setWillNotDraw(false)

        // 키를 누르면 터치 모드가 풀리며 포커스된 뷰에 흰 강조가 그려집니다. 화면 위에 덮이지 않게 끕니다.
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

    /** 커서 모양을 화면 배율에 맞춰 시스템 포인터 아이콘으로 씁니다. 이미지와 핫스팟은 Mac의 논리 좌표 크기입니다. */
    fun setCursor(cursor: Event.Cursor) {
        val width = cursor.width.toInt()
        val height = cursor.height.toInt()
        val rgba = cursor.rgba
        val pixels = IntArray(width * height) { index ->
            val at = index * 4
            val channel = { offset: Int -> rgba[at + offset].toInt() and 0xff }
            (channel(3) shl 24) or (channel(0) shl 16) or (channel(1) shl 8) or channel(2)
        }

        // setPixels는 미리 곱하지 않은 색을 받습니다.
        val source = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
        source.setPixels(pixels, 0, width, 0, 0, width, height)

        val scale = logicalWidth?.let { this.width.toFloat() / it }?.takeIf { it > 0 } ?: 1f
        val scaledWidth = (width * scale).roundToInt().coerceAtLeast(1)
        val scaledHeight = (height * scale).roundToInt().coerceAtLeast(1)

        // 커서는 논리 크기 비트맵뿐이라 키워야 합니다. Apple 클라이언트처럼 늘 보간해 계단 없이 매끄럽게 키웁니다.
        val bitmap = Bitmap.createScaledBitmap(source, scaledWidth, scaledHeight, true)

        pointerIcon = PointerIcon.create(bitmap, cursor.hotspotX.toFloat() * scale, cursor.hotspotY.toFloat() * scale)
        cursorBitmap = bitmap
        cursorHotspot = cursor.hotspotX.toFloat() * scale to cursor.hotspotY.toFloat() * scale
        invalidate()
    }

    /** 손가락 터치로 옮긴 커서 자리(레이아웃 픽셀). 이 자리에 커서를 그립니다. */
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

    // 손가락마다 위치를 레이아웃 픽셀로 바꿔 코어에 넘깁니다. 화면 밖으로 나가도 상대 이동이므로 자르지 않습니다.
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

    // 마우스나 펜을 쓰면 시스템 포인터가 커서를 그리므로 직접 그리던 커서를 지웁니다.
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
            // Android는 오른쪽이 양수이고 Mac은 왼쪽이 양수입니다. 세로는 둘 다 위가 양수입니다.
            MotionEvent.ACTION_SCROLL -> connection?.wheel(
                -event.getAxisValue(MotionEvent.AXIS_HSCROLL),
                event.getAxisValue(MotionEvent.AXIS_VSCROLL),
            )
            MotionEvent.ACTION_BUTTON_PRESS, MotionEvent.ACTION_BUTTON_RELEASE -> {
                buttons = mouseButtons(event.buttonState)
                send(event)
            }
            else -> return false
        }

        return true
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
        // 서버의 버튼 비트는 표준 RFB와 달리 오른쪽이 1, 가운데가 2입니다.
        private const val BUTTON_LEFT = 1 shl 0
        private const val BUTTON_RIGHT = 1 shl 1
        private const val BUTTON_MIDDLE = 1 shl 2
    }
}
