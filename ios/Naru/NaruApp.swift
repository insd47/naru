import SwiftUI

@main
struct NaruApp: App {
    @State private var hosts: Hosts
    @State private var preferences: Preferences
    @State private var discovery = Discovery()
    @State private var session: SessionController
    @State private var probing = false
    @Environment(\.scenePhase) private var phase

    init() {
        let hosts = Hosts()
        let preferences = Preferences()
        _hosts = State(initialValue: hosts)
        _preferences = State(initialValue: preferences)
        _session = State(initialValue: SessionController(hosts: hosts, preferences: preferences))
    }

    var body: some Scene {
        WindowGroup {
            HomeView()
                .environment(hosts)
                .environment(discovery)
                .environment(session)
                .sheet(isPresented: $probing) { KeyProbe() }
        }
        .onChange(of: phase) { _, phase in
            switch phase {
            case .background: session.enterBackground()
            case .active: session.enterForeground()
            default: break
            }
        }
        // 세션 중에는 단축키가 Mac으로 가야 하므로 메뉴 항목에 단축키를 붙이지 않습니다.
        .commands {
            CommandMenu("Connection") {
                Button("Show Connections") { session.presented = false }
                    .disabled(!session.presented)
                Button("Disconnect") { session.close() }
                    .disabled(session.connection == nil)
            }
            CommandGroup(after: .pasteboard) {
                Toggle("Use Shared Clipboard", isOn: Binding(get: { preferences.sharedClipboard }, set: { preferences.sharedClipboard = $0 }))
                Button("Send Clipboard") { session.sendClipboard() }
                    .disabled(session.connection == nil)
            }
            // Mac의 기본 단축키입니다. Control+위쪽, Control+아래쪽 화살표, F11
            CommandGroup(after: .toolbar) {
                Group {
                    Button("Mission Control") { session.press([0xffe3, 0xff52]) }
                    Button("App Windows") { session.press([0xffe3, 0xff54]) }
                    Button("Desktop") { session.press([0xffc8]) }
                }
                .disabled(session.connection == nil)
            }
            CommandGroup(replacing: .help) {
                Button("Keyboard Test") { probing = true }
            }
        }
    }
}
