import Network
import Observation

/// Finds Macs with Screen Sharing on the local network over Bonjour (`_rfb._tcp`).
///
/// - Browses only while the screen is visible, between [start] and [stop].
@MainActor @Observable
final class Discovery {
    /// A discovered Mac's name and service endpoint
    struct Found: Identifiable {
        let name: String
        let endpoint: NWEndpoint
        var id: String { name }
    }

    private(set) var found: [Found] = []
    private var browser: NWBrowser?

    func start() {
        guard browser == nil else {
            return
        }

        let browser = NWBrowser(for: .bonjour(type: "_rfb._tcp", domain: nil), using: .tcp)
        browser.browseResultsChangedHandler = { [weak self] results, _ in
            let found = results.compactMap { result -> Found? in
                guard case let .service(name, _, _, _) = result.endpoint else { return nil }
                return Found(name: name, endpoint: result.endpoint)
            }

            MainActor.assumeIsolated { self?.found = found.sorted { $0.name < $1.name } }
        }

        browser.start(queue: .main)
        self.browser = browser
    }

    func stop() {
        browser?.cancel()
        browser = nil
        found = []
    }

    /// Resolves a discovered Mac's IPv4 address by briefly connecting to the service, reading the remote address, and disconnecting. Returns `nil` on failure.
    func resolve(_ endpoint: NWEndpoint) async -> String? {
        // The core connects only over IPv4 because UDP media never arrives over IPv6, so this connects over IPv4 to avoid picking a link-local IPv6 address.
        let parameters = NWParameters.tcp
        (parameters.defaultProtocolStack.internetProtocol as? NWProtocolIP.Options)?.version = .v4
        let connection = NWConnection(to: endpoint, using: parameters)
        defer { connection.cancel() }

        return await withCheckedContinuation { continuation in
            let resumed = Resumed()

            connection.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    var address: String?

                    // The string form carries an interface suffix like `%en0`, so this formats the four bytes instead.
                    if case let .hostPort(.ipv4(ipv4), _) = connection.currentPath?.remoteEndpoint {
                        address = ipv4.rawValue.map(String.init).joined(separator: ".")
                    }

                    resumed.once { continuation.resume(returning: address) }
                case .failed, .cancelled:
                    resumed.once { continuation.resume(returning: nil) }
                default:
                    break
                }
            }

            connection.start(queue: .main)
        }
    }
}

// Lets the continuation resume only once even if the state callback fires several times.
private final class Resumed: @unchecked Sendable {
    private var done = false

    func once(_ action: () -> Void) {
        guard !done else { return }
        done = true
        action()
    }
}
