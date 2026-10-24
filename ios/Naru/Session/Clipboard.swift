import UIKit
import UniformTypeIdentifiers
import os

/// Shares the clipboard with the Mac, which offers the kinds it holds and sends the data when asked.
///
/// - The Mac's data is fetched as soon as it offers new content and goes on the pasteboard as is. Content promised to
///   the pasteboard would gain nothing: iPadOS loads every promise right after it is set, and a promise answered on
///   the main thread then deadlocks with the pasteboard.
/// - Content copied in another app is offered when Naru comes to the front, and its data is sent when the Mac asks for
///   it, as when it is pasted there. Reading the data then makes iPadOS ask for paste permission. A paste the user
///   makes in Naru, with Command-V or the Edit menu, reads it without asking, so its data goes to the Mac right away.
/// - What this class put on the pasteboard carries a private marker kind and is never offered back. Its change count
///   cannot tell, because iPadOS counts a write made in the background again when the app comes to the front.
/// - An image is also offered as PNG, which Mac apps paste. They do not paste every image kind iPadOS holds, such as WebP.
/// - Content is never logged, only kinds and sizes.
@MainActor
final class Clipboard {
    private static let marker = "dev.insd.naru.mac"
    private static let png = "public.png"

    private let connection: Connection
    private let preferences: Preferences
    // The pasteboard's change count when this class last read it
    private var change = UIPasteboard.general.changeCount
    // Kinds offered to the Mac, whose data it may ask for
    private var offered: [String] = []

    init(connection: Connection, preferences: Preferences) {
        self.connection = connection
        self.preferences = preferences
    }

    /// The Mac's clipboard changed to content of these kinds.
    func offered(_ kinds: [String]) {
        guard preferences.sharedClipboard, !kinds.isEmpty else {
            return
        }

        connection.fetchClipboard()
    }

    /// The Mac's clipboard data arrived.
    func received(_ items: [ClipboardItem]) {
        guard preferences.sharedClipboard, !items.isEmpty else {
            return
        }

        var item = [Self.marker: Data()]
        for received in items { item[received.kind] = received.data }

        UIPasteboard.general.setItems([item], options: [.localOnly: true])
        log.log("put the Mac's clipboard on the pasteboard: \(Self.describe(items), privacy: .public)")
    }

    /// Offers content copied in another app. Called when the app comes to the front.
    func sync() {
        guard preferences.sharedClipboard, UIPasteboard.general.changeCount != change else {
            return
        }

        send()
    }

    /// Offers the pasteboard's content whether or not it changed. Backs Send Clipboard in the Edit menu.
    func send() {
        guard let kinds = takeKinds() else {
            return
        }

        connection.offerClipboard(kinds)
        log.log("offered the clipboard to the Mac: \(kinds.joined(separator: ", "), privacy: .public)")
    }

    /// Sends the pasteboard's data for a paste the user made in Naru, before the Mac's own paste.
    func paste() {
        guard preferences.sharedClipboard, takeKinds() != nil else {
            return
        }

        sendData()
    }

    /// The Mac wants the data of what was offered, as when it is pasted there.
    func requested() {
        sendData()
    }

    // The kinds on the pasteboard to offer, now remembered as offered, or `nil` for nothing to offer or content that
    // came from the Mac.
    private func takeKinds() -> [String]? {
        let pasteboard = UIPasteboard.general
        change = pasteboard.changeCount

        // Dynamic identifiers stand for types without a name and mean nothing on the Mac.
        let types = pasteboard.types
        var kinds = types.filter { !$0.hasPrefix("dyn.") }

        if !kinds.contains(Self.png), kinds.contains(where: Self.isImage) {
            kinds.insert(Self.png, at: 0)
        }

        guard !kinds.isEmpty, !types.contains(Self.marker) else {
            return nil
        }

        offered = kinds
        return kinds
    }

    private func sendData() {
        let pasteboard = UIPasteboard.general
        let items = offered.compactMap { kind in
            (pasteboard.data(forPasteboardType: kind) ?? (kind == Self.png ? Self.png(from: pasteboard) : nil))
                .map { ClipboardItem(kind: kind, data: $0) }
        }

        guard connection.sendClipboard(items) else {
            log.error("clipboard of \(items.reduce(0) { $0 + $1.data.count }) bytes is too large to send")
            return
        }

        log.log("sent clipboard data to the Mac: \(Self.describe(items), privacy: .public)")
    }

    private static func isImage(_ kind: String) -> Bool {
        UTType(kind)?.conforms(to: .image) == true
    }

    // The first image on the pasteboard that iPadOS can decode, as PNG
    private static func png(from pasteboard: UIPasteboard) -> Data? {
        pasteboard.types.lazy
            .filter(isImage)
            .compactMap { pasteboard.data(forPasteboardType: $0).flatMap(UIImage.init(data:))?.pngData() }
            .first
    }

    private static func describe(_ items: [ClipboardItem]) -> String {
        items.map { "\($0.kind) \($0.data.count) bytes" }.joined(separator: ", ")
    }
}

private let log = Logger(subsystem: Diagnostics.subsystem, category: "Clipboard")
