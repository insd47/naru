import SwiftUI

/// The session screen. Shows the Mac's screen under a title bar, with a loading state until the first frame arrives.
///
/// - The title bar follows Screen Sharing's toolbar: the way back to the connections, the Mac's name, and the Mac's
///   window shortcuts. It shows while the app is in a window and hides with the status bar when the app fills the
///   screen, where the menu bar still opens from the top edge.
struct SessionView: View {
    @Environment(SessionController.self) private var session
    @State private var fillsScreen = false

    var body: some View {
        NavigationStack {
            ZStack {
                Screen(connection: session.connection, layout: session.layout, cursor: session.cursor, syncsLanguage: session.syncsKeyboardLanguage) { mode in
                    session.resize(mode)
                }
                .ignoresSafeArea(edges: fillsScreen ? .all : [.horizontal, .bottom])

                if !session.shown {
                    ProgressView()
                        .controlSize(.large)
                        .accessibilityLabel("Preparing Screen")
                        .foregroundStyle(.white)
                        .tint(.white)
                }
            }
            .background(.black)
            .navigationTitle(session.state?.host.title ?? "")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { toolbar }
            .toolbar(fillsScreen ? .hidden : .visible, for: .navigationBar)
        }
        .statusBarHidden(fillsScreen)
        // Hidden only when filling the screen, since in a window it also hides the window's controls.
        .persistentSystemOverlays(fillsScreen ? .hidden : .automatic)
        .background {
            ScreenFill { fills in
                withAnimation(.snappy) { fillsScreen = fills }
            }
        }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .topBarLeading) {
            Button("Connections", systemImage: "chevron.backward") { session.presented = false }
        }

        // The Mac's default shortcuts: Control-Up Arrow, Control-Down Arrow, F11
        ToolbarItemGroup(placement: .topBarTrailing) {
            Button("Mission Control", systemImage: "rectangle.3.group") { session.press([0xffe3, 0xff52]) }
            Button("App Windows", systemImage: "macwindow.on.rectangle") { session.press([0xffe3, 0xff54]) }
            Button("Desktop", systemImage: "menubar.dock.rectangle") { session.press([0xffc8]) }
            Menu("More", systemImage: "ellipsis") {
                Button("Disconnect", systemImage: "xmark", role: .destructive) { session.close() }
            }
        }
    }
}

// The window behind the session: reports whether it fills its screen, and drags the window where nothing in front
// takes the touch.
//
// - iPadOS has no property for filling the screen, so the window and screen sizes are compared, either way round for
//   rotation.
// - The title bar drags the window only in its upper half. Touches that fall through its lower half land here.
private struct ScreenFill: UIViewRepresentable {
    let onChange: (Bool) -> Void

    func makeUIView(context: Context) -> FillView {
        let view = FillView()
        view.addInteraction(UIWindowSceneDragInteraction())
        return view
    }

    func updateUIView(_ view: FillView, context: Context) {
        view.onChange = onChange
    }

    final class FillView: UIView {
        var onChange: ((Bool) -> Void)?
        private var fills: Bool?

        override func layoutSubviews() {
            super.layoutSubviews()

            guard let window, let screen = window.windowScene?.screen.bounds.size else {
                return
            }

            let size = window.bounds.size
            let now = size == screen || size == CGSize(width: screen.height, height: screen.width)

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
