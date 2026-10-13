import AVFoundation
import UIKit

/// Draws the Mac's screen and sends keyboard and pointer input to the Mac.
///
/// - Video is drawn by attaching [Video]'s layer. The system pointer is hidden and the cursor bitmap the Mac sent is drawn at its position.
/// - Pointers (trackpad, mouse, Apple Pencil) send their position to the Mac. Finger touches use the whole screen as a trackpad through the core's
///   [Touchpad], moving the cursor relatively.
/// - Scrolls continue with [Momentum] after the fingers lift, since iPadOS reports none.
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

    private let cursorLayer = CALayer()
    private var keys = Keys()
    // Presses also given to the text system, so their release goes there too
    private var typing = Set<UIKeyboardHIDUsage>()
    // The latest typing key still held, which the text system's repeats resend
    private var held: (keysym: UInt32, code: UIKeyboardHIDUsage, pressed: Date, repeated: Date?)?
    // The navigation key repeating on a timer, and the repeat timing to use
    private var navigation: (code: UIKeyboardHIDUsage, timer: Timer)?
    private var repeatTiming = (delay: 0.4, interval: 0.1)
    private var buttons: UInt8 = 0
    // Where the cursor is drawn in this view, and where the Mac's cursor is in layout pixels
    private var pointer: CGPoint?
    private var at = CGPoint.zero
    private let touchpad = Touchpad()
    private var touchButtons: UInt8 = 0
    // Watches two-finger touches only for their velocity when a touch scroll ends.
    private let fingers = UIPanGestureRecognizer()
    private lazy var momentum = Momentum { [weak self] phase, delta in
        self?.connection?.momentum(phase, dx: Float(delta.x), dy: Float(delta.y))
    }
    private var resizing: DispatchWorkItem?
    private var measured = CGSize.zero

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .black
        isMultipleTouchEnabled = true
        cursorLayer.zPosition = 1
        layer.addSublayer(cursorLayer)

        addInteraction(UIPointerInteraction(delegate: self))
        NotificationCenter.default.addObserver(self, selector: #selector(languageChanged), name: UITextInputMode.currentInputModeDidChangeNotification, object: nil)
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

        fingers.minimumNumberOfTouches = 2
        fingers.allowedTouchTypes = [UITouch.TouchType.direct.rawValue as NSNumber]
        fingers.cancelsTouchesInView = false
        fingers.delaysTouchesEnded = false
        addGestureRecognizer(fingers)
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
            navigation?.timer.invalidate()
            navigation = nil
            momentum.stop()
        }
    }

    // MARK: Keyboard

    override var canBecomeFirstResponder: Bool { true }

    override func becomeFirstResponder() -> Bool {
        let became = super.becomeFirstResponder()
        DiagnosticsLog.shared.record("FOCUS", "become \(became) key window \(window?.isKeyWindow == true), key is \(DiagnosticsLog.describe(DiagnosticsLog.keyWindow))")
        return became
    }

    override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        DiagnosticsLog.shared.record("FOCUS", "resign \(resigned)")
        return resigned
    }

    // Never shows the on-screen keyboard; only hardware keyboards are accepted.
    override var inputView: UIView? { blank }
    private let blank = UIView()

    var hasText: Bool { false }

    func insertText(_ text: String) {
        let keyWindow = "key window \(window?.isKeyWindow == true)"
        DiagnosticsLog.shared.record("TXT", DiagnosticsLog.shared.secure ? "insert (secure input) \(keyWindow)" : "insert [\(text)] \(keyWindow)")
        repeatHeld()
    }

    func deleteBackward() {
        DiagnosticsLog.shared.record("TXT", "delete")
        repeatHeld()
    }

    // The Mac does not repeat held keys. iPadOS reports a held key's repeats only as text input, at the user's repeat
    // settings, so each one resends the key. Input right after a press is the press itself, or Korean composition,
    // which delivers a deletion and an insertion together; repeats start well after it.
    private func repeatHeld() {
        guard var held, Date.now.timeIntervalSince(held.pressed) >= 0.15 else { return }
        defer { self.held = held }

        let burst = held.repeated.map { Date.now.timeIntervalSince($0) < 0.02 } ?? false

        if !burst {
            if let previous = held.repeated {
                repeatTiming.interval = Date.now.timeIntervalSince(previous)
            } else {
                repeatTiming.delay = Date.now.timeIntervalSince(held.pressed)
            }

            DiagnosticsLog.shared.record("KEY", "repeat")
            connection?.key(held.keysym, down: true)
        }

        held.repeated = .now
    }

    // Navigation keys make no text, so the text system reports no repeats for them. They repeat on a timer at the
    // timing last seen from the text system, starting from iPadOS's defaults, and only the latest one repeats.
    private func repeatNavigation(_ keysym: UInt32, code: UIKeyboardHIDUsage) {
        navigation?.timer.invalidate()

        let timer = Timer.scheduledTimer(withTimeInterval: repeatTiming.delay, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, self.navigation?.code == code else { return }
                self.connection?.key(keysym, down: true)

                let timer = Timer.scheduledTimer(withTimeInterval: self.repeatTiming.interval, repeats: true) { [weak self] _ in
                    MainActor.assumeIsolated { self?.connection?.key(keysym, down: true) }
                }
                self.navigation = (code, timer)
            }
        }
        navigation = (code, timer)
    }

    // Typing keys go to the text system before they are translated: a pending hardware keyboard language switch takes
    // effect only when the text system sees the next key, so the language read afterwards is the one that key types in.
    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let log = DiagnosticsLog.shared
        let before = textInputMode?.primaryLanguage
        var system = Set<UIPress>()
        var typed: [UIKey] = []

        for press in presses {
            guard let key = press.key else { continue }

            if Keys.isSystem(key) {
                system.insert(press)
                log.key("down", key, "lang \(before ?? "nil") system")
            } else if Keys.isTyping(key) {
                system.insert(press)
                typing.insert(key.keyCode)
                typed.append(key)
            } else {
                send(key, language: before, switched: false)
            }
        }

        if !system.isEmpty { super.pressesBegan(system, with: event) }

        let language = textInputMode?.primaryLanguage
        typed.forEach { send($0, language: language, switched: language != before) }
        log.record("KEY", "after lang \(language ?? "nil") first responder \(isFirstResponder) passed \(system.count)")
    }

    private func send(_ key: UIKey, language: String?, switched: Bool) {
        let log = DiagnosticsLog.shared

        guard let keysym = keys.press(key, language: language, switched: switched, source: connection?.source) else {
            log.key("down", key, "lang \(language ?? "nil") not sent")
            return
        }

        connection?.key(keysym, down: true)
        log.key("down", key, "lang \(language ?? "nil") sent \(log.secure ? "-" : String(keysym, radix: 16))")

        if typing.contains(key.keyCode) {
            held = (keysym, key.keyCode, .now, nil)
        } else if Keys.isNavigation(key) {
            repeatNavigation(keysym, code: key.keyCode)
        }
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        release(presses, with: event) { super.pressesEnded($0, with: $1) }
    }

    override func pressesCancelled(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        DiagnosticsLog.shared.record("KEY", "cancelled \(presses.count)")
        release(presses, with: event) { super.pressesCancelled($0, with: $1) }
    }

    private func release(_ presses: Set<UIPress>, with event: UIPressesEvent?, system pass: (Set<UIPress>, UIPressesEvent?) -> Void) {
        var system = Set<UIPress>()

        for press in presses {
            guard let key = press.key else { continue }
            DiagnosticsLog.shared.key("up", key, "lang \(textInputMode?.primaryLanguage ?? "nil")")

            if Keys.isSystem(key) {
                system.insert(press)
                continue
            }

            if held?.code == key.keyCode {
                held = nil
            }

            if navigation?.code == key.keyCode {
                navigation?.timer.invalidate()
                navigation = nil
            }

            if typing.remove(key.keyCode) != nil {
                system.insert(press)
            }

            if let keysym = keys.release(key) {
                connection?.key(keysym, down: false)
            }
        }

        if !system.isEmpty { pass(system, event) }
    }

    @objc private func languageChanged() {
        DiagnosticsLog.shared.record("LANG", "changed to \(textInputMode?.primaryLanguage ?? "nil") first responder \(isFirstResponder)")
    }

    // MARK: Pointer

    func pointerInteraction(_ interaction: UIPointerInteraction, styleFor region: UIPointerRegion) -> UIPointerStyle? {
        .hidden()
    }

    // When the pointer leaves the view, the ending hover reports (0, 0), which would throw the Mac's cursor into the corner.
    // The Mac's cursor stays where it was, and the drawn one is hidden until the pointer comes back.
    @objc private func hover(_ recognizer: UIHoverGestureRecognizer) {
        guard recognizer.state == .began || recognizer.state == .changed else {
            pointer = nil
            placeCursor()
            return
        }

        move(to: recognizer.location(in: self))
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        momentum.stop()
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

    // Gives each finger's position to the touchpad in layout pixels. Positions are not clamped to the screen because movement is relative.
    private func finger(_ touches: Set<UITouch>, _ phase: TouchPhase) {
        guard let layout else { return }
        let rect = videoRect(layout)

        for touch in touches where touch.type == .direct {
            let location = touch.location(in: self)
            let x = (location.x - rect.minX) / rect.width * layout.pixels.width
            let y = (location.y - rect.minY) / rect.height * layout.pixels.height
            let id = UInt32(truncatingIfNeeded: ObjectIdentifier(touch).hashValue)
            touchpad.touch(id: id, phase: phase, x: Float(x), y: Float(y)).forEach { gesture($0, layout) }
        }
    }

    private func gesture(_ gesture: Gesture, _ layout: SessionController.Layout) {
        let pointsPerPixel = Float(layout.logical.width / layout.pixels.width)

        switch gesture {
        case let .move(dx, dy):
            at = CGPoint(
                x: (at.x + CGFloat(dx)).clamped(to: 0...(layout.pixels.width - 1)),
                y: (at.y + CGFloat(dy)).clamped(to: 0...(layout.pixels.height - 1))
            )
            let rect = videoRect(layout)
            pointer = CGPoint(x: rect.minX + at.x / layout.pixels.width * rect.width, y: rect.minY + at.y / layout.pixels.height * rect.height)
            placeCursor()
            connection?.pointer(touchButtons, x: UInt16(at.x.rounded()), y: UInt16(at.y.rounded()))
        case let .buttons(buttons):
            touchButtons = buttons
            connection?.pointer(buttons, x: UInt16(at.x.rounded()), y: UInt16(at.y.rounded()))
        case let .scroll(phase, dx, dy):
            connection?.scroll(phase, dx: dx * pointsPerPixel, dy: dy * pointsPerPixel)

            if phase == .ended {
                let velocity = fingers.velocity(in: self)
                let scale = pointsPerViewPoint()
                momentum.start(velocity: CGPoint(x: velocity.x * scale, y: velocity.y * scale))
            }
        case let .magnify(phase, scale):
            connection?.magnify(phase, scale: scale)
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
        at = CGPoint(x: x, y: y)
        connection.pointer(buttons, x: UInt16(x), y: UInt16(y))
    }

    // Trackpad scrolling is converted to Mac points. The scroll begins with the movement the recognizer needed to begin,
    // as the Mac's own scrolls do, and momentum follows from the release velocity.
    @objc private func scroll(_ recognizer: UIPanGestureRecognizer) {
        guard let connection else { return }

        let delta = recognizer.translation(in: self)
        recognizer.setTranslation(.zero, in: self)
        let scale = pointsPerViewPoint()

        switch recognizer.state {
        case .began:
            momentum.stop()
            connection.scroll(.began, dx: Float(delta.x * scale), dy: Float(delta.y * scale))
        case .changed:
            connection.scroll(.changed, dx: Float(delta.x * scale), dy: Float(delta.y * scale))
        case .ended:
            connection.scroll(.ended, dx: 0, dy: 0)
            let velocity = recognizer.velocity(in: self)
            momentum.start(velocity: CGPoint(x: velocity.x * scale, y: velocity.y * scale))
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

        if recognizer.state == .began {
            momentum.stop()
        }

        if recognizer.state == .changed {
            connection?.wheel(
                dx: Float(delta.x * scale), dy: Float(delta.y * scale),
                linesX: Float(delta.x * scale / 10), linesY: Float(delta.y * scale / 10)
            )
        }
    }

    @objc private func pinch(_ recognizer: UIPinchGestureRecognizer) {
        let scale = Float(recognizer.scale)

        switch recognizer.state {
        case .began:
            momentum.stop()
            connection?.magnify(.began, scale: 1)
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
