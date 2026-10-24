import SwiftUI

/// What Naru adds to the menu bar for a session. The session's title bar offers the same under More, since iPadOS shows
/// the menu bar only for windowed apps.
enum SessionCommand {
    case sharedClipboard, sendClipboard, syncKeyboardLanguage, disconnect

    var title: String {
        switch self {
        case .sharedClipboard: "Use Shared Clipboard"
        case .sendClipboard: "Send Clipboard"
        case .syncKeyboardLanguage: "Sync Keyboard Language"
        case .disconnect: "Disconnect"
        }
    }

    var systemImage: String {
        switch self {
        case .sharedClipboard: "list.clipboard"
        case .sendClipboard: "doc.on.clipboard"
        case .syncKeyboardLanguage: "globe"
        case .disconnect: "xmark"
        }
    }

    /// A toggle's value, or `nil` for a button
    @MainActor
    func isOn(_ session: SessionController) -> Bool? {
        switch self {
        case .sharedClipboard: session.preferences.sharedClipboard
        case .syncKeyboardLanguage: session.preferences.syncKeyboardLanguage
        case .sendClipboard, .disconnect: nil
        }
    }

    /// Whether it applies now. Buttons need a session.
    @MainActor
    func available(_ session: SessionController) -> Bool {
        isOn(session) != nil || session.connection != nil
    }

    @MainActor
    func perform(_ session: SessionController) {
        switch self {
        case .sharedClipboard: session.preferences.sharedClipboard.toggle()
        case .sendClipboard: session.sendClipboard()
        case .syncKeyboardLanguage: session.preferences.syncKeyboardLanguage.toggle()
        case .disconnect: session.close()
        }
    }

    /// The command as a UIKit menu item, with its value at this moment
    @MainActor
    func action(_ session: SessionController) -> UIAction {
        let action = UIAction(title: title, image: UIImage(systemName: systemImage)) { _ in perform(session) }
        action.state = isOn(session) == true ? .on : .off
        action.attributes = self == .disconnect ? .destructive : []
        if !available(session) { action.attributes.insert(.disabled) }
        return action
    }
}

/// A command in a SwiftUI menu: a toggle for a setting, otherwise a button.
struct SessionCommandItem: View {
    let command: SessionCommand
    let session: SessionController

    init(_ command: SessionCommand, session: SessionController) {
        self.command = command
        self.session = session
    }

    var body: some View {
        if let on = command.isOn(session) {
            Toggle(command.title, systemImage: command.systemImage, isOn: Binding(get: { on }, set: { _ in command.perform(session) }))
        } else {
            Button(command.title, systemImage: command.systemImage, role: command == .disconnect ? .destructive : nil) { command.perform(session) }
                .disabled(!command.available(session))
        }
    }
}
