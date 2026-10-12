import SwiftUI

/// 세션 화면. Mac 화면을 꽉 채워 보이고, 첫 화면이 올 때까지 준비 중임을 보입니다.
///
/// - 연결 목록으로 돌아가기와 연결 끊기는 메뉴 막대에 둡니다. 손가락으로는 화면 위에서 쓸어내려 메뉴 막대를 엽니다.
struct SessionView: View {
    @Environment(SessionController.self) private var session

    var body: some View {
        ZStack {
            Screen(connection: session.connection, layout: session.layout, cursor: session.cursor, touchCursor: session.touchCursor) { mode in
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
    let touchCursor: CGPoint?
    let onResize: (DisplayMode) -> Void

    func makeUIView(context: Context) -> ScreenView {
        ScreenView()
    }

    func updateUIView(_ view: ScreenView, context: Context) {
        view.connection = connection
        view.layout = layout
        view.cursor = cursor
        view.touchCursor = touchCursor
        view.onResize = onResize
    }
}
