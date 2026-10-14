import Foundation

/// Settings shared by all Macs.
///
/// - Clipboard sharing and keyboard language sync are toggled from the Edit menu, as in Screen Sharing on the Mac.
/// - There is no key mapping setting (Alt and Command) because iPadOS's modifier key settings handle it.
@MainActor @Observable
final class Preferences {
    /// Whether to share the clipboard with the Mac
    var sharedClipboard: Bool {
        didSet { UserDefaults.standard.set(sharedClipboard, forKey: "sharedClipboard") }
    }

    /// Whether the Mac's input source follows the iPad's input language. Off by default, as in Screen Sharing.
    var syncKeyboardLanguage: Bool {
        didSet { UserDefaults.standard.set(syncKeyboardLanguage, forKey: "syncKeyboardLanguage") }
    }

    init() {
        sharedClipboard = UserDefaults.standard.object(forKey: "sharedClipboard") as? Bool ?? true
        syncKeyboardLanguage = UserDefaults.standard.bool(forKey: "syncKeyboardLanguage")
    }
}
