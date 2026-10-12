// macOS 키보드 배열에서 Naru가 쓰는 변환 표를 뽑습니다. 입력 소스 설정은 읽기만 합니다.
//
//     swift tools/layouts.swift > src/session/keyboard/layouts.txt
//
// - ABC와, 라틴 문자를 칠 수 없는 배열(2벌식, 러시아어, 주음 등)만 담습니다. 라틴 배열에는 글자를 그대로 보내므로 표가 필요 없습니다.
// - 열은 메인 키 블록에서 ABC가 공백이 아닌 글자 하나를 내는 키 자리입니다. keysym 하나로 보낼 수 없거나 데드 키인 자리는 U+FFFD로 둡니다.
import Carbon
import Foundation

func property(_ source: TISInputSource, _ key: CFString) -> AnyObject? {
    TISGetInputSourceProperty(source, key).map { Unmanaged<AnyObject>.fromOpaque($0).takeUnretainedValue() }
}

let keyboardType = UInt32(LMGetKbdType())
let shift = UInt32(shiftKey >> 8)
let empty: Character = "\u{FFFD}"

// 키 코드 하나를 수식키와 함께 누를 때 나오는 글자. keysym 하나로 보낼 수 없거나(코드 포인트 여러 개, 공백, 제어 문자) 데드 키면 `nil`입니다.
func character(_ layout: UnsafePointer<UCKeyboardLayout>, _ code: Int, _ modifiers: UInt32) -> Character? {
    var dead: UInt32 = 0
    var length = 0
    var chars = [UniChar](repeating: 0, count: 8)
    let status = UCKeyTranslate(layout, UInt16(code), UInt16(kUCKeyActionDown), modifiers, keyboardType, 0, &dead, chars.count, &length, &chars)
    guard status == noErr, dead == 0, length > 0 else { return nil }
    let text = String(utf16CodeUnits: chars, count: length)
    guard text.unicodeScalars.count == 1, let scalar = text.unicodeScalars.first, scalar.value > 0x20, scalar.value != 0x7f else { return nil }
    return Character(scalar)
}

func rows(_ source: TISInputSource, _ codes: [Int]) -> (String, String)? {
    guard let data = property(source, kTISPropertyUnicodeKeyLayoutData) as? Data else { return nil }
    return data.withUnsafeBytes { raw in
        let layout = raw.bindMemory(to: UCKeyboardLayout.self).baseAddress!
        let base = String(codes.map { character(layout, $0, 0) ?? empty })
        let shifted = String(codes.map { character(layout, $0, shift) ?? empty })
        return (base, shifted)
    }
}

let filter = [kTISPropertyInputSourceType: kTISTypeKeyboardLayout] as CFDictionary
let layouts = (TISCreateInputSourceList(filter, true).takeRetainedValue() as! [TISInputSource])
    .map { (id: property($0, kTISPropertyInputSourceID) as! String, source: $0) }
    .sorted { $0.id < $1.id }
    // 같은 ID가 두 번 나오는 배열이 있습니다.
    .reduce(into: [(id: String, source: TISInputSource)]()) { if $0.last?.id != $1.id { $0.append($1) } }
guard let abc = layouts.first(where: { $0.id == "com.apple.keylayout.ABC" }) else { fatalError("ABC 배열이 없습니다") }

// 메인 키 블록에서 ABC가 글자를 내는 자리만 열로 씁니다. 키패드는 숫자가 겹치므로 뺍니다.
let codes = (0..<64).filter { code in
    (property(abc.source, kTISPropertyUnicodeKeyLayoutData) as! Data).withUnsafeBytes {
        character($0.bindMemory(to: UCKeyboardLayout.self).baseAddress!, code, 0) != nil
    }
}

let version = ProcessInfo.processInfo.operatingSystemVersion
print("# tools/layouts.swift가 만든 파일입니다. 손으로 고치지 마십시오.")
print("# macOS \(version.majorVersion).\(version.minorVersion).\(version.patchVersion), keyboard type \(keyboardType)")
print("# 키 코드: \(codes.map(String.init).joined(separator: " "))")
print("# 입력 소스 ID<TAB>기본<TAB>Shift")

for (id, source) in layouts {
    let ascii = property(source, kTISPropertyInputSourceIsASCIICapable) as? Bool ?? true
    guard id == abc.id || !ascii, let (base, shifted) = rows(source, codes) else { continue }
    print("\(id)\t\(base)\t\(shifted)")
}
