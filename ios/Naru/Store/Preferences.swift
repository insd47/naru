import Foundation

/// Settings shared by all Macs.
///
/// - Quality is changed in the Settings app (Settings.bundle) and read when connecting.
/// - Clipboard sharing is toggled from the Edit menu, as in Screen Sharing on the Mac.
/// - There is no key mapping setting (Alt and Command) because iPadOS's modifier key settings handle it.
@MainActor @Observable
final class Preferences {
    /// Whether to favor latency or quality on packet loss. Latency comes first, as in Screen Sharing.
    var priority: Priority {
        UserDefaults.standard.string(forKey: "priority") == "quality" ? .quality : .latency
    }

    /// Whether to share the clipboard with the Mac
    var sharedClipboard: Bool {
        didSet { UserDefaults.standard.set(sharedClipboard, forKey: "sharedClipboard") }
    }

    init() {
        sharedClipboard = UserDefaults.standard.object(forKey: "sharedClipboard") as? Bool ?? true
    }
}
