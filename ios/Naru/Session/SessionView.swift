import SwiftUI

/// The session screen. Shows the Mac's screen edge to edge, with a loading state until the first frame arrives.
///
/// - Returning to the connection list and disconnecting live in the menu bar. With a finger, swipe down from the top of the screen to open it.
struct SessionView: View {
    @Environment(SessionController.self) private var session

    var body: some View {
        ZStack {
            Screen(connection: session.connection, layout: session.layout, cursor: session.cursor, syncsLanguage: session.syncsKeyboardLanguage) { mode in
                session.resize(mode)
            }
            .ignoresSafeArea()

            if !session.shown {
                ProgressView("Preparing Screen")
                    .controlSize(.large)
                    .foregroundStyle(.white)
                    .tint(.white)
            }
        }
        .background(.black)
        .statusBarHidden()
        .persistentSystemOverlays(.hidden)
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
