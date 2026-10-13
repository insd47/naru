import AVFoundation
import UIKit

/// Draws the Mac's screen and sends keyboard and pointer input to the Mac.
///
/// - Video is drawn by attaching [Video]'s layer. The system pointer is hidden and the cursor bitmap the Mac sent is drawn at its position.
/// - Pointers (trackpad, mouse, Apple Pencil) send their position to the Mac. For finger touches the core uses the whole screen as a trackpad and moves the cursor relatively,
///   and the cursor is drawn where [touchCursor] reports.
/// - The view accepts text input so iPadOS input language switching (Caps Lock, Globe key) works, but character keys are sent directly instead of through the input method.
/// - After a size change it waits briefly, then reports the virtual display size through [onResize].
final class ScreenView: UIView, UIKeyInput, UIPointerInteractionDelegate {
    var onResize: (DisplayMode) -> Void = { _ in }

    var connection: Connection? {
        didSet {
            guard connection !== oldValue else { return }
            oldValue?.video.layer.removeFromSuperlayer()
            if let layer = connection?.video.layer { self.layer.insertSublayer(layer, at: 0) }
            setNeedsLayout()
        }
    }

    var layout: SessionController.Layout? {
        didSet { setNeedsLayout() }
    }

    var cursor: SessionController.Cursor? {
        didSet { placeCursor() }
    }

    /// The cursor position moved by finger touches, in layout pixels
    var touchCursor: CGPoint? {
        didSet {
            guard let touchCursor, touchCursor != oldValue, let layout else { return }
            let rect = videoRect(layout)
            pointer = CGPoint(
                x: rect.minX + touchCursor.x / layout.pixels.width * rect.width,
                y: rect.minY + touchCursor.y / layout.pixels.height * rect.height
            )
            placeCursor()
        }
    }

    private let cursorLayer = CALayer()
    private var keys = Keys()
    private var buttons: UInt8 = 0
    private var pointer: CGPoint?
    private var resizing: DispatchWorkItem?
    private var measured = CGSize.zero

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .black
        isMultipleTouchEnabled = true
        cursorLayer.zPosition = 1
        layer.addSublayer(cursorLayer)

        addInteraction(UIPointerInteraction(delegate: self))
        addGestureRecognizer(UIHoverGestureRecognizer(target: self, action: #selector(hover)))

        let scroll = UIPanGestureRecognizer(target: self, action: #selector(scroll))
        scroll.allowedScrollTypesMask = .continuous
        scroll.allowedTouchTypes = []
        addGestureRecognizer(scroll)

        let wheel = UIPanGestureRecognizer(target: self, action: #selector(wheel))
        wheel.allowedScrollTypesMask = .discrete
        wheel.allowedTouchTypes = []
        addGestureRecognizer(wheel)

        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(pinch))
        pinch.allowedTouchTypes = [UITouch.TouchType.indirectPointer.rawValue as NSNumber]
        addGestureRecognizer(pinch)
    }

    required init?(coder: NSCoder) {
        fatalError("created in code only")
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        connection?.video.layer.frame = bounds
        placeCursor()

        if bounds.size != measured, bounds.width > 0, bounds.height > 0 {
            measured = bounds.size
            resizing?.cancel()
            let mode = DisplayMode(width: UInt32(bounds.width.rounded()), height: UInt32(bounds.height.rounded()), scale: 2)
            let work = DispatchWorkItem { [weak self] in self?.onResize(mode) }
            resizing = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: work)
        }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()

        if window != nil {
            becomeFirstResponder()
        } else {
            keys.releaseAll().forEach { connection?.key($0, down: false) }
        }
    }

    // MARK: Keyboard

    override var canBecomeFirstResponder: Bool { true }

    // Never shows the on-screen keyboard; only hardware keyboards are accepted.
    override var inputView: UIView? { blank }
    private let blank = UIView()

    var hasText: Bool { false }
    func insertText(_ text: String) {}
    func deleteBackward() {}

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        var system = Set<UIPress>()

        for press in presses {
            guard let key = press.key else { continue }

            if Keys.isSystem(key) {
                system.insert(press)
            } else if let keysym = keys.press(key) {
                connection?.key(keysym, down: true)
            }
        }

        if !system.isEmpty { super.pressesBegan(system, with: event) }
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        release(presses, with: event) { super.pressesEnded($0, with: $1) }
    }

    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        release(presses, with: event) { super.pressesCancelled($0, with: $1) }
    }

    private func release(_ presses: Set<UIPress>, with event: UIPressesEvent?, system pass: (Set<UIPress>, UIPressesEvent?) -> Void) {
        var system = Set<UIPress>()

        for press in presses {
            guard let key = press.key else { continue }

            if Keys.isSystem(key) {
                system.insert(press)
            } else if let keysym = keys.release(key) {
                connection?.key(keysym, down: false)
            }
        }

        if !system.isEmpty { pass(system, event) }
    }

    // MARK: Pointer

    func pointerInteraction(_ interaction: UIPointerInteraction, styleFor region: UIPointerRegion) -> UIPointerStyle? {
        .hidden()
    }

    @objc private func hover(_ recognizer: UIHoverGestureRecognizer) {
        move(to: recognizer.location(in: self))
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        point(touches, event: event)
        finger(touches, .began)
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        point(touches, event: event)
        finger(touches, .moved)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        point(touches, event: event, released: true)
        finger(touches, .ended)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        point(touches, event: event, released: true)
        finger(touches, .cancelled)
    }

    // Converts each finger's position to layout pixels for the core. Positions are not clamped to the screen because movement is relative.
    private func finger(_ touches: Set<UITouch>, _ phase: TouchPhase) {
        guard let connection, let layout else { return }
        let rect = videoRect(layout)

        for touch in touches where touch.type == .direct {
            let location = touch.location(in: self)
            let x = (location.x - rect.minX) / rect.width * layout.pixels.width
            let y = (location.y - rect.minY) / rect.height * layout.pixels.height
            connection.touch(UInt32(truncatingIfNeeded: ObjectIdentifier(touch).hashValue), phase, x: Float(x), y: Float(y))
        }
    }

    // Unlike standard RFB, the server's button bits are 2 for right and 4 for middle. The pencil is the left button.
    private func point(_ touches: Set<UITouch>, event: UIEvent?, released: Bool = false) {
        guard let touch = touches.first(where: { $0.type == .indirectPointer || $0.type == .pencil }) else {
            return
        }

        if released {
            buttons = 0
        } else if touch.type == .pencil {
            buttons = 1
        } else if let mask = event?.buttonMask {
            buttons = (mask.contains(.primary) ? 1 : 0) | (mask.contains(.secondary) ? 2 : 0) | (mask.contains(.button(3)) ? 4 : 0)
        }

        move(to: touch.location(in: self))
    }

    private func move(to location: CGPoint) {
        pointer = location
        placeCursor()

        guard let connection, let layout else {
            return
        }

        let rect = videoRect(layout)
        let x = ((location.x - rect.minX) / rect.width * layout.pixels.width).clamped(to: 0...(layout.pixels.width - 1))
        let y = ((location.y - rect.minY) / rect.height * layout.pixels.height).clamped(to: 0...(layout.pixels.height - 1))
        connection.pointer(buttons, x: UInt16(x), y: UInt16(y))
    }

    // Trackpad scrolling is converted to Mac points. The session handles momentum and pointer position.
    @objc private func scroll(_ recognizer: UIPanGestureRecognizer) {
        guard let connection else { return }

        let delta = recognizer.translation(in: self)
        recognizer.setTranslation(.zero, in: self)
        let scale = pointsPerViewPoint()

        switch recognizer.state {
        case .began:
            connection.scroll(.began, dx: 0, dy: 0)
        case .changed:
            connection.scroll(.changed, dx: Float(delta.x * scale), dy: Float(delta.y * scale))
        case .ended:
            connection.scroll(.ended, dx: 0, dy: 0)
        case .cancelled, .failed:
            connection.scroll(.cancelled, dx: 0, dy: 0)
        default:
            break
        }
    }

    // Mouse wheel scrolling sends the distance iPadOS reports, in the Mac's points. iPadOS has no line count and, like
    // macOS, moves 10 points a line, so the lines are that distance over 10.
    @objc private func wheel(_ recognizer: UIPanGestureRecognizer) {
        let delta = recognizer.translation(in: self)
        recognizer.setTranslation(.zero, in: self)
        let scale = pointsPerViewPoint()

        if recognizer.state == .changed {
            connection?.wheel(
                dx: Float(delta.x * scale), dy: Float(delta.y * scale),
                linesX: Float(delta.x / 10), linesY: Float(delta.y / 10)
            )
        }
    }

    @objc private func pinch(_ recognizer: UIPinchGestureRecognizer) {
        let scale = Float(recognizer.scale)

        switch recognizer.state {
        case .began: connection?.magnify(.began, scale: 1)
        case .changed: connection?.magnify(.changed, scale: scale)
        case .ended: connection?.magnify(.ended, scale: scale)
        case .cancelled, .failed: connection?.magnify(.cancelled, scale: scale)
        default: break
        }
    }

    // MARK: Cursor

    private func placeCursor() {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        defer { CATransaction.commit() }

        guard let cursor, let layout, let pointer else {
            cursorLayer.isHidden = true
            return
        }

        let scale = videoRect(layout).width / layout.logical.width
        cursorLayer.isHidden = false
        cursorLayer.contents = cursor.image
        cursorLayer.frame = CGRect(
            x: pointer.x - cursor.hotspot.x * scale, y: pointer.y - cursor.hotspot.y * scale,
            width: cursor.size.width * scale, height: cursor.size.height * scale
        )
    }

    private func videoRect(_ layout: SessionController.Layout) -> CGRect {
        AVMakeRect(aspectRatio: layout.pixels, insideRect: bounds)
    }

    private func pointsPerViewPoint() -> CGFloat {
        guard let layout else { return 1 }
        return layout.logical.width / videoRect(layout).width
    }
}

private extension CGFloat {
    func clamped(to range: ClosedRange<CGFloat>) -> CGFloat {
        Swift.min(Swift.max(self, range.lowerBound), range.upperBound)
    }
}
