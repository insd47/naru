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
final class ScreenView: UIView, UIKeyInput, UIPointerInteractionDelegate, UIGestureRecognizerDelegate {
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

    /// Whether the Mac's input source follows the iPad's input language ("Sync Keyboard Language")
    var syncsLanguage = false {
        didSet {
            guard syncsLanguage != oldValue else { return }
            syncChanged()
        }
    }

    private let cursorLayer = CALayer()
    private var keys = Keys()
    // The iPad's input language
    private var language: String?
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
    // Takes a double click in full screen before iPadOS, which would put the app in a window with it.
    private let doubleClick = UITapGestureRecognizer()
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

        doubleClick.numberOfTapsRequired = 2
        doubleClick.allowedTouchTypes = [UITouch.TouchType.indirectPointer.rawValue as NSNumber]
        doubleClick.cancelsTouchesInView = false
        doubleClick.delaysTouchesEnded = false
        doubleClick.delegate = self
        addGestureRecognizer(doubleClick)

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
        connection?.video.place(in: bounds, screen: layout?.pixels)
        placeCursor()

        doubleClick.isEnabled = window?.fillsScreen == true

        if bounds.size != measured, bounds.width > 0, bounds.height > 0 {
            measured = bounds.size
            resizing?.cancel()
            let mode = DisplayMode(size: bounds.size)
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
            keys.releaseAll().forEach { press($0.keysym, $0.key, down: false) }
            navigation?.timer.invalidate()
            navigation = nil
            momentum.stop()
        }
    }

    // MARK: Keyboard

    override var canBecomeFirstResponder: Bool { true }

    override func becomeFirstResponder() -> Bool {
        let became = super.becomeFirstResponder()
        Diagnostics.input.log("Screen became first responder: \(became), in the key window: \(self.window?.isKeyWindow == true), key window \(Diagnostics.describe(UIWindow.key), privacy: .public)")
        return became
    }

    override func resignFirstResponder() -> Bool {
        let resigned = super.resignFirstResponder()
        Diagnostics.input.log("Screen resigned first responder: \(resigned)")
        return resigned
    }

    // Never shows the on-screen keyboard; only hardware keyboards are accepted.
    override var inputView: UIView? { blank }
    private let blank = UIView()

    var hasText: Bool { false }

    func insertText(_ text: String) {
        Diagnostics.input.log("Text input: insert, in the key window: \(self.window?.isKeyWindow == true), key held: \(!self.typing.isEmpty)")
        reclaimPresses()
        repeatHeld()
    }

    func deleteBackward() {
        Diagnostics.input.log("Text input: delete, key held: \(!self.typing.isEmpty)")
        reclaimPresses()
        repeatHeld()
    }

    // Text with no key down means the key presses go to another responder while the text system still writes here, as
    // an iPad on iPadOS 26 showed right after the session screen opened; the same iPad no longer did on iPadOS 27.
    // Becoming first responder again, as reopening the screen did there, brings the presses back. It waits for the text
    // system to return first.
    private func reclaimPresses() {
        guard typing.isEmpty else { return }

        Diagnostics.input.log("Text input without a key press; taking the keyboard again")
        DispatchQueue.main.async { [weak self] in
            guard let self, self.window != nil else { return }
            _ = self.resignFirstResponder()
            _ = self.becomeFirstResponder()
        }
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldBeRequiredToFailBy other: UIGestureRecognizer) -> Bool {
        gestureRecognizer === doubleClick && other.view !== self
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

            press(held.keysym, held.code, down: true)
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
                self.press(keysym, code, down: true)

                let timer = Timer.scheduledTimer(withTimeInterval: self.repeatTiming.interval, repeats: true) { [weak self] _ in
                    MainActor.assumeIsolated { self?.press(keysym, code, down: true) }
                }
                self.navigation = (code, timer)
            }
        }
        navigation = (code, timer)
    }

    // Typing keys go to the text system before they are translated: a pending hardware keyboard language switch takes
    // effect only when the text system sees the next key, so the language read afterwards is the one that key types in.
    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        let before = textInputMode?.primaryLanguage
        var system = Set<UIPress>()
        var typed: [UIKey] = []

        for press in presses {
            guard let key = press.key else { continue }

            if Keys.isSystem(key) {
                system.insert(press)
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
        Diagnostics.input.log("Keys down: \(presses.count), to the text system: \(system.count), language \(language ?? "none", privacy: .public)")
    }

    private func send(_ key: UIKey, language: String?, switched: Bool) {
        guard let keysym = keys.press(key, language: language, switched: switched, source: connection?.source) else {
            return
        }

        // Only a typing key carries the input language. iPadOS switches it at the first one, so a modifier pressed before,
        // such as Shift for a capital, still reports the old language, and selecting it would switch the Mac back.
        if typing.contains(key.keyCode), let language {
            changed(to: language)
        }

        press(keysym, key.keyCode, down: true)

        if typing.contains(key.keyCode) {
            held = (keysym, key.keyCode, .now, nil)
        } else if Keys.isNavigation(key) {
            repeatNavigation(keysym, code: key.keyCode)
        }
    }

    // Like Apple's client, a key goes with its physical key only while the Mac is on the source of the iPad's language,
    // where that key types the same there, and otherwise the Mac types the keysym the iPad chose. With Sync Keyboard
    // Language on, the Mac is on it, as for Apple's client, since keys wait for the switch to it; with it off, the source
    // the Mac reported counts.
    private func press(_ keysym: UInt32, _ key: UIKeyboardHIDUsage, down: Bool) {
        guard let connection else { return }

        let source = connection.source.flatMap { source in language.flatMap { inputSource(language: $0, current: source) } }
        let onSource = source != nil && (syncsLanguage || source == connection.source?.id)
        connection.key(keysym, usage: onSource ? key.rawValue : nil, down: down)
    }

    // iPadOS reports a language switch on its own a moment after the switch key, usually before the first key in the
    // new language, which also reports it and catches a notice that came late. The core switches the Mac at once.
    private func changed(to language: String) {
        self.language = language

        if syncsLanguage {
            selectSource(for: language)
        }
    }

    // Like Apple's client, turning Sync Keyboard Language on asks for the current language's source at once, and turning
    // it off lets the Mac keep its own source, which the agent would otherwise undo at every key.
    private func syncChanged() {
        if !syncsLanguage {
            connection?.releaseInputSource()
        } else if let language = language ?? textInputMode?.primaryLanguage {
            selectSource(for: language)
        }
    }

    private func selectSource(for language: String) {
        guard let connection, let source = inputSource(language: language, current: connection.source) else {
            return
        }

        connection.selectInputSource(source)
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
                self.press(keysym, key.keyCode, down: false)
            }
        }

        if !system.isEmpty { pass(system, event) }
    }

    @objc private func languageChanged() {
        let language = textInputMode?.primaryLanguage
        Diagnostics.input.log("Input language changed to \(language ?? "none", privacy: .public)")

        if let language {
            changed(to: language)
        }
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
        // Sheets and the menu bar take the keyboard without giving it back, so a click takes it again.
        if !isFirstResponder {
            becomeFirstResponder()
        }

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

        for touch in touches where touch.type == .direct {
            let pixel = layoutPixel(touch.location(in: self), layout)
            let id = UInt32(truncatingIfNeeded: ObjectIdentifier(touch).hashValue)
            touchpad.touch(id: id, phase: phase, x: Float(pixel.x), y: Float(pixel.y)).forEach { gesture($0, layout) }
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
            sendPointer(touchButtons)
        case let .buttons(buttons):
            touchButtons = buttons
            sendPointer(buttons)
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

    // The pencil is the left button.
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

        guard let layout else {
            return
        }

        let pixel = layoutPixel(location, layout)
        at = CGPoint(x: pixel.x.clamped(to: 0...(layout.pixels.width - 1)), y: pixel.y.clamped(to: 0...(layout.pixels.height - 1)))
        sendPointer(buttons)
    }

    // The layout pixel under a point of this view, not clamped to the screen
    private func layoutPixel(_ location: CGPoint, _ layout: SessionController.Layout) -> CGPoint {
        let rect = videoRect(layout)
        return CGPoint(x: (location.x - rect.minX) / rect.width * layout.pixels.width, y: (location.y - rect.minY) / rect.height * layout.pixels.height)
    }

    private func sendPointer(_ buttons: UInt8) {
        connection?.pointer(buttons, x: UInt16(at.x.rounded()), y: UInt16(at.y.rounded()))
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
            width: CGFloat(cursor.image.width) * scale, height: CGFloat(cursor.image.height) * scale
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

extension DisplayMode {
    /// A Retina virtual display of `size` in points
    init(size: CGSize) {
        self.init(width: UInt32(size.width.rounded()), height: UInt32(size.height.rounded()), scale: 2)
    }
}
