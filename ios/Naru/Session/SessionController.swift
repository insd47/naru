import GameController
import UIKit

/// Holds the session with one Mac, observed by both the list screen and the session screen.
///
/// - The list screen watches [state] for the connection stage and failures. Once authentication succeeds, it opens the session screen.
/// - The session screen watches [connection], [layout], [cursor], and [shown]. Closing the screen does not end the session.
/// - Saving connection results (the Mac list, password, server key) and clipboard sync happen here because they must work without a screen.
/// - In the background the session continues only as long as iPadOS allows, and disconnects before the app is suspended.
@MainActor @Observable
final class SessionController {
    /// The session stage. Every case carries the Mac it belongs to in [host].
    enum State: Equatable {
        case connecting(Host)
        /// Authentication succeeded. The screen may still be loading.
        case connected(Host)
        case failed(Host, Failure)

        var host: Host {
            switch self {
            case let .connecting(host), let .connected(host), let .failed(host, _): host
            }
        }
    }

    /// The user name and password to connect with. On successful authentication, the password is saved if [remember] is `true`.
    struct Credentials {
        var username: String
        var password: String
        var remember: Bool
    }

    /// The logical size of the Mac's screen and the pixel size of the decoded video
    struct Layout: Equatable {
        var logical: CGSize
        var pixels: CGSize
    }

    /// A Mac shortcut iPadOS takes for its own three-finger gestures, sent from the View menu and the toolbar
    enum Shortcut {
        case missionControl, appWindows, desktop

        // The Mac's defaults: Control-Up Arrow, Control-Down Arrow, F11
        var keysyms: [UInt32] {
            switch self {
            case .missionControl: [0xffe3, 0xff52]
            case .appWindows: [0xffe3, 0xff54]
            case .desktop: [0xffc8]
            }
        }
    }

    /// The cursor image. Its size and hotspot are in the Mac's logical coordinates.
    struct Cursor {
        var image: CGImage
        var hotspot: CGPoint
    }

    /// The current session state, or `nil` with no session. A failure stays until [dismiss].
    private(set) var state: State?
    private(set) var connection: Connection?
    private(set) var layout: Layout?
    /// The Mac's cursor, or `nil` while the Mac hides it
    private(set) var cursor: Cursor?
    /// Whether the first frame has been drawn
    private(set) var shown = false
    /// Whether the Mac's input source follows the iPad's input language
    var syncsKeyboardLanguage: Bool { preferences.syncKeyboardLanguage }

    /// Whether the session screen is shown. Turned on when authentication succeeds and off when the session ends.
    var presented = false

    private let hosts: Hosts
    let preferences: Preferences
    private var running: Running?
    private var background = UIBackgroundTaskIdentifier.invalid
    private var clipboard: Clipboard?

    init(hosts: Hosts, preferences: Preferences) {
        self.hosts = hosts
        self.preferences = preferences
    }

    /// Opens a session with this Mac, closing any session already open.
    func open(_ host: Host, credentials: Credentials, display: DisplayMode?) {
        connection?.disconnect()
        state = .connecting(host)
        clearScreen()

        let config = Config(
            host: host.address, port: UInt16(host.port), username: credentials.username, password: credentials.password,
            knownKey: ServerKeys.find(host), display: display, audio: true,
            codec: Video.decodesHevc444 ? .hevc : .h264
        )

        // Drops events from a previous session that arrive after a new one has started.
        let running = Running(host: host, credentials: credentials, display: display)
        let token = running.token
        let connection = Connection(
            config: config,
            announce: { [weak self] event in DispatchQueue.main.async { MainActor.assumeIsolated { self?.receive(event, from: token) } } },
            firstFrame: { [weak self] in DispatchQueue.main.async { MainActor.assumeIsolated { if self?.running?.token == token { self?.shown = true } } } }
        )

        self.running = running
        self.connection = connection
        clipboard = Clipboard(connection: connection, preferences: preferences)
    }

    /// Clears a failure after it has been shown.
    func dismiss() {
        if case .failed = state { state = nil }
    }

    func close() {
        connection?.disconnect()
    }

    /// Resizes the virtual display. Called after the window size changes. Sends nothing when connected to the Mac's physical display or when the size is unchanged.
    func resize(_ mode: DisplayMode) {
        guard let running, let current = running.display, mode != current else {
            return
        }

        running.display = mode
        connection?.resize(mode)
    }

    /// Offers content copied in another app to the Mac. Called when the app returns to the foreground.
    func syncClipboard() {
        clipboard?.sync()
    }

    /// Offers the current clipboard to the Mac whether or not it changed. Backs Send Clipboard in the Edit menu.
    func sendClipboard() {
        clipboard?.send()
    }

    /// Presses a shortcut's keys in order and releases them in reverse.
    func press(_ shortcut: Shortcut) {
        guard let connection else {
            return
        }

        shortcut.keysyms.forEach { connection.key($0, usage: nil, down: true) }
        shortcut.keysyms.reversed().forEach { connection.key($0, usage: nil, down: false) }
    }

    /// Writes the diagnostics log to a file and offers it in the share sheet.
    func exportDiagnostics() {
        let info = Bundle.main.infoDictionary
        var system = utsname()
        uname(&system)
        let model = withUnsafeBytes(of: system.machine) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }

        let header = [
            "Naru \(info?["CFBundleShortVersionString"] ?? "?") (\(info?["CFBundleVersion"] ?? "?")), iPadOS \(UIDevice.current.systemVersion), \(model)",
            "HEVC 4:4:4 hardware decoding: \(Video.decodesHevc444 ? "supported" : "unsupported")",
            "Input modes: \(UITextInputMode.activeInputModes.compactMap(\.primaryLanguage).joined(separator: ", ")), system language \(Locale.preferredLanguages.first ?? "none")",
            "Pointers: \(GCMouse.mice().map { "\($0.vendorName ?? "?") (\($0.productCategory))" }.joined(separator: ", "))",
        ]

        guard let file = try? Diagnostics.export(header: header), var top = UIWindow.key?.rootViewController else {
            return
        }

        while let presented = top.presentedViewController { top = presented }

        let share = UIActivityViewController(activityItems: [file], applicationActivities: nil)
        share.popoverPresentationController?.sourceView = top.view
        share.popoverPresentationController?.sourceRect = CGRect(x: top.view.bounds.midX, y: top.view.bounds.minY, width: 0, height: 0)
        top.present(share, animated: true)
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
        case let .clipboardOffer(kinds):
            clipboard?.offered(kinds)
        case let .clipboard(items):
            clipboard?.received(items)
        case .clipboardRequested:
            clipboard?.requested()
        case let .layout(logicalWidth, logicalHeight, pixelWidth, pixelHeight):
            layout = Layout(
                logical: CGSize(width: Int(logicalWidth), height: Int(logicalHeight)),
                pixels: CGSize(width: Int(pixelWidth), height: Int(pixelHeight))
            )
        case let .cursor(width, height, hotspotX, hotspotY, rgba):
            cursor = Self.cursor(width: Int(width), height: Int(height), rgba: rgba)
                .map { Cursor(image: $0, hotspot: CGPoint(x: Int(hotspotX), y: Int(hotspotY))) }
        case .cursorHidden:
            cursor = nil
        case let .closed(failure, _):
            clipboard = nil
            end(running, failure: failure)
        default:
            break
        }
    }

    // Saves the authenticated Mac to the list, and the password if requested. The password is not held after this.
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

    // A Mac added without a name gets the computer name it reports. A user-given name is kept.
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

    // Saves onto the store's latest value so edits made in the list screen while connecting are not overwritten.
    private func latest(_ running: Running) -> Host {
        hosts.find(running.host.id) ?? running.host
    }

    // Deletes the saved password if it was wrong. The failure stays in [state] until the list screen shows it.
    private func end(_ running: Running, failure: Failure?) {
        self.running = nil
        connection = nil
        presented = false
        clearScreen()
        state = failure.map { .failed(running.host, $0) }

        if failure == .rejected {
            Keychain.remove(running.host.id)
        }

        endBackground()
    }

    private func clearScreen() {
        layout = nil
        cursor = nil
        shown = false
    }

    // The core provides non-premultiplied RGBA.
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

// One open session. Credentials are held only until authentication. `display` is `nil` without a virtual display (the Mac's physical display).
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
