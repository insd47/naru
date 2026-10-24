import GameController
import SwiftUI

/// The session screen. Shows the Mac's screen under a title bar, with a loading state until the first frame arrives.
///
/// - The title bar follows Screen Sharing's toolbar: the way back to the connections, the Mac's name, and the Mac's
///   window shortcuts.
/// - Filling the screen, the title bar hides until the pointer rests at the top edge. Without a mouse or trackpad nothing
///   could bring it down, so it stays as in a window.
/// - The stage is built again when the bar starts or stops hiding, since a navigation bar hidden in full screen kept the
///   window draggable only within the full screen's top strip after going back to a window.
struct SessionView: View {
    @Environment(SessionController.self) private var session
    @State private var fillsScreen = false
    @State private var pointing = GCMouse.pointing

    var body: some View {
        let hidesBar = fillsScreen && pointing

        ScreenStage(
            session: session, connection: session.connection, layout: session.layout, cursor: session.cursor,
            syncsLanguage: session.syncsKeyboardLanguage, hidesBar: hidesBar
        )
        .id(hidesBar)
        .ignoresSafeArea()
        .overlay {
            if !session.shown {
                ProgressView()
                    .controlSize(.large)
                    .accessibilityLabel("Preparing Screen")
                    .foregroundStyle(.white)
                    .tint(.white)
            }
        }
        // Hidden only when filling the screen, since in a window it also hides the window's controls.
        .persistentSystemOverlays(fillsScreen ? .hidden : .automatic)
        .background {
            Backdrop { fillsScreen = $0 }
        }
        .onReceive(NotificationCenter.default.publisher(for: .GCMouseDidConnect)) { _ in pointing = GCMouse.pointing }
        .onReceive(NotificationCenter.default.publisher(for: .GCMouseDidDisconnect)) { note in
            pointing = GCMouse.pointing(without: note.object as? GCMouse)
        }
    }
}

// The stage, inside a navigation controller whose bar shows its title, or on its own with a bar it brings down. The
// coordinator is the stage, so updates reach it either way.
private struct ScreenStage: UIViewControllerRepresentable {
    let session: SessionController
    let connection: Connection?
    let layout: SessionController.Layout?
    let cursor: SessionController.Cursor?
    let syncsLanguage: Bool
    let hidesBar: Bool

    func makeCoordinator() -> StageController {
        StageController(session: session, hidesBar: hidesBar)
    }

    func makeUIViewController(context: Context) -> UIViewController {
        let stage = context.coordinator
        return hidesBar ? stage : UINavigationController(rootViewController: stage)
    }

    func updateUIViewController(_ controller: UIViewController, context: Context) {
        let stage = context.coordinator
        stage.navigationItem.title = session.state?.host.title

        let screen = stage.screen
        screen.connection = connection
        screen.layout = layout
        screen.cursor = cursor
        screen.syncsLanguage = syncsLanguage
        screen.onResize = { [session] mode in session.resize(mode) }
    }
}

/// Shows the Mac's screen at its own full size under the title bar.
///
/// - A bar that stays is its navigation controller's. A bar that hides, filling the screen, is one the stage lays out on
///   its own at the top of the safe area, attached to the top so its background goes under the status bar. As in Screen
///   Sharing on the Mac, it comes down with the status bar when the pointer rests at the top edge, and goes once the
///   pointer leaves it downward. iPadOS has no toolbar that does this; macOS's
///   `windowToolbarFullScreenVisibility(.onHover)` is not available.
/// - The bar, the status bar, and the screen move in one UIKit spring. The screen moves rather than shrinks, since a
///   new size would resize the Mac's display.
final class StageController: UIViewController, UINavigationBarDelegate, UIGestureRecognizerDelegate {
    let screen = ScreenView()
    private let hidesBar: Bool
    private var bar: UINavigationBar?
    private var revealed = false
    private var reveal: Task<Void, Never>?

    init(session: SessionController, hidesBar: Bool) {
        self.hidesBar = hidesBar
        super.init(nibName: nil, bundle: nil)

        navigationItem.largeTitleDisplayMode = .never
        navigationItem.leftBarButtonItem = UIBarButtonItem(title: "Connections", image: UIImage(systemName: "chevron.backward"), primaryAction: UIAction { _ in session.presented = false })
        navigationItem.rightBarButtonItems = [
            // Built at each opening, so toggles show their current values
            UIBarButtonItem(title: "More", image: UIImage(systemName: "ellipsis"), menu: UIMenu(children: [
                UIDeferredMenuElement.uncached { completion in
                    let groups: [[SessionCommand]] = [[.sharedClipboard, .sendClipboard], [.syncKeyboardLanguage], [.disconnect]]
                    completion(groups.map { group in UIMenu(options: .displayInline, children: group.map { $0.action(session) }) })
                },
            ])),
            UIBarButtonItem(title: "Desktop", image: UIImage(systemName: "menubar.dock.rectangle"), primaryAction: UIAction { _ in session.press(.desktop) }),
            UIBarButtonItem(title: "App Windows", image: UIImage(systemName: "macwindow.on.rectangle"), primaryAction: UIAction { _ in session.press(.appWindows) }),
            UIBarButtonItem(title: "Mission Control", image: UIImage(systemName: "rectangle.3.group"), primaryAction: UIAction { _ in session.press(.missionControl) }),
        ]
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var prefersStatusBarHidden: Bool { hidesBar && !revealed }
    override var preferredStatusBarUpdateAnimation: UIStatusBarAnimation { .slide }

    override func viewDidLoad() {
        super.viewDidLoad()

        // Only behind the screen, so the title bar keeps the system background.
        view.backgroundColor = .systemBackground
        screen.backgroundColor = .black
        // The hidden bar sits above the stage, where it would show while the session slides in.
        view.clipsToBounds = true
        view.addSubview(screen)

        guard hidesBar else { return }

        let bar = UINavigationBar()
        bar.delegate = self
        bar.setItems([navigationItem], animated: false)
        view.addSubview(bar)
        self.bar = bar

        let hover = UIHoverGestureRecognizer(target: self, action: #selector(hover))
        hover.delegate = self
        view.addGestureRecognizer(hover)
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()

        // Bounds and centers rather than frames, which a transform would distort. Under a bar that stays the screen starts
        // below it.
        let bounds = view.bounds
        let top = bar == nil ? view.safeAreaInsets.top : 0
        screen.bounds = CGRect(x: 0, y: 0, width: bounds.width, height: bounds.height - top)
        screen.center = CGPoint(x: bounds.midX, y: top + (bounds.height - top) / 2)

        if let bar {
            let height = bar.sizeThatFits(bounds.size).height
            bar.bounds = CGRect(x: 0, y: 0, width: bounds.width, height: height)
            bar.center = CGPoint(x: bounds.midX, y: view.safeAreaInsets.top + height / 2)
            place()
        }
    }

    func position(for bar: any UIBarPositioning) -> UIBarPosition {
        .topAttached
    }

    // Alongside the screen's own hover, which moves the Mac's cursor
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        other is UIHoverGestureRecognizer
    }

    // As in Screen Sharing on the Mac, the bar comes down after the pointer rests at the top edge for half a second,
    // and goes once the pointer leaves it downward. The edge reaches a little past the bar while it shows.
    @objc private func hover(_ recognizer: UIHoverGestureRecognizer) {
        guard recognizer.state == .began || recognizer.state == .changed, let bar else {
            return
        }

        let edge = revealed ? bar.frame.maxY + 16 : 2

        if recognizer.location(in: view).y > edge {
            reveal?.cancel()
            reveal = nil
            if revealed { show(false) }
        } else if !revealed, reveal == nil {
            reveal = Task {
                try? await Task.sleep(for: .milliseconds(500))
                guard !Task.isCancelled else { return }
                reveal = nil
                show(true)
            }
        }
    }

    // The status bar comes in the same animation, and the safe area it brings places the bar.
    private func show(_ revealed: Bool) {
        self.revealed = revealed

        UIView.animate(springDuration: 0.35, bounce: 0) {
            self.setNeedsStatusBarAppearanceUpdate()
            self.view.setNeedsLayout()
            self.view.layoutIfNeeded()
        }
    }

    private func place() {
        guard let bar else { return }

        let bottom = bar.center.y + bar.bounds.height / 2
        bar.transform = revealed ? .identity : CGAffineTransform(translationX: 0, y: -bottom)
        screen.transform = CGAffineTransform(translationX: 0, y: revealed ? bottom : 0)
    }
}

// The window behind the session, which reports whether it fills its screen
private struct Backdrop: UIViewRepresentable {
    let onChange: (Bool) -> Void

    func makeUIView(context: Context) -> BackdropView {
        BackdropView()
    }

    func updateUIView(_ view: BackdropView, context: Context) {
        view.onChange = onChange
    }

    final class BackdropView: UIView {
        var onChange: ((Bool) -> Void)?
        private var fills: Bool?

        override func layoutSubviews() {
            super.layoutSubviews()

            guard let now = window?.fillsScreen else {
                return
            }

            if fills != now {
                fills = now
                onChange?(now)
            }
        }
    }
}

extension GCMouse {
    /// Whether a mouse or trackpad is connected, leaving out one going, which may still be listed, and the pointer a Mac
    /// connected for development adds ("CoreDevice touchscreenGesture"), which no user has.
    static func pointing(without going: GCMouse? = nil) -> Bool {
        mice().contains { $0 !== going && $0.vendorName != "CoreDevice touchscreenGesture" }
    }

    static var pointing: Bool { pointing() }
}

extension UIWindow {
    /// The key window of the app's foreground scene
    static var key: UIWindow? {
        UIApplication.shared.connectedScenes.compactMap { ($0 as? UIWindowScene)?.keyWindow }.first
    }

    /// Whether the window fills its screen. iPadOS has no property for it, so the sizes are compared, either way round
    /// for rotation.
    var fillsScreen: Bool {
        guard let screen = windowScene?.screen.bounds.size else {
            return false
        }

        return bounds.size == screen || bounds.size == CGSize(width: screen.height, height: screen.width)
    }
}
