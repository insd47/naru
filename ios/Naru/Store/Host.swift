import Foundation

/// One saved Mac. The password is kept separately by [Keychain].
struct Host: Codable, Identifiable, Hashable {
    var id = UUID().uuidString
    /// The user-given name. When empty, it is filled with the computer name the Mac reports on connection.
    var name = ""
    var address: String
    var username = ""
    var port = 5900
    /// Whether to connect with a virtual display sized to the iPad screen. When off, the Mac's physical display is shown.
    var virtualDisplay = true
    /// The time of the last successful connection
    var lastConnected: Date?

    /// The name to display, or the address when there is no name.
    var title: String { name.isEmpty ? address : name }
}
