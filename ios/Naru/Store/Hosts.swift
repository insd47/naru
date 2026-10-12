import Foundation

/// The list of saved Macs, stored as JSON in user defaults.
@MainActor @Observable
final class Hosts {
    private(set) var list: [Host]

    init() {
        let data = UserDefaults.standard.data(forKey: Self.key)
        list = data.flatMap { try? JSONDecoder().decode([Host].self, from: $0) } ?? []
    }

    /// The saved Mac with this ID, or `nil` if there is none.
    func find(_ id: String) -> Host? {
        list.first { $0.id == id }
    }

    /// Replaces the entry with the same ID, or appends it if there is none.
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
