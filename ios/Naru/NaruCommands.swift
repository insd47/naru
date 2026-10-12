import SwiftUI

/// The menu bar, following Screen Sharing's menus and wording but keeping only what Naru does.
///
/// - Shortcuts are attached only while the connection list is showing, because during a session they must go to the Mac.
struct NaruCommands: Commands {
    static let connection = "Connection"

    let hosts: Hosts
    let preferences: Preferences
    let session: SessionController
    @Binding var diagnosing: Bool
    @FocusedValue(\.home) private var home

    var body: some Commands {
        CommandMenu(Self.connection) {
            Button("New…") { home?.add() }
                .keyboardShortcut(shortcut("n"))
                .disabled(home == nil || session.presented)
            Menu("Open Recent") {
                ForEach(recent) { host in
                    Button(host.title) { home?.connect(host) }
                }
            }
            .disabled(home == nil || session.presented || recent.isEmpty)

            Divider()

            Button("Close") { session.close() }
                .disabled(session.connection == nil)
        }

        CommandGroup(after: .pasteboard) {
            Divider()
            Toggle("Use Shared Clipboard", isOn: Binding(get: { preferences.sharedClipboard }, set: { preferences.sharedClipboard = $0 }))
            Button("Send Clipboard") { session.sendClipboard() }
                .disabled(session.connection == nil)
        }

        // The Mac's default shortcuts: Control-Up Arrow, Control-Down Arrow, F11
        CommandGroup(replacing: .toolbar) {
            Group {
                Button("Mission Control") { session.press([0xffe3, 0xff52]) }
                Button("App Windows") { session.press([0xffe3, 0xff54]) }
                Button("Desktop") { session.press([0xffc8]) }
            }
            .disabled(session.connection == nil)
        }

        CommandGroup(before: .windowArrangement) {
            Button("Connections") { session.presented = false }
                .keyboardShortcut(shortcut("0"))
            Divider()
        }

        CommandGroup(replacing: .help) {
            Button("Diagnostics") { diagnosing = true }
        }
    }

    private var recent: [Host] {
        hosts.list
            .filter { $0.lastConnected != nil }
            .sorted { $0.lastConnected! > $1.lastConnected! }
            .prefix(10)
            .map(\.self)
    }

    private func shortcut(_ key: KeyEquivalent) -> KeyboardShortcut? {
        session.presented ? nil : KeyboardShortcut(key)
    }
}
