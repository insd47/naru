import Foundation

/// 접속한 Mac의 공개키입니다. 처음 접속할 때 저장하고, 다음 접속부터 바뀌었는지 확인하는 데 씁니다.
///
/// - Mac의 공개키에는 인증서 체인이 없으므로, 처음 본 키를 믿고 이후 바뀌면 사용자에게 묻습니다.
enum ServerKeys {
    static func find(_ host: Host) -> Data? {
        try? Data(contentsOf: file(host))
    }

    /// 저장한 키가 없을 때만 저장합니다.
    static func remember(_ host: Host, der: Data) {
        let file = file(host)

        if !FileManager.default.fileExists(atPath: file.path()) {
            try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try? der.write(to: file)
        }
    }

    static func forget(_ host: Host) {
        try? FileManager.default.removeItem(at: file(host))
    }

    private static var directory: URL {
        .applicationSupportDirectory.appending(path: "server-keys")
    }

    private static func file(_ host: Host) -> URL {
        directory.appending(path: host.address.replacing(/[^A-Za-z0-9.:-]/, with: "_") + ".der")
    }
}
