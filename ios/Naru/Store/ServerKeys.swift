import Foundation

/// Public keys of Macs connected to, saved on the first connection and checked for changes on later ones.
///
/// - The Mac's public key has no certificate chain, so the first key seen is trusted and the user is asked if it later changes.
enum ServerKeys {
    static func find(_ host: Host) -> Data? {
        try? Data(contentsOf: file(host))
    }

    /// Saves the key only when none is saved yet.
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
