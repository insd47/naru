import SwiftUI

/// The connection list window. Like Screen Sharing, the sidebar holds All Connections and Network, and the detail shows the Macs in the selected one.
///
/// - A new connection asks only for the address, like Screen Sharing. The user name and password are asked while connecting and saved to the list once authentication succeeds.
/// - Connecting asks for the user name and password and opens the session screen once authentication succeeds. A wrong password is reported in the login sheet.
/// - The list is disabled while connecting so a second connection to another Mac cannot start on top of it.
struct HomeView: View {
    @Environment(Hosts.self) private var hosts
    @Environment(Discovery.self) private var discovery
    @Environment(SessionController.self) private var session

    @State private var section: Section? = .saved
    @State private var search = ""
    @State private var editing: Host?
    @State private var adding = false
    @State private var address = ""
    @State private var signingIn: Host?
    @State private var loginMessage: String?

    private enum Section: Hashable {
        case saved, network
    }

    var body: some View {
        @Bindable var session = session

        NavigationSplitView {
            List(selection: $section) {
                Label("All Connections", systemImage: "rectangle.on.rectangle").tag(Section.saved)
                Label("Network", systemImage: "network").tag(Section.network)
            }
            .navigationTitle("Naru")
        } detail: {
            NavigationStack {
                switch section {
                case .network: network
                default: saved
                }
            }
        }
        .sheet(item: $editing) { host in
            EditSheet(original: host) { edited, connect in
                save(edited)
                if connect { self.connect(hosts.find(edited.id) ?? edited) }
            }
        }
        .alert("Connect To", isPresented: $adding) {
            TextField("Hostname or IP address", text: $address)
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            Button("Cancel", role: .cancel) {}
            Button("Connect") { connect(Host(address: address.trimmingCharacters(in: .whitespaces))) }
                .disabled(address.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .sheet(item: $signingIn) { host in
            LoginSheet(host: host, message: loginMessage) { credentials in
                open(host, credentials: credentials)
            } onCancel: {
                signingIn = nil
            }
            .disabled(connecting)
        }
        .alert(failureTitle, isPresented: failurePresented, presenting: failure) { failed in
            if failed.failure == .keyChanged {
                Button("Reconnect") {
                    ServerKeys.forget(failed.host)
                    connect(failed.host)
                }
            }
            Button("Close", role: .cancel) { session.dismiss() }
        } message: { failed in
            Text(message(failed.failure, host: failed.host))
        }
        .fullScreenCover(isPresented: $session.presented) {
            SessionView()
        }
        .onChange(of: session.state) { _, state in react(state) }
        .focusedSceneValue(\.home, HomeActions(add: add, connect: connect))
    }

    private var saved: some View {
        let list = hosts.list.filter { search.isEmpty || $0.title.localizedCaseInsensitiveContains(search) || $0.address.contains(search) }

        return List(list) { host in
            let active = session.state.map { $0.host.id == host.id && !isFailed($0) } ?? false

            HostRow(
                title: host.title,
                subtitle: host.name.isEmpty ? nil : host.address,
                detail: host.lastConnected.map(formatDate),
                active: active,
                connecting: active && connecting,
                onConnect: { connect(host) },
                onDisconnect: { session.close() },
                onInfo: { editing = host }
            )
            .disabled(connecting && !active)
            .contextMenu {
                Button("Connect") { connect(host) }
                Button("Edit") { editing = host }
                Divider()
                Button("Delete", role: .destructive) { delete(host) }
            }
        }
        .overlay {
            if hosts.list.isEmpty {
                ContentUnavailableView("No Connections", systemImage: "macmini", description: Text("Add a Mac to connect to its screen."))
            }
        }
        .navigationTitle("All Connections")
        .searchable(text: $search)
        .toolbar {
            Button("New Connection", systemImage: "plus", action: add)
                .disabled(connecting)
        }
    }

    private var network: some View {
        List(discovery.found) { found in
            HostRow(title: found.name, subtitle: nil, detail: nil, onConnect: { connect(found) })
                .disabled(connecting)
        }
        .overlay {
            if discovery.found.isEmpty {
                ContentUnavailableView("No Macs Found", systemImage: "network", description: Text("Macs that share their screen on this network appear here."))
            }
        }
        .navigationTitle("Network")
        .onAppear { discovery.start() }
        .onDisappear { discovery.stop() }
    }

    private var connecting: Bool {
        if case .connecting = session.state { true } else { false }
    }

    private var failure: (host: Host, failure: Failure)? {
        guard case let .failed(host, failure) = session.state, failure != .rejected else { return nil }
        return (host, failure)
    }

    private var failurePresented: Binding<Bool> {
        Binding(get: { failure != nil }, set: { if !$0 { session.dismiss() } })
    }

    private var failureTitle: String {
        failure.map { "Can't Connect to “\($0.host.title)”" } ?? ""
    }

    private func isFailed(_ state: SessionController.State) -> Bool {
        if case .failed = state { true } else { false }
    }

    private func add() {
        address = ""
        adding = true
    }

    private func connect(_ host: Host) {
        if let state = session.state, state.host.id == host.id, !isFailed(state) {
            if case .connected = state { session.presented = true }
            return
        }

        guard !host.username.isEmpty, let password = Keychain.find(host.id) else {
            loginMessage = nil
            signingIn = host
            return
        }

        open(host, credentials: .init(username: host.username, password: password, remember: false))
    }

    // Resolves a discovered Mac's address and connects. If a saved Mac has the same address, connects with that one.
    private func connect(_ found: Discovery.Found) {
        Task {
            guard let address = await discovery.resolve(found.endpoint) else { return }
            connect(hosts.list.first { $0.address == address } ?? Host(name: found.name, address: address))
        }
    }

    private func open(_ host: Host, credentials: SessionController.Credentials) {
        session.dismiss()
        session.open(host, credentials: credentials, display: host.virtualDisplay ? virtualDisplay() : nil)
    }

    // Closes the login sheet on successful authentication and reports a wrong password in it. Other failures are shown as an alert.
    private func react(_ state: SessionController.State?) {
        switch state {
        case .connected:
            signingIn = nil
        case let .failed(host, .rejected):
            loginMessage = message(.rejected, host: host)
            signingIn = signingIn ?? host
            session.dismiss()
        case .failed:
            signingIn = nil
        default:
            break
        }
    }

    // Applies only the edited fields to the latest value, so the connection time or name a session saved while the sheet was open is not overwritten.
    private func save(_ edited: Host) {
        var latest = hosts.find(edited.id) ?? edited
        latest.name = edited.name
        latest.address = edited.address
        latest.username = edited.username
        latest.port = edited.port
        latest.virtualDisplay = edited.virtualDisplay
        hosts.save(latest)
    }

    private func delete(_ host: Host) {
        hosts.remove(host)
        Keychain.remove(host.id)
    }

    private func message(_ failure: Failure, host: Host) -> String {
        switch failure {
        case .rejected: "Incorrect name or password"
        case .keyChanged: "This Mac's identity is different from the last time you connected. Reconnect only if you trust this Mac."
        case let .otherUser(name): "\(name) is logged in to this Mac."
        case .unreachable: "Can't reach \(host.address). Check the address and that Screen Sharing is turned on."
        case .unsupported: "This Mac doesn't support High Performance screen sharing."
        case .noMedia: "The Mac didn't start sending its screen."
        case .other: "The connection ended unexpectedly."
        }
    }
}

/// Actions the menu bar delegates to the connection list window
struct HomeActions {
    let add: () -> Void
    let connect: (Host) -> Void
}

extension FocusedValues {
    @Entry var home: HomeActions?
}

/// A virtual display sized to this window. The scale is 2 so the Mac renders at Retina resolution, and the pixel size matches the iPad screen.
@MainActor
func virtualDisplay() -> DisplayMode? {
    let window = UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow }.first
    let size = window?.bounds.size ?? .zero
    guard size.width > 0, size.height > 0 else { return nil }
    return DisplayMode(width: UInt32(size.width.rounded()), height: UInt32(size.height.rounded()), scale: 2)
}
