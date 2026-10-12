import SwiftUI

/// 세션 화면. Mac 화면을 꽉 채워 보이고, 첫 화면이 올 때까지 준비 중임을 보입니다.
///
/// - 손가락으로 탭하면 연결 목록으로 돌아가기와 연결 끊기 버튼을 보이거나 숨깁니다. 포인터를 쓰면 메뉴 막대의 연결 메뉴에도 있습니다.
struct SessionView: View {
    @Environment(SessionController.self) private var session
    @State private var controls = false

    var body: some View {
        ZStack {
            Screen(connection: session.connection, layout: session.layout, cursor: session.cursor) {
                withAnimation { controls.toggle() }
            } onResize: { mode in
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
        .overlay(alignment: .top) {
            if controls {
                HStack {
                    Button("Connections", systemImage: "chevron.backward") { session.presented = false }
                    Button("Disconnect", systemImage: "xmark", role: .destructive) { session.close() }
                }
                .buttonStyle(.glass)
                .padding()
                .transition(.move(edge: .top).combined(with: .opacity))
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
    let onTap: () -> Void
    let onResize: (DisplayMode) -> Void

    func makeUIView(context: Context) -> ScreenView {
        ScreenView()
    }

    func updateUIView(_ view: ScreenView, context: Context) {
        view.connection = connection
        view.layout = layout
        view.cursor = cursor
        view.onTap = onTap
        view.onResize = onResize
    }
}
