import UIKit

/// Mac 한 대와의 세션을 들고 있습니다. 목록 화면과 세션 화면이 함께 봅니다.
///
/// - 목록 화면은 [state]로 접속 단계와 실패를 봅니다. 인증에 성공하면 목록 화면이 세션 화면을 엽니다.
/// - 세션 화면은 [connection], [layout], [cursor], [shown]을 봅니다. 화면을 닫아도 세션은 이어집니다.
/// - 접속 결과를 저장하는 일(Mac 목록, 비밀번호, 서버 키)과 클립보드는 화면이 없어도 일어나므로 여기서 합니다.
/// - 앱이 백그라운드로 가면 iPadOS가 허락하는 동안만 이어 가고, 멈추기 전에 끊습니다.
@MainActor @Observable
final class SessionController {
    /// 세션의 단계. 모두 어느 Mac과의 세션인지 [host]로 알려 줍니다.
    enum State: Equatable {
        case connecting(Host, Stage)
        /// 인증에 성공했습니다. 화면은 아직 준비 중일 수 있습니다.
        case connected(Host)
        case failed(Host, Failure)

        var host: Host {
            switch self {
            case let .connecting(host, _), let .connected(host), let .failed(host, _): host
            }
        }
    }

    /// 접속에 쓸 사용자 이름과 비밀번호. 인증에 성공하면 [remember]가 `true`일 때 비밀번호를 저장합니다.
    struct Credentials {
        var username: String
        var password: String
        var remember: Bool
    }

    /// Mac 화면의 논리 크기와 디코딩된 영상의 픽셀 크기
    struct Layout: Equatable {
        var logical: CGSize
        var pixels: CGSize
    }

    /// 커서 모양. 크기와 핫스팟은 Mac의 논리 좌표입니다.
    struct Cursor {
        var image: CGImage
        var size: CGSize
        var hotspot: CGPoint
    }

    /// 지금 세션의 상태. 세션이 없으면 `nil`이고, 실패는 [dismiss]할 때까지 남습니다.
    private(set) var state: State?
    private(set) var connection: Connection?
    private(set) var layout: Layout?
    private(set) var cursor: Cursor?
    /// 첫 화면을 그렸는지 여부
    private(set) var shown = false
    /// 세션 화면을 보이는지 여부. 인증에 성공하면 켜고, 세션이 끝나면 끕니다.
    var presented = false

    private let hosts: Hosts
    private let preferences: Preferences
    private var running: Running?
    private var background = UIBackgroundTaskIdentifier.invalid
    private var pasteboardChange = UIPasteboard.general.changeCount
    private var clipboard: String?

    init(hosts: Hosts, preferences: Preferences) {
        self.hosts = hosts
        self.preferences = preferences
    }

    /// 이 Mac과의 세션을 엽니다. 이미 열린 세션이 있으면 닫고 새로 엽니다.
    func open(_ host: Host, credentials: Credentials, display: DisplayMode?) {
        connection?.disconnect()
        state = .connecting(host, .warmup)
        layout = nil
        cursor = nil
        shown = false

        let config = Config(
            host: host.address, port: UInt16(host.port), username: credentials.username, password: credentials.password,
            knownKey: ServerKeys.find(host), display: display, audio: true, priority: preferences.priority,
            hevc: Video.decodesHevc444
        )

        // 새 세션으로 바뀐 뒤 늦게 도착한 이전 세션의 사건은 버립니다.
        let running = Running(host: host, credentials: credentials, display: display)
        let token = running.token
        let connection = Connection(
            config: config,
            announce: { [weak self] event in DispatchQueue.main.async { MainActor.assumeIsolated { self?.receive(event, from: token) } } },
            firstFrame: { [weak self] in DispatchQueue.main.async { MainActor.assumeIsolated { if self?.running?.token == token { self?.shown = true } } } }
        )

        self.running = running
        self.connection = connection
    }

    /// 실패를 보여 준 뒤 지웁니다.
    func dismiss() {
        if case .failed = state { state = nil }
    }

    func close() {
        connection?.disconnect()
    }

    /// 가상 디스플레이를 이 크기로 바꿉니다. 창 크기가 바뀐 뒤에 부릅니다. Mac의 실제 화면으로 접속했거나 크기가 같으면 보내지 않습니다.
    func resize(_ mode: DisplayMode) {
        guard let running, let current = running.display, mode != current else {
            return
        }

        running.display = mode
        connection?.resize(mode)
    }

    /// 다른 앱에서 복사한 텍스트를 Mac에 보냅니다. 앱이 다시 앞으로 올 때 부릅니다.
    ///
    /// - 클립보드를 읽으면 iPadOS가 붙여 넣기 허용을 물으므로, 바뀌었을 때만 읽습니다.
    func syncClipboard() {
        let pasteboard = UIPasteboard.general

        guard let connection, preferences.sharedClipboard, pasteboard.changeCount != pasteboardChange else {
            return
        }

        pasteboardChange = pasteboard.changeCount

        if let text = pasteboard.string, text != clipboard, connection.clipboard(text) {
            clipboard = text
        }
    }

    /// 지금 클립보드의 텍스트를 바뀌었는지와 관계없이 Mac에 보냅니다. 편집 메뉴의 Send Clipboard입니다.
    func sendClipboard() {
        guard let connection, let text = UIPasteboard.general.string else {
            return
        }

        pasteboardChange = UIPasteboard.general.changeCount

        if connection.clipboard(text) {
            clipboard = text
        }
    }

    /// 단축키를 차례로 눌렀다가 거꾸로 뗍니다. iPadOS가 가져가는 세 손가락 제스처 대신 보기 메뉴에서 씁니다.
    func press(_ keysyms: [UInt32]) {
        guard let connection else {
            return
        }

        keysyms.forEach { connection.key($0, down: true) }
        keysyms.reversed().forEach { connection.key($0, down: false) }
    }

    func enterBackground() {
        guard connection != nil, background == .invalid else {
            return
        }

        background = UIApplication.shared.beginBackgroundTask(withName: "session") { [weak self] in
            MainActor.assumeIsolated {
                self?.close()
                self?.endBackground()
            }
        }
    }

    func enterForeground() {
        endBackground()
        connection?.requestKeyframe()
        syncClipboard()
    }

    private func endBackground() {
        if background != .invalid {
            UIApplication.shared.endBackgroundTask(background)
            background = .invalid
        }
    }

    private func receive(_ event: Event, from token: UUID) {
        guard let running, running.token == token else {
            return
        }

        switch event {
        case let .serverKey(der):
            ServerKeys.remember(running.host, der: der)
        case .authenticated:
            remember(running)
            state = .connected(running.host)
            presented = true
        case let .serverName(name):
            rename(running, name)
        case let .clipboard(text):
            // Mac에 보낸 글이 되돌아오면 다시 쓰지 않습니다.
            guard preferences.sharedClipboard, text != clipboard else { break }
            clipboard = text
            UIPasteboard.general.string = text
            pasteboardChange = UIPasteboard.general.changeCount
        case let .layout(logicalWidth, logicalHeight, pixelWidth, pixelHeight):
            layout = Layout(
                logical: CGSize(width: Int(logicalWidth), height: Int(logicalHeight)),
                pixels: CGSize(width: Int(pixelWidth), height: Int(pixelHeight))
            )
        case let .cursor(width, height, hotspotX, hotspotY, rgba):
            cursor = Self.cursor(width: Int(width), height: Int(height), rgba: rgba)
                .map { Cursor(image: $0, size: CGSize(width: Int(width), height: Int(height)), hotspot: CGPoint(x: Int(hotspotX), y: Int(hotspotY))) }
        case let .progress(stage):
            if case .connecting = state { state = .connecting(running.host, stage) }
        case let .closed(failure, _):
            end(running, failure: failure)
        default:
            break
        }
    }

    // 인증에 성공한 Mac을 목록에 저장하고, 원하면 비밀번호도 저장합니다. 비밀번호는 이 뒤로 들고 있지 않습니다.
    private func remember(_ running: Running) {
        guard let credentials = running.credentials else {
            return
        }

        running.credentials = nil
        var host = latest(running)
        host.username = credentials.username
        host.lastConnected = .now
        running.host = host
        hosts.save(host)

        if credentials.remember {
            Keychain.save(host.id, password: credentials.password)
        }
    }

    // 이름 없이 추가한 Mac은 Mac이 알려 준 컴퓨터 이름으로 채웁니다. 사용자가 붙인 이름은 그대로 둡니다.
    private func rename(_ running: Running, _ name: String) {
        var host = latest(running)

        guard !name.isEmpty, host.name.isEmpty else {
            return
        }

        host.name = name
        running.host = host
        hosts.save(host)
        if case .connected = state { state = .connected(host) }
    }

    // 접속하는 동안 목록 화면에서 고친 내용을 덮어쓰지 않도록, 저장할 때는 저장소의 최신 값에 더합니다.
    private func latest(_ running: Running) -> Host {
        hosts.find(running.host.id) ?? running.host
    }

    // 비밀번호가 틀렸으면 저장한 비밀번호를 지웁니다. 실패는 목록 화면이 보여 줄 때까지 [state]에 남깁니다.
    private func end(_ running: Running, failure: Failure?) {
        self.running = nil
        connection = nil
        presented = false
        layout = nil
        cursor = nil
        shown = false
        state = failure.map { .failed(running.host, $0) }

        if failure == .rejected {
            Keychain.remove(running.host.id)
        }

        endBackground()
    }

    // 코어는 미리 곱하지 않은 RGBA를 줍니다.
    private static func cursor(width: Int, height: Int, rgba: Data) -> CGImage? {
        guard width > 0, height > 0, rgba.count == width * height * 4, let provider = CGDataProvider(data: rgba as CFData) else {
            return nil
        }

        return CGImage(
            width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
            space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.last.rawValue),
            provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent
        )
    }
}

// 열려 있는 세션 하나. 자격 증명은 인증할 때까지만 듭니다. 가상 디스플레이가 없으면(Mac의 실제 화면) `display`도 없습니다.
private final class Running {
    let token = UUID()
    var host: Host
    var credentials: SessionController.Credentials?
    var display: DisplayMode?

    init(host: Host, credentials: SessionController.Credentials, display: DisplayMode?) {
        self.host = host
        self.credentials = credentials
        self.display = display
    }
}
