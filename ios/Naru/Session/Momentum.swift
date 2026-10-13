import UIKit

/// Continues a scroll after the fingers lift, decelerating as a scroll view does, and sends it as momentum.
///
/// - iPadOS reports nothing after a trackpad or touch scroll ends; only scroll views decelerate. This follows their
///   curve from the release velocity, once per display frame.
/// - UIScrollView's start and stop speeds are not public, so the thresholds here are chosen.
final class Momentum: NSObject {
    private static let minimumSpeed: CGFloat = 100
    private static let stopSpeed: CGFloat = 10

    private let send: (MomentumPhase, CGPoint) -> Void
    private var link: CADisplayLink?
    // Release velocity in points per second, when it started, and how far it has moved
    private var velocity = CGPoint.zero
    private var started: CFTimeInterval = 0
    private var travelled = CGPoint.zero
    private var began = false

    init(send: @escaping (MomentumPhase, CGPoint) -> Void) {
        self.send = send
    }

    /// Starts from a release velocity in points per second. A slow release has no momentum.
    func start(velocity: CGPoint) {
        stop()

        guard hypot(velocity.x, velocity.y) >= Self.minimumSpeed else {
            return
        }

        self.velocity = velocity
        started = CACurrentMediaTime()
        travelled = .zero

        let link = CADisplayLink(target: self, selector: #selector(frame))
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    /// Ends momentum in progress. The Mac hears the end only if momentum began.
    func stop() {
        guard let link else {
            return
        }

        link.invalidate()
        self.link = nil

        if began {
            send(.ended, .zero)
        }

        began = false
    }

    // Speed decays by the deceleration rate every millisecond, so the distance after t ms is v₀(rᵗ − 1) / ln r.
    @objc private func frame(_ link: CADisplayLink) {
        let rate = UIScrollView.DecelerationRate.normal.rawValue
        let elapsed = (link.targetTimestamp - started) * 1000
        let decay = pow(rate, elapsed)

        guard hypot(velocity.x, velocity.y) * decay >= Self.stopSpeed else {
            stop()
            return
        }

        let distance = (decay - 1) / log(rate) / 1000
        let position = CGPoint(x: velocity.x * distance, y: velocity.y * distance)
        let delta = CGPoint(x: position.x - travelled.x, y: position.y - travelled.y)
        travelled = position

        send(began ? .changed : .began, delta)
        began = true
    }
}
