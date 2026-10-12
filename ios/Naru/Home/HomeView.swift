import SwiftUI

/// 연결 목록 창. Screen Sharing 앱처럼 사이드바에 모든 연결과 네트워크를 두고, 고른 쪽의 Mac을 보입니다.
///
/// - 새 연결은 Screen Sharing 앱처럼 주소만 묻습니다. 이름과 비밀번호는 접속하면서 묻고, 인증에 성공하면 목록에 저장합니다.
/// - 접속하면서 이름과 비밀번호를 묻고, 인증에 성공하면 세션 화면을 엽니다. 비밀번호가 틀리면 로그인 시트에 알립니다.
/// - 접속하는 동안에는 목록을 막아 다른 Mac으로 접속을 겹쳐 시작하지 않게 합니다.
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
            Button("New Connection", systemImage: "plus") {
                address = ""
                adding = true
            }
            .disabled(connecting)
        }
    }

    private var network: some View {
        List(discovery.found, id: \.name) { found in
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

    // 찾은 Mac의 주소를 풀어 접속합니다. 같은 주소로 저장한 Mac이 있으면 그것으로 접속합니다.
    private func connect(_ found: (name: String, endpoint: NWEndpointValue)) {
        Task {
            guard let address = await discovery.resolve(found.endpoint) else { return }
            connect(hosts.list.first { $0.address == address } ?? Host(name: found.name, address: address))
        }
    }

    private func open(_ host: Host, credentials: SessionController.Credentials) {
        session.dismiss()
        session.open(host, credentials: credentials, display: host.virtualDisplay ? virtualDisplay() : nil)
    }

    // 인증에 성공하면 로그인 시트를 닫고, 비밀번호가 틀리면 로그인 시트에 알립니다. 그 밖의 실패는 알림으로 보입니다.
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

    // 시트를 여는 사이 세션이 저장한 접속 시각이나 이름을 덮어쓰지 않도록, 고친 항목만 최신 값에 더합니다.
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

typealias NWEndpointValue = Discovery.Endpoint

/// 이 창 크기에 맞춘 가상 디스플레이. 배율 2로 두어 Mac이 Retina로 그리고, 픽셀 크기는 iPad 화면과 같습니다.
@MainActor
func virtualDisplay() -> DisplayMode? {
    let window = UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow }.first
    let size = window?.bounds.size ?? .zero
    guard size.width > 0, size.height > 0 else { return nil }
    return DisplayMode(width: UInt32(size.width.rounded()), height: UInt32(size.height.rounded()), scale: 2)
}
