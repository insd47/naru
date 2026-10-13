import OSLog
import UIKit

/// The app's log, written with the system logger and exported from Help > Export Diagnostics for bug reports.
///
/// - Export reads back this launch's entries of the app's subsystem, so it needs no log of its own.
/// - Entries use the default level, which the system keeps; values are public because nothing logged is sensitive.
///   Never log passwords, keys, typed text or key codes, or clipboard contents.
enum Diagnostics {
    static let subsystem = "dev.insd.naru"
    static let input = Logger(subsystem: subsystem, category: "Input")

    /// Records key window changes, since keys reach only the key window.
    @MainActor
    static func start() {
        for (name, label) in [(UIWindow.didBecomeKeyNotification, "became"), (UIWindow.didResignKeyNotification, "is no longer")] {
            NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { note in
                // Delivered on the main queue, where the window belongs.
                nonisolated(unsafe) let window = note.object as? UIWindow
                MainActor.assumeIsolated { input.log("\(describe(window), privacy: .public) \(label, privacy: .public) the key window") }
            }
        }
    }

    /// The window's type and its root view controller's type, to tell the app's windows apart
    @MainActor
    static func describe(_ window: UIWindow?) -> String {
        guard let window else { return "no window" }
        let root = window.rootViewController.map { String(describing: type(of: $0)) } ?? "no root"
        return "\(type(of: window)) (\(root), level \(window.windowLevel.rawValue))"
    }

    /// The key window of the app's foreground scene
    @MainActor
    static var keyWindow: UIWindow? {
        UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow }.first
    }

    /// Writes the header and this launch's log to a text file and returns its location.
    static func export(header: [String]) throws -> URL {
        let store = try OSLogStore(scope: .currentProcessIdentifier)
        let formatter = DateFormatter()
        formatter.dateFormat = "HH:mm:ss.SSS"

        let lines = try store.getEntries(matching: NSPredicate(format: "subsystem == %@", subsystem))
            .compactMap { $0 as? OSLogEntryLog }
            .map { "\(formatter.string(from: $0.date)) \($0.category) \($0.composedMessage)" }

        let text = (header + [""] + lines).joined(separator: "\n")
        let stamp = ISO8601DateFormatter().string(from: .now).replacingOccurrences(of: ":", with: "-")
        let file = FileManager.default.temporaryDirectory.appending(path: "naru-diagnostics-\(stamp).txt")
        try text.write(to: file, atomically: true, encoding: .utf8)
        return file
    }
}
