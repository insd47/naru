import Network
import Observation

/// 같은 네트워크의 화면 공유 Mac을 Bonjour(`_rfb._tcp`)로 찾습니다.
///
/// - 화면이 보이는 동안만 [start]와 [stop]으로 찾습니다.
@MainActor @Observable
final class Discovery {
    typealias Endpoint = NWEndpoint

    /// 찾은 Mac의 이름과 서비스 주소
    private(set) var found: [(name: String, endpoint: NWEndpoint)] = []
    private var browser: NWBrowser?

    func start() {
        guard browser == nil else {
            return
        }

        let browser = NWBrowser(for: .bonjour(type: "_rfb._tcp", domain: nil), using: .tcp)
        browser.browseResultsChangedHandler = { [weak self] results, _ in
            let found = results.compactMap { result -> (name: String, endpoint: NWEndpoint)? in
                guard case let .service(name, _, _, _) = result.endpoint else { return nil }
                return (name, result.endpoint)
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

    /// 찾은 Mac의 IPv4 주소를 풉니다. 서비스에 잠깐 연결해 상대 주소를 읽고 바로 끊습니다. 실패하면 `nil`입니다.
    func resolve(_ endpoint: NWEndpoint) async -> String? {
        // 코어는 UDP 미디어 때문에 IPv4로만 접속하므로, 링크 로컬 IPv6 주소가 고르지 않도록 IPv4로 연결합니다.
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

                    // 문자열 표현에는 `%en0` 같은 인터페이스가 붙으므로 네 바이트로 씁니다.
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

// 연결 상태 콜백이 여러 번 와도 한 번만 이어 가게 합니다.
private final class Resumed: @unchecked Sendable {
    private var done = false

    func once(_ action: () -> Void) {
        guard !done else { return }
        done = true
        action()
    }
}
