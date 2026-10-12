import AVFoundation
import UIKit

/// Mac 화면을 그리고 키보드와 포인터 입력을 Mac에 보냅니다.
///
/// - 영상은 [Video]의 레이어를 붙여 그리고, 커서는 시스템 포인터를 숨긴 뒤 Mac이 보낸 비트맵을 포인터 자리에 그립니다.
/// - 포인터(트랙패드, 마우스, Apple Pencil)만 Mac에 보냅니다. 손가락 터치는 Mac에 대응하는 입력이 없으므로 [onTap]으로 넘깁니다.
/// - 텍스트 입력을 받는 뷰로 두어 iPadOS의 입력 언어 전환(Caps Lock, 지구본 키)이 동작하게 하고, 글자 키는 입력기로 넘기지 않고 직접 보냅니다.
/// - 크기가 바뀌면 잠시 기다렸다가 [onResize]로 가상 디스플레이 크기를 알립니다.
final class ScreenView: UIView, UIKeyInput, UIPointerInteractionDelegate {
    var onTap: () -> Void = {}
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
    private var buttons: UInt8 = 0
    private var pointer: CGPoint?
    private var scrolled = CGPoint.zero
    private var resizing: DispatchWorkItem?
    private var measured = CGSize.zero

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .black
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

        let tap = UITapGestureRecognizer(target: self, action: #selector(tap))
        tap.allowedTouchTypes = [UITouch.TouchType.direct.rawValue as NSNumber]
        addGestureRecognizer(tap)
    }

    required init?(coder: NSCoder) {
        fatalError("코드로만 만듭니다")
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

    // MARK: 키보드

    override var canBecomeFirstResponder: Bool { true }

    // 화면 키보드는 띄우지 않습니다. 하드웨어 키보드만 받습니다.
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

    // MARK: 포인터

    func pointerInteraction(_ interaction: UIPointerInteraction, styleFor region: UIPointerRegion) -> UIPointerStyle? {
        .hidden()
    }

    @objc private func hover(_ recognizer: UIHoverGestureRecognizer) {
        move(to: recognizer.location(in: self))
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        point(touches, event: event)
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        point(touches, event: event)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        point(touches, event: event, released: true)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        point(touches, event: event, released: true)
    }

    // 서버의 버튼 비트는 표준 RFB와 달리 오른쪽이 2, 가운데가 4입니다. 펜은 왼쪽 버튼입니다.
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

    // 트랙패드 스크롤은 Mac의 포인트로 옮깁니다. 관성과 포인터 위치는 세션이 처리합니다.
    @objc private func scroll(_ recognizer: UIPanGestureRecognizer) {
        guard let connection else { return }

        let total = recognizer.translation(in: self)
        let delta = CGPoint(x: total.x - scrolled.x, y: total.y - scrolled.y)
        scrolled = total
        let scale = pointsPerViewPoint()

        switch recognizer.state {
        case .began:
            scrolled = .zero
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

    // 마우스 휠은 한 칸을 10포인트로 보고 줄 단위로 보냅니다.
    @objc private func wheel(_ recognizer: UIPanGestureRecognizer) {
        let delta = recognizer.translation(in: self)
        recognizer.setTranslation(.zero, in: self)

        if recognizer.state == .changed {
            connection?.wheel(dx: Float(delta.x / 10), dy: Float(delta.y / 10))
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

    @objc private func tap() {
        onTap()
    }

    // MARK: 커서

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
