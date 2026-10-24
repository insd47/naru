import SwiftUI

/// The menu bar, following Screen Sharing's menus and wording but keeping only what Naru does.
///
/// - Shortcuts are attached only while the connection list is showing, because during a session they must go to the Mac.
struct NaruCommands: Commands {
    static let connection = "Connection"

    let hosts: Hosts
    let session: SessionController
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

            SessionCommandItem(.disconnect, session: session)
        }

        CommandGroup(after: .pasteboard) {
            Divider()
            SessionCommandItem(.sharedClipboard, session: session)
            SessionCommandItem(.sendClipboard, session: session)
            Divider()
            SessionCommandItem(.syncKeyboardLanguage, session: session)
        }

        CommandGroup(replacing: .toolbar) {
            Group {
                Button("Mission Control") { session.press(.missionControl) }
                Button("App Windows") { session.press(.appWindows) }
                Button("Desktop") { session.press(.desktop) }
            }
            .disabled(session.connection == nil)
        }

        CommandGroup(before: .windowArrangement) {
            Button("Connections") { session.presented = false }
                .keyboardShortcut(shortcut("0"))
            Divider()
        }

        CommandGroup(replacing: .help) {
            Button("Export Diagnostics…") { session.exportDiagnostics() }
            Button("Privacy Policy") { UIApplication.shared.open(URL(string: "https://github.com/insd47/naru/blob/main/PRIVACY.md")!) }
        }
    }

    private var recent: [Host] {
        let connected = hosts.list.compactMap { host in host.lastConnected.map { (host, $0) } }
        return connected.sorted { $0.1 > $1.1 }.prefix(10).map(\.0)
    }

    private func shortcut(_ key: KeyEquivalent) -> KeyboardShortcut? {
        session.presented ? nil : KeyboardShortcut(key)
    }
}
