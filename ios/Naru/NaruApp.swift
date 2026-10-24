import SwiftUI

@main
struct NaruApp: App {
    @UIApplicationDelegateAdaptor private var delegate: AppDelegate
    @State private var hosts: Hosts
    @State private var discovery = Discovery()
    @State private var session: SessionController
    @Environment(\.scenePhase) private var phase

    init() {
        let hosts = Hosts()
        _hosts = State(initialValue: hosts)
        _session = State(initialValue: SessionController(hosts: hosts, preferences: Preferences()))
        Diagnostics.start()
    }

    var body: some Scene {
        WindowGroup {
            HomeView()
                .environment(hosts)
                .environment(discovery)
                .environment(session)
        }
        .onChange(of: phase) { _, phase in
            switch phase {
            case .background: session.enterBackground()
            case .active: session.enterForeground()
            default: break
            }
        }
        .commands {
            NaruCommands(hosts: hosts, session: session)
        }
    }
}

// Puts the Connection menu where the File menu goes, as Screen Sharing does. SwiftUI inserts custom menus after View, so this moves it.
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
