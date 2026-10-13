import os
import SwiftUI
import UIKit

// A temporary screen for testing on real devices. Shows decoder support, the current session's codec, and the characters and input language the hardware keyboard produces.
struct Diagnostics: View {
    let session: SessionController
    @State private var lines: [String] = []

    var body: some View {
        VStack(alignment: .leading) {
            Text("HEVC 4:4:4 hardware decoding: \(Video.decodesHevc444 ? "supported" : "unsupported")")
            Text("Current session codec: \(session.codec.map { $0 == .hevc ? "HEVC 4:4:4" : "H.264" } ?? "no session")")
            Text("Last session ended: \(session.closedDetail ?? "none")")
                .textSelection(.enabled)
            Text("Type on the hardware keyboard").font(.headline).padding(.top)
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
        view.show = log
        DispatchQueue.main.async { view.becomeFirstResponder() }
        return view
    }

    func updateUIView(_ view: ProbeInput, context: Context) {}
}

final class ProbeInput: UIView {
    var show: (String) -> Void = { _ in }
    private let logger = Logger(subsystem: "dev.insd.naru", category: "Diagnostics")

    private func log(_ line: String) {
        logger.info("\(line, privacy: .public)")
        show(line)
    }
    private var count = 0

    override init(frame: CGRect) {
        super.init(frame: frame)
        NotificationCenter.default.addObserver(forName: UITextInputMode.currentInputModeDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.report("language changed") }
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override var canBecomeFirstResponder: Bool { true }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        for press in presses {
            guard let key = press.key else { continue }
            count += 1
            log("\(Self.time()) \(count) press code \(key.keyCode.rawValue) chars [\(key.characters)] ignoring [\(key.charactersIgnoringModifiers)] mods \(key.modifierFlags.rawValue) lang \(textInputMode?.primaryLanguage ?? "nil")")
        }
        super.pressesBegan(presses, with: event)
    }

    override func pressesEnded(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        for press in presses {
            guard let key = press.key else { continue }
            log("\(Self.time()) release code \(key.keyCode.rawValue) lang \(textInputMode?.primaryLanguage ?? "nil")")
        }
        super.pressesEnded(presses, with: event)
    }

    // Seconds and milliseconds within the minute, enough to see the gap between a language switch and the next key
    private static func time() -> String {
        let now = Date().timeIntervalSince1970
        return String(format: "%06.3f", now.truncatingRemainder(dividingBy: 60))
    }

    private func report(_ tag: String) {
        log("\(Self.time()) \(tag): textInputMode \(textInputMode?.primaryLanguage ?? "nil"), current \(UITextInputMode.activeInputModes.first?.primaryLanguage ?? "nil")")
    }
}
