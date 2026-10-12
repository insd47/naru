import UIKit

/// 하드웨어 키를 X11 keysym으로 바꿉니다.
///
/// - 글자는 iPadOS가 지금 입력 언어의 배열을 적용한 글자(`characters`)를 보냅니다. 한국어면 2벌식 자모가 오고 Mac의 입력기가 조합하므로,
///   코어의 배열 변환(`language`)은 쓰지 않습니다.
/// - Control, Option, Command가 눌려 있으면 단축키이므로 물리 자리의 라틴 문자를 보냅니다.
/// - 지구본 키와 Caps Lock은 iPadOS의 입력 언어 전환에 쓰이므로 보내지 않습니다. Caps Lock은 키로 오지도 않습니다.
/// - 뗄 때는 누를 때 보낸 keysym을 보냅니다. 그 사이 수식 키나 언어가 바뀌어도 Mac에 눌린 키가 남지 않게 합니다.
struct Keys {
    private var pressed: [UIKeyboardHIDUsage: UInt32] = [:]

    /// iPadOS가 처리하도록 넘길 키인지 여부
    static func isSystem(_ key: UIKey) -> Bool {
        key.keyCode == .keyboardCapsLock || (key.keyCode == .keyboardErrorUndefined && key.modifierFlags.rawValue & globe != 0)
    }

    /// 누른 키의 keysym. 보내지 않는 키면 `nil`입니다.
    mutating func press(_ key: UIKey) -> UInt32? {
        guard let keysym = Self.keysym(key) else {
            return nil
        }

        pressed[key.keyCode] = keysym
        return keysym
    }

    /// 뗀 키의 keysym. 누를 때 보내지 않은 키면 `nil`입니다.
    mutating func release(_ key: UIKey) -> UInt32? {
        pressed.removeValue(forKey: key.keyCode)
    }

    /// 눌린 채로 남은 키를 모두 뗍니다. 화면을 떠날 때 Mac에 키가 눌린 채 남지 않게 합니다.
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

    // 라틴-1은 문자 코드 그대로, 그 밖은 유니코드 keysym입니다.
    private static func keysym(_ code: UInt32) -> UInt32 {
        (0x20...0x7e).contains(code) || (0xa0...0xff).contains(code) ? code : 0x0100_0000 + code
    }

    // US 배열의 물리 자리 글자(HID 사용 코드)
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

    // 지구본 키는 HID 코드 없이 이 수식 비트로 옵니다(macOS의 Fn과 같은 자리).
    private static let globe = 1 << 23

    // Mac은 Super를 Command로, Meta를 Option으로 받습니다.
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
