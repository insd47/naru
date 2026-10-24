import UIKit

/// Translates hardware keys to X11 keysyms of what the Mac should type.
///
/// - The input language is iPadOS's, read after the text system has seen the press, since a pending language switch takes effect only then.
/// - For a language with a Mac layout (Korean), the core's `character` gives what the physical Latin key types there, which also follows the Mac's
///   secure input and its own layout of that language. For other languages, the character iPadOS produced is sent, which keeps layouts such as QWERTZ.
/// - With Control, Option, or Command held, the key is a shortcut, so the Latin character at the physical position is sent.
/// - The Globe key and Caps Lock are not sent because iPadOS uses them to switch input languages.
/// - A release sends the keysym sent on press, so no key stays held on the Mac if modifiers or the language changed in between.
@MainActor
struct Keys {
    private var pressed: [UIKeyboardHIDUsage: UInt32] = [:]

    /// Whether the key should be left for iPadOS to handle
    static func isSystem(_ key: UIKey) -> Bool {
        key.keyCode == .keyboardCapsLock || (key.keyCode == .keyboardErrorUndefined && key.modifierFlags.rawValue & globe != 0)
    }

    /// Whether the key types or edits text rather than triggering a shortcut, moving, or being a modifier. Such keys also go to the text system,
    /// which reports their repeats.
    static func isTyping(_ key: UIKey) -> Bool {
        key.modifierFlags.isDisjoint(with: shortcut) && (special[key.keyCode.rawValue] == nil || editing.contains(key.keyCode))
    }

    // Arrows get no repeats from the text system, so they stay out of it.
    private static let editing: Set<UIKeyboardHIDUsage> = [
        .keyboardReturnOrEnter, .keypadEnter, .keyboardDeleteOrBackspace, .keyboardTab, .keyboardSpacebar,
    ]

    private static let shortcut: UIKeyModifierFlags = [.command, .control, .alternate]

    /// Whether the key is an arrow, Home, End, Page Up, Page Down, or forward delete, without a shortcut modifier
    static func isNavigation(_ key: UIKey) -> Bool {
        (74...82).contains(key.keyCode.rawValue) && key.modifierFlags.isDisjoint(with: shortcut)
    }

    /// The keysym for a pressed key, or `nil` for a key that is not sent.
    ///
    /// - `language` is the input language after the text system saw the press, and `source` the Mac's latest input source.
    /// - `switched` tells that the language changed while the text system saw the press, so the key's characters are still the old language's.
    mutating func press(_ key: UIKey, language: String?, switched: Bool, source: InputSource?) -> UInt32? {
        guard let keysym = Self.keysym(key, language: language, switched: switched, source: source) else {
            return nil
        }

        pressed[key.keyCode] = keysym
        return keysym
    }

    /// The keysym for a released key, or `nil` if nothing was sent on press.
    mutating func release(_ key: UIKey) -> UInt32? {
        pressed.removeValue(forKey: key.keyCode)
    }

    /// Releases every key still held, so no key stays down on the Mac after leaving the screen.
    mutating func releaseAll() -> [(key: UIKeyboardHIDUsage, keysym: UInt32)] {
        defer { pressed.removeAll() }
        return pressed.map { ($0.key, $0.value) }
    }

    private static func keysym(_ key: UIKey, language: String?, switched: Bool, source: InputSource?) -> UInt32? {
        let usage = key.keyCode.rawValue

        if let special = special[usage] {
            return special
        }

        guard isTyping(key) else {
            return latin(usage, shifted: false).map { keysym($0.value) }
        }

        let position = latin(usage, shifted: key.modifierFlags.contains(.shift)).map { keysym($0.value) }

        if let position, let language, let typed = character(latin: position, language: language, source: source) {
            return typed
        }

        // Right after a switch to a language typed in Latin, the key's characters are still the old language's.
        if switched, let position {
            return position
        }

        let scalars = key.characters.unicodeScalars

        if scalars.count == 1, let scalar = scalars.first, scalar.value >= 0x20, scalar.value != 0x7f {
            return keysym(scalar.value)
        }

        return latin(usage, shifted: false).map { keysym($0.value) }
    }

    // Latin-1 maps to its character code, everything else to a Unicode keysym.
    private static func keysym(_ code: UInt32) -> UInt32 {
        (0x20...0x7e).contains(code) || (0xa0...0xff).contains(code) ? code : 0x0100_0000 + code
    }

    // Characters at each physical position on the US layout (HID usage codes), plain and with Shift
    private static func latin(_ usage: Int, shifted: Bool) -> Unicode.Scalar? {
        let (plain, shift): (Character, Character)

        switch usage {
        case 4...29:
            let letter = Character(Unicode.Scalar(UInt8(ascii: "a") + UInt8(usage - 4)))
            (plain, shift) = (letter, Character(letter.uppercased()))
        case 30...39:
            (plain, shift) = (Array("1234567890")[usage - 30], Array("!@#$%^&*()")[usage - 30])
        case 45: (plain, shift) = ("-", "_")
        case 46: (plain, shift) = ("=", "+")
        case 47: (plain, shift) = ("[", "{")
        case 48: (plain, shift) = ("]", "}")
        case 49: (plain, shift) = ("\\", "|")
        case 51: (plain, shift) = (";", ":")
        case 52: (plain, shift) = ("'", "\"")
        case 53: (plain, shift) = ("`", "~")
        case 54: (plain, shift) = (",", "<")
        case 55: (plain, shift) = (".", ">")
        case 56: (plain, shift) = ("/", "?")
        default: return nil
        }

        return (shifted ? shift : plain).unicodeScalars.first
    }

    // The Globe key arrives as this modifier bit with no HID code (the same bit as Fn on macOS).
    private static let globe = 1 << 23

    // Command goes as Alt and Option as Meta, as Apple's client sends them.
    private static let special: [Int: UInt32] = {
        var keys: [Int: UInt32] = [
            224: 0xffe3, 225: 0xffe1, 226: 0xffe7, 227: 0xffe9,
            228: 0xffe4, 229: 0xffe2, 230: 0xffe8, 231: 0xffea,
            40: 0xff0d, 88: 0xff0d, 41: 0xff1b, 42: 0xff08, 43: 0xff09, 44: 0x20,
            73: 0xff63, 74: 0xff50, 75: 0xff55, 76: 0xffff, 77: 0xff57, 78: 0xff56,
            79: 0xff53, 80: 0xff51, 81: 0xff54, 82: 0xff52,
        ]

        for index in 0..<12 {
            keys[58 + index] = 0xffbe + UInt32(index)
        }

        return keys
    }()
}
