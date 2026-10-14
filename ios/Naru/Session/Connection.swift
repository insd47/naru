import Foundation
import os

/// Opens one session and routes the events the core pushes to video, audio, and the UI.
///
/// - Events arrive on the session's event thread. Video and audio are fed directly on that thread, and the rest go to [announce].
/// - The core drops input sent after the session ends.
final class Connection: SessionListener, @unchecked Sendable {
    let video: Video
    private var audio: Audio?
    private var session: Session!
    private let announce: @Sendable (Event) -> Void
    private let log = Logger(subsystem: Diagnostics.subsystem, category: "Connection")
    private let sourceLock = NSLock()
    private var latestSource: InputSource?

    /// The Mac's latest input source, which decides what keys type there
    var source: InputSource? { sourceLock.withLock { latestSource } }

    // The session starts its event thread while connecting, so the video that receives events must exist first.
    @MainActor
    init(config: Config, announce: @escaping @Sendable (Event) -> Void, firstFrame: @escaping @Sendable () -> Void) {
        self.announce = announce
        video = Video(firstFrame: firstFrame)
        log.log("requested codec: \(config.codec == .hevc ? "HEVC 4:4:4" : "H.264", privacy: .public)")
        session = Session.connect(config: config, listener: self)
        video.requestKeyframe = { [weak self] in self?.session.requestKeyframe() }
    }

    func key(_ keysym: UInt32, down: Bool) { session.key(keysym: keysym, down: down) }

    func pointer(_ buttons: UInt8, x: UInt16, y: UInt16) { session.pointer(buttons: buttons, x: x, y: y) }

    func scroll(_ phase: Phase, dx: Float, dy: Float) { session.scroll(phase: phase, dx: dx, dy: dy) }

    func momentum(_ phase: MomentumPhase, dx: Float, dy: Float) { session.momentum(phase: phase, dx: dx, dy: dy) }

    func wheel(dx: Float, dy: Float, linesX: Float, linesY: Float) { session.wheel(dx: dx, dy: dy, linesX: linesX, linesY: linesY) }

    func magnify(_ phase: Phase, scale: Float) { session.magnify(phase: phase, scale: scale) }

    func selectInputSource(_ id: String) { session.selectInputSource(id: id) }

    /// `true` if sent, or `false` without sending when it exceeds the size limit.
    func clipboard(_ text: String) -> Bool { session.clipboard(text: text) }

    func resize(_ mode: DisplayMode) { session.resize(mode: mode) }

    func requestKeyframe() { session.requestKeyframe() }

    func disconnect() { session.disconnect() }

    // Video and audio arrive dozens of times a second, so they are neither passed to the UI nor logged.
    func onEvent(event: Event) {
        switch event {
        case .frame, .present, .audio, .stats:
            break
        default:
            log.log("\(Self.describe(event), privacy: .public)")
        }

        if case let .source(source) = event {
            sourceLock.withLock { latestSource = source }
        }

        switch event {
        case let .frame(unit):
            video.queue(unit)
        case .present:
            video.present()
        case let .audio(frame):
            audio?.play(frame)
        case .stats:
            break
        case let .decoder(config):
            video.configure(config)
            announce(event)
        case .resetDecoder:
            video.reset()
            announce(event)
        case let .audioConfig(config):
            // If audio cannot play, continue with video only.
            audio = Audio(config: config)
            if audio == nil {
                log.error("audio decoder could not be created")
            }
            announce(event)
        case .closed:
            audio = nil
            announce(event)
        default:
            announce(event)
        }
    }

    // Logs only the event kind and size, never clipboard contents or key bytes.
    private static func describe(_ event: Event) -> String {
        switch event {
        case let .progress(stage): "stage: \(stage)"
        case let .serverKey(der): "server key, \(der.count) bytes"
        case let .authenticated(serverProof): "authenticated (server proof \(serverProof))"
        case let .layout(logicalWidth, logicalHeight, pixelWidth, pixelHeight):
            "layout: logical \(logicalWidth)×\(logicalHeight), pixels \(pixelWidth)×\(pixelHeight)"
        case let .decoder(config): "decoder config: \(config.codec), \(config.vps.count) VPS, \(config.sps.count) SPS, \(config.pps.count) PPS"
        case let .cursor(width, height, _, _, _): "cursor \(width)×\(height)"
        case let .serverName(name): "Mac name: \(name)"
        case let .source(source): "input source: \(source.id) (secure input \(source.secure))"
        case let .clipboard(text): "clipboard, \(text.count) characters"
        case .resetDecoder: "decoder reset requested"
        case let .closed(_, detail): "session closed: \(detail ?? "normally")"
        case let .audioConfig(config): "audio config, \(config.count) bytes"
        default: "\(event)"
        }
    }
}
