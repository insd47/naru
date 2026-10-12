import SwiftUI
import UIKit

// 하드웨어 키보드가 주는 글자와 입력 언어를 확인하는 임시 화면입니다.
struct KeyProbe: View {
    @State private var lines: [String] = []

    var body: some View {
        VStack(alignment: .leading) {
            Text("하드웨어 키보드로 쳐 보세요").font(.headline)
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
