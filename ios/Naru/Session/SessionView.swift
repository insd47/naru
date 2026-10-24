import SwiftUI

/// The session screen. Shows the Mac's screen under a title bar, with a loading state until the first frame arrives.
///
/// - The title bar follows Screen Sharing's toolbar: the way back to the connections, the Mac's name, and the Mac's
///   window shortcuts. In a window it is the navigation bar of a `NavigationStack`.
/// - Filling the screen, it comes down with the status bar when the pointer rests at the top edge, as in Screen Sharing
///   on the Mac, and the Mac's screen moves down below it. iPadOS has no toolbar that does this; macOS's
///   `windowToolbarFullScreenVisibility(.onHover)` is not available. The window's own navigation bar is removed there,
///   since one hidden in full screen kept the window draggable only within the full screen's top strip after going back
///   to a window, and the stage lays out a navigation bar of its own.
struct SessionView: View {
    @Environment(SessionController.self) private var session
    @State private var fillsScreen = false
    // Filling the screen, whether the title bar came down for the pointer at the top edge
    @State private var revealed = false
    @State private var reveal: Task<Void, Never>?

    var body: some View {
        Group {
            if fillsScreen {
                screen(revealed: revealed)
                    .ignoresSafeArea()
            } else {
                NavigationStack {
                    screen(revealed: nil)
                        .ignoresSafeArea(edges: [.horizontal, .bottom])
                        .navigationTitle(session.state?.host.title ?? "")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar { toolbar }
                }
            }
        }
        .statusBarHidden(fillsScreen && !revealed)
        // Hidden only when filling the screen, since in a window it also hides the window's controls.
        .persistentSystemOverlays(fillsScreen ? .hidden : .automatic)
        .background {
            Backdrop { fills in
                fillsScreen = fills
                if !fills { revealed = false }
            }
        }
    }

    // As in Screen Sharing on the Mac, the title bar comes down after the pointer rests at the top edge for half a second,
    // and goes once the pointer leaves it downward.
    private func pointerAtTop(_ top: Bool) {
        reveal?.cancel()

        if top {
            reveal = Task {
                try? await Task.sleep(for: .milliseconds(500))
                guard !Task.isCancelled else { return }
                revealed = true
            }
        } else if revealed {
            revealed = false
        }
    }

    // `revealed` is whether the stage's own title bar shows, or `nil` for a stage without one.
    private func screen(revealed: Bool?) -> some View {
        ZStack {
            ScreenStage(
                session: session, connection: session.connection, layout: session.layout, cursor: session.cursor,
                syncsLanguage: session.syncsKeyboardLanguage, revealed: revealed, onTop: pointerAtTop
            )

            if !session.shown {
                ProgressView()
                    .controlSize(.large)
                    .accessibilityLabel("Preparing Screen")
                    .foregroundStyle(.white)
                    .tint(.white)
            }
        }
        // Only behind the screen, so the title bar keeps the system background.
        .background(.black, ignoresSafeAreaEdges: [])
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .topBarLeading) {
            Button("Connections", systemImage: "chevron.backward") { session.presented = false }
        }

        ToolbarItemGroup(placement: .topBarTrailing) {
            Button("Mission Control", systemImage: "rectangle.3.group") { session.press(.missionControl) }
            Button("App Windows", systemImage: "macwindow.on.rectangle") { session.press(.appWindows) }
            Button("Desktop", systemImage: "menubar.dock.rectangle") { session.press(.desktop) }
            Menu("More", systemImage: "ellipsis") {
                Button("Disconnect", systemImage: "xmark", role: .destructive) { session.close() }
            }
        }
    }
}

// The Mac's screen, and filling the screen the title bar over it
private struct ScreenStage: UIViewRepresentable {
    let session: SessionController
    let connection: Connection?
    let layout: SessionController.Layout?
    let cursor: SessionController.Cursor?
    let syncsLanguage: Bool
    let revealed: Bool?
    let onTop: (Bool) -> Void

    func makeUIView(context: Context) -> StageView {
        let stage = StageView()
        if revealed != nil { stage.addBar(titleItem()) }
        return stage
    }

    func updateUIView(_ stage: StageView, context: Context) {
        let view = stage.screen
        view.connection = connection
        view.layout = layout
        view.cursor = cursor
        view.syncsLanguage = syncsLanguage
        view.onTop = onTop
        view.onResize = { [session] mode in session.resize(mode) }
        stage.bar?.topItem?.title = session.state?.host.title
        stage.reveal(revealed ?? false)
    }

    // The window's toolbar items
    private func titleItem() -> UINavigationItem {
        let item = UINavigationItem()
        item.leftBarButtonItem = UIBarButtonItem(title: "Connections", image: UIImage(systemName: "chevron.backward"), primaryAction: UIAction { [session] _ in session.presented = false })
        item.rightBarButtonItems = [
            UIBarButtonItem(title: "More", image: UIImage(systemName: "ellipsis"), menu: UIMenu(children: [
                UIAction(title: "Disconnect", image: UIImage(systemName: "xmark"), attributes: .destructive) { [session] _ in session.close() },
            ])),
            UIBarButtonItem(title: "Desktop", image: UIImage(systemName: "menubar.dock.rectangle"), primaryAction: UIAction { [session] _ in session.press(.desktop) }),
            UIBarButtonItem(title: "App Windows", image: UIImage(systemName: "macwindow.on.rectangle"), primaryAction: UIAction { [session] _ in session.press(.appWindows) }),
            UIBarButtonItem(title: "Mission Control", image: UIImage(systemName: "rectangle.3.group"), primaryAction: UIAction { [session] _ in session.press(.missionControl) }),
        ]
        return item
    }
}

/// Lays out the Mac's screen at its own full size and, filling the screen, a navigation bar at the top of the safe area,
/// attached to the top so its background goes under the status bar.
///
/// - Hidden, the bar sits above the window; shown, the screen moves down to its bottom. Both are placed in the same
///   layout pass and move with UIKit springs. The screen moves rather than shrinks, since a new size would resize the
///   Mac's display.
/// - The status bar comes a moment after it is asked for, so the bar comes down with it, when the safe area grows, rather
///   than at once and again when it grows.
/// - Touches pass through where neither is, as over a hidden bar.
final class StageView: UIView, UINavigationBarDelegate {
    let screen = ScreenView()
    private(set) var bar: UINavigationBar?
    private var revealed = false
    // Shown, whether the bar waits for the status bar
    private var waiting = false

    init() {
        super.init(frame: .zero)
        // The hidden bar sits above the stage, where it would show while the session slides in.
        clipsToBounds = true
        addSubview(screen)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func addBar(_ item: UINavigationItem) {
        let bar = UINavigationBar()
        bar.delegate = self
        bar.setItems([item], animated: false)
        addSubview(bar)
        self.bar = bar
    }

    func reveal(_ revealed: Bool) {
        guard revealed != self.revealed else { return }
        self.revealed = revealed

        guard revealed, safeAreaInsets.top == 0 else {
            waiting = false
            UIView.animate(springDuration: 0.35, bounce: 0) { self.place() }
            return
        }

        // Not for long, should the status bar never come
        waiting = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in self?.come() }
    }

    private func come() {
        guard waiting else { return }
        waiting = false
        UIView.animate(springDuration: 0.35, bounce: 0) {
            self.setNeedsLayout()
            self.layoutIfNeeded()
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()

        // Bounds and centers rather than frames, which a transform would distort
        screen.bounds = bounds
        screen.center = CGPoint(x: bounds.midX, y: bounds.midY)

        if let bar {
            let height = bar.sizeThatFits(bounds.size).height
            bar.bounds = CGRect(x: 0, y: 0, width: bounds.width, height: height)
            bar.center = CGPoint(x: bounds.midX, y: safeAreaInsets.top + height / 2)
        }

        place()
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()

        if waiting, safeAreaInsets.top > 0 {
            come()
        } else {
            UIView.animate(springDuration: 0.35, bounce: 0) {
                self.setNeedsLayout()
                self.layoutIfNeeded()
            }
        }
    }

    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        let hit = super.hitTest(point, with: event)
        return hit === self ? nil : hit
    }

    func position(for bar: any UIBarPositioning) -> UIBarPosition {
        .topAttached
    }

    private func place() {
        guard let bar else {
            screen.transform = .identity
            screen.topZone = 0
            return
        }

        let shown = revealed && !waiting
        let bottom = bar.center.y + bar.bounds.height / 2
        bar.transform = shown ? .identity : CGAffineTransform(translationX: 0, y: -bottom)
        screen.transform = CGAffineTransform(translationX: 0, y: shown ? bottom : 0)
        screen.topZone = shown ? bottom + 16 : 2
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
