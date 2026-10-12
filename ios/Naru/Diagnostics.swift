import SwiftUI
import UIKit

// TestFlight로 실기기를 확인하는 동안 쓰는 임시 화면입니다. 디코더 지원과 지금 세션의 코덱, 하드웨어 키보드가 주는 글자와 입력 언어를 보입니다.
struct Diagnostics: View {
    let session: SessionController
    @State private var lines: [String] = []

    var body: some View {
        VStack(alignment: .leading) {
            Text("HEVC 4:4:4 하드웨어 디코딩: \(Video.decodesHevc444 ? "가능" : "불가")")
            Text("지금 세션의 코덱: \(session.codec.map { $0 == .hevc ? "HEVC 4:4:4" : "H.264" } ?? "세션 없음")")
            Text("마지막 세션 종료: \(session.closedDetail ?? "없음")")
                .textSelection(.enabled)
            Text("하드웨어 키보드로 쳐 보세요").font(.headline).padding(.top)
            ProbeView { line in lines.insert(line, at: 0) }
                .frame(height: 1)
            List(lines, id: \.self) { Text($0).font(.system(.body, design: .monospaced)) }
        }
        .padding()
    }
}

private struct ProbeView: UIViewRepresentable {
    let log: (String) -> Void

    func makeUIView(context: Context) -> ProbeInput {
        let view = ProbeInput()
        view.log = log
        DispatchQueue.main.async { view.becomeFirstResponder() }
        return view
    }

    func updateUIView(_ view: ProbeInput, context: Context) {}
}

final class ProbeInput: UIView {
    var log: (String) -> Void = { _ in }
    private var count = 0

    override init(frame: CGRect) {
        super.init(frame: frame)
        NotificationCenter.default.addObserver(forName: UITextInputMode.currentInputModeDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.report("언어 변경") }
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override var canBecomeFirstResponder: Bool { true }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        for press in presses {
            guard let key = press.key else { continue }
            count += 1
            log("\(count) 누름 code \(key.keyCode.rawValue) chars [\(key.characters)] ignoring [\(key.charactersIgnoringModifiers)] mods \(key.modifierFlags.rawValue) lang \(textInputMode?.primaryLanguage ?? "nil")")
        }
        super.pressesBegan(presses, with: event)
    }

    private func report(_ tag: String) {
        log("\(tag): textInputMode \(textInputMode?.primaryLanguage ?? "nil"), current \(UITextInputMode.activeInputModes.first?.primaryLanguage ?? "nil")")
    }
}
