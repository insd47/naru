import Foundation

/// 모든 Mac에 함께 쓰는 설정입니다.
///
/// - 화질은 설정 앱(Settings.bundle)에서 바꾸고, 접속할 때 읽습니다.
/// - 클립보드 공유는 Mac의 Screen Sharing처럼 메뉴 막대의 편집 메뉴에서 바꿉니다.
/// - 키 배치(Alt와 Command)는 iPadOS의 보조 키 설정이 맡으므로 두지 않습니다.
@MainActor @Observable
final class Preferences {
    /// 패킷을 잃었을 때 지연과 화질 중 무엇을 우선할지. Screen Sharing 앱처럼 지연을 먼저 둡니다.
    var priority: Priority {
        UserDefaults.standard.string(forKey: "priority") == "quality" ? .quality : .latency
    }

    /// Mac과 클립보드를 주고받을지 여부
    var sharedClipboard: Bool {
        didSet { UserDefaults.standard.set(sharedClipboard, forKey: "sharedClipboard") }
    }

    init() {
        sharedClipboard = UserDefaults.standard.object(forKey: "sharedClipboard") as? Bool ?? true
    }
}
