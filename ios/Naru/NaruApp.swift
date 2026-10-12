import SwiftUI

@main
struct NaruApp: App {
    @UIApplicationDelegateAdaptor private var delegate: AppDelegate
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
        .commands {
            NaruCommands(hosts: hosts, preferences: preferences, session: session, probing: $probing)
        }
    }
}

// Screen Sharing 앱처럼 Connection 메뉴를 File 메뉴 자리에 둡니다. SwiftUI는 직접 만든 메뉴를 View 뒤에 넣으므로 옮깁니다.
final class AppDelegate: UIResponder, UIApplicationDelegate {
    override func buildMenu(with builder: any UIMenuBuilder) {
        super.buildMenu(with: builder)
        builder.remove(menu: .file)

        let connection = builder.menu(for: .root)?.children
            .compactMap { $0 as? UIMenu }
            .first { $0.title == NaruCommands.connection }
        if let connection {
            builder.remove(menu: connection.identifier)
            builder.insertSibling(connection, beforeMenu: .edit)
        }
    }
}
