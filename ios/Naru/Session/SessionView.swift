import SwiftUI

/// The session screen. Shows the Mac's screen under a title bar, with a loading state until the first frame arrives.
///
/// - The title bar follows Screen Sharing's toolbar: the way back to the connections, the Mac's name, and the Mac's
///   window shortcuts. It shows while the app is in a window. Filling the screen, it goes with the status bar, and the
///   menu bar still opens from the top edge.
/// - The title bar is removed rather than hidden. A hidden one moved and zoomed the window only within the full screen's
///   top strip after going back to a window.
struct SessionView: View {
    @Environment(SessionController.self) private var session
    @State private var fillsScreen = false

    var body: some View {
        Group {
            if fillsScreen {
                screen
                    .ignoresSafeArea()
            } else {
                NavigationStack {
                    screen
                        .ignoresSafeArea(edges: [.horizontal, .bottom])
                        .navigationTitle(session.state?.host.title ?? "")
                        .navigationBarTitleDisplayMode(.inline)
                        .toolbar { toolbar }
                }
            }
        }
        .statusBarHidden(fillsScreen)
        // Hidden only when filling the screen, since in a window it also hides the window's controls.
        .persistentSystemOverlays(fillsScreen ? .hidden : .automatic)
        .background {
            Backdrop { fills in fillsScreen = fills }
        }
    }

    private var screen: some View {
        ZStack {
            Screen(connection: session.connection, layout: session.layout, cursor: session.cursor, syncsLanguage: session.syncsKeyboardLanguage) { mode in
                session.resize(mode)
            }

            if !session.shown {
                ProgressView()
                    .controlSize(.large)
                    .accessibilityLabel("Preparing Screen")
                    .foregroundStyle(.white)
                    .tint(.white)
            }
        }
        .background(.black)
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

private struct Screen: UIViewRepresentable {
    let connection: Connection?
    let layout: SessionController.Layout?
    let cursor: SessionController.Cursor?
    let syncsLanguage: Bool
    let onResize: (DisplayMode) -> Void

    func makeUIView(context: Context) -> ScreenView {
        ScreenView()
    }

    func updateUIView(_ view: ScreenView, context: Context) {
        view.connection = connection
        view.layout = layout
        view.cursor = cursor
        view.syncsLanguage = syncsLanguage
        view.onResize = onResize
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
