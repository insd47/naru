import Foundation

/// 저장한 Mac 목록입니다. 앱 설정에 JSON으로 둡니다.
@MainActor @Observable
final class Hosts {
    private(set) var list: [Host]

    init() {
        let data = UserDefaults.standard.data(forKey: Self.key)
        list = data.flatMap { try? JSONDecoder().decode([Host].self, from: $0) } ?? []
    }

    /// 이 ID로 저장한 Mac. 없으면 `nil`입니다.
    func find(_ id: String) -> Host? {
        list.first { $0.id == id }
    }

    /// 같은 ID가 있으면 바꾸고, 없으면 끝에 더합니다.
    func save(_ host: Host) {
        if let index = list.firstIndex(where: { $0.id == host.id }) {
            list[index] = host
        } else {
            list.append(host)
        }

        store()
    }

    func remove(_ host: Host) {
        list.removeAll { $0.id == host.id }
        store()
    }

    private func store() {
        UserDefaults.standard.set(try? JSONEncoder().encode(list), forKey: Self.key)
    }

    private static let key = "hosts"
}
