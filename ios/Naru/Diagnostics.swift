import GameController
import UIKit

/// A temporary log for diagnosing input on real devices, exported from Help > Export Diagnostics.
///
/// - It records keys, text input, language changes, focus, GameController keys, and session events, in time order.
/// - It stays in memory and becomes a file only when exported. While the Mac reports secure input, keys are recorded without their characters or codes.
final class DiagnosticsLog: @unchecked Sendable {
    static let shared = DiagnosticsLog()

    /// Whether the Mac is in secure input, such as a password field
    var secure: Bool {
        get { lock.withLock { secureInput } }
        set { lock.withLock { secureInput = newValue } }
    }

    private let lock = NSLock()
    private var entries: [String] = []
    private var last: String?
    private var secureInput = false
    private let start = Date()

    private init() {
        NotificationCenter.default.addObserver(forName: .GCKeyboardDidConnect, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.watchGameController() }
        }
        Task { @MainActor in watchGameController() }
    }

    /// Adds an entry. An entry identical to the previous one is skipped.
    func record(_ kind: String, _ text: String) {
        let stamp = String(format: "%9.3f", Date().timeIntervalSince(start))

        lock.withLock {
            let body = "\(kind) \(text)"
            guard body != last else { return }
            last = body
            entries.append("\(stamp) \(body)")
            if entries.count > 5000 { entries.removeFirst(entries.count - 5000) }
        }
    }

    /// A key, without its characters or code while the Mac is in secure input.
    func key(_ phase: String, _ key: UIKey, _ detail: String) {
        if secure {
            record("KEY", "\(phase) (secure input) \(detail)")
        } else {
            record("KEY", "\(phase) code \(key.keyCode.rawValue) chars [\(key.characters)] mods \(key.modifierFlags.rawValue) \(detail)")
        }
    }

    /// Writes the header and the log to a text file and returns its location.
    func export(header: [String]) throws -> URL {
        let lines = lock.withLock { entries }
        let text = (header + ["", "time(s) kind detail"] + lines).joined(separator: "\n")
        let stamp = ISO8601DateFormatter().string(from: .now).replacingOccurrences(of: ":", with: "-")
        let file = FileManager.default.temporaryDirectory.appending(path: "naru-diagnostics-\(stamp).txt")
        try text.write(to: file, atomically: true, encoding: .utf8)
        return file
    }

    // GameController sees physical keys separately from UIKit, including some that UIKit never delivers.
    @MainActor
    private func watchGameController() {
        GCKeyboard.coalesced?.keyboardInput?.keyChangedHandler = { [weak self] _, _, code, pressed in
            guard let self else { return }
            record("GC", secure ? "\(pressed ? "down" : "up") (secure input)" : "\(pressed ? "down" : "up") code \(code.rawValue)")
        }
    }
}
