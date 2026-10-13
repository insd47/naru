import UIKit

/// Translates hardware keys to X11 keysyms.
///
/// - Characters are sent as iPadOS produces them with the current input language's layout (`characters`). For Korean that yields Dubeolsik jamo that the Mac's input method composes,
///   so the core's layout translation (`language`) is not used.
/// - With Control, Option, or Command held, the key is a shortcut, so the Latin character at the physical position is sent.
/// - The Globe key and Caps Lock are not sent because iPadOS uses them to switch input languages. Caps Lock never arrives as a key anyway.
/// - A release sends the keysym sent on press, so no key stays held on the Mac if modifiers or the language changed in between.
@MainActor
struct Keys {
    private var pressed: [UIKeyboardHIDUsage: UInt32] = [:]

    /// Whether the key should be left for iPadOS to handle
    static func isSystem(_ key: UIKey) -> Bool {
        key.keyCode == .keyboardCapsLock || (key.keyCode == .keyboardErrorUndefined && key.modifierFlags.rawValue & globe != 0)
    }

    /// The keysym for a pressed key, or `nil` for a key that is not sent.
    mutating func press(_ key: UIKey) -> UInt32? {
        guard let keysym = Self.keysym(key) else {
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
    mutating func releaseAll() -> [UInt32] {
        defer { pressed.removeAll() }
        return Array(pressed.values)
    }

    private static func keysym(_ key: UIKey) -> UInt32? {
        if let special = special[key.keyCode.rawValue] {
            return special
        }

        let shortcut = !key.modifierFlags.isDisjoint(with: [.command, .control, .alternate])
        let scalars = key.characters.unicodeScalars

        if !shortcut, scalars.count == 1, let scalar = scalars.first, scalar.value >= 0x20, scalar.value != 0x7f {
            return keysym(scalar.value)
        }

        return latin(key.keyCode.rawValue).map { keysym($0.value) }
    }

    // Latin-1 maps to its character code, everything else to a Unicode keysym.
    private static func keysym(_ code: UInt32) -> UInt32 {
        (0x20...0x7e).contains(code) || (0xa0...0xff).contains(code) ? code : 0x0100_0000 + code
    }

    // Characters at each physical position on the US layout (HID usage codes)
    private static func latin(_ usage: Int) -> Unicode.Scalar? {
        switch usage {
        case 4...29: Unicode.Scalar(UInt8(ascii: "a") + UInt8(usage - 4))
        case 30...38: Unicode.Scalar(UInt8(ascii: "1") + UInt8(usage - 30))
        case 39: "0"
        case 45: "-"
        case 46: "="
        case 47: "["
        case 48: "]"
        case 49: "\\"
        case 51: ";"
        case 52: "'"
        case 53: "`"
        case 54: ","
        case 55: "."
        case 56: "/"
        default: nil
        }
    }

    // The Globe key arrives as this modifier bit with no HID code (the same bit as Fn on macOS).
    private static let globe = 1 << 23

    // The Mac maps Super to Command and Meta to Option.
    private static let special: [Int: UInt32] = {
        var keys: [Int: UInt32] = [
            224: 0xffe3, 225: 0xffe1, 226: 0xffe7, 227: 0xffeb,
            228: 0xffe4, 229: 0xffe2, 230: 0xffe8, 231: 0xffec,
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
