package dev.insd.naru.session

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.PointerIcon
import android.view.SurfaceHolder
import android.view.SurfaceView
import dev.insd.naru.core.Event
import kotlin.math.roundToInt

/**
 * Mac 화면을 그리고 포인터 입력을 Mac 좌표로 바꿔 보냅니다.
 *
 * - 뷰는 영상과 같은 비율로 놓이므로, 뷰 안의 위치를 레이아웃 픽셀 좌표로 비례해 옮기면 됩니다.
 * - 마우스는 버튼 상태를 그대로 보냅니다. 펜은 대면 왼쪽 버튼, 옆 버튼을 누른 채 대면 오른쪽 버튼이고, 띄우면 포인터만 움직입니다.
 * - 프로토콜에 터치가 없으므로 손가락 터치는 보내지 않습니다. 터치패드 제스처는 [Touchpad]가 맡습니다.
 * - 커서 모양은 시스템 포인터 아이콘으로 바꿔 지연 없이 그립니다.
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

    override fun dispatchKeyEventPreIme(event: KeyEvent): Boolean =
        onKey?.invoke(event) ?: super.dispatchKeyEventPreIme(event)

    init {
        isFocusable = true
        isFocusableInTouchMode = true

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
    }

    private fun touch(event: MotionEvent): Boolean {
        requestFocus()

        // Mac에는 터치 입력이 없고 프로토콜도 포인터만 싣습니다. 손가락을 클릭으로 바꾸면 의도치 않은 클릭이 생기므로 버립니다.
        if (event.getToolType(0) == MotionEvent.TOOL_TYPE_FINGER && !event.isFromSource(InputDevice.SOURCE_MOUSE)) {
            return true
        }

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

    private fun motion(event: MotionEvent): Boolean {
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
