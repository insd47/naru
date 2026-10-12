import SwiftUI

/// 메뉴 막대. Screen Sharing 앱의 메뉴 구성과 문구를 따르고, Naru가 하는 일만 둡니다.
///
/// - 세션 중에는 단축키가 Mac으로 가야 하므로, 단축키는 연결 목록을 보는 동안에만 붙입니다.
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

        // Mac의 기본 단축키입니다. Control+위쪽, Control+아래쪽 화살표, F11
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
