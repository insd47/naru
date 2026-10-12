import Foundation

/// 저장한 Mac 하나. 비밀번호는 [Keychain]이 따로 둡니다.
struct Host: Codable, Identifiable, Hashable {
    var id = UUID().uuidString
    /// 사용자가 붙인 이름. 비어 있으면 접속할 때 Mac이 알려 준 컴퓨터 이름으로 채웁니다.
    var name = ""
    var address: String
    var username = ""
    var port = 5900
    /// iPad 화면 크기의 가상 디스플레이로 접속할지 여부. 끄면 Mac의 실제 화면을 받습니다.
    var virtualDisplay = true
    /// 마지막으로 접속에 성공한 시각
    var lastConnected: Date?

    /// 화면에 보일 이름. 이름이 없으면 주소입니다.
    var title: String { name.isEmpty ? address : name }
}
