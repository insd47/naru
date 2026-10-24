//! macOS virtual key codes (`kVK_*` in Carbon's `Events.h`) of physical keys, which go with keys to the Mac.

/// The macOS virtual key code to send with a key, or `None` for a key the Mac has no code for.
///
/// - A modifier's comes from its keysym, which names the Mac key the client made of it: a PC's Alt is Command, so it
///   is pressed as Command whatever its USB usage.
/// - Any other key's comes from its USB HID keyboard usage (page 7), the physical key.
pub fn keycode(keysym: u32, usage: Option<u16>) -> Option<u16> {
    let modifier = match keysym {
        0xffe1 => 0x38, // Shift_L
        0xffe2 => 0x3c, // Shift_R
        0xffe3 => 0x3b, // Control_L
        0xffe4 => 0x3e, // Control_R
        0xffe7 => 0x3a, // Meta_L, Option
        0xffe8 => 0x3d, // Meta_R, right Option
        0xffe9 => 0x37, // Alt_L, Command
        0xffea => 0x36, // Alt_R, right Command
        _ => return usage.and_then(mac_keycode),
    };

    Some(modifier)
}

fn mac_keycode(usage: u16) -> Option<u16> {
    let code = match usage {
        // Letters a to z
        0x04..=0x1d => LETTERS[usize::from(usage - 0x04)],
        // Digits 1 to 9, then 0
        0x1e..=0x27 => DIGITS[usize::from(usage - 0x1e)],
        0x28 => 0x24,        // Return
        0x29 => 0x35,        // Escape
        0x2a => 0x33,        // Delete (Backspace)
        0x2b => 0x30,        // Tab
        0x2c => 0x31,        // Space
        0x2d => 0x1b,        // -
        0x2e => 0x18,        // =
        0x2f => 0x21,        // [
        0x30 => 0x1e,        // ]
        0x31 | 0x32 => 0x2a, // \ and the ISO key beside Return
        0x33 => 0x29,        // ;
        0x34 => 0x27,        // '
        0x35 => 0x32,        // `
        0x36 => 0x2b,        // ,
        0x37 => 0x2f,        // .
        0x38 => 0x2c,        // /
        0x39 => 0x39,        // Caps Lock
        // F1 to F12
        0x3a..=0x45 => FUNCTIONS[usize::from(usage - 0x3a)],
        0x49 => 0x72, // Insert, Help on a Mac
        0x4a => 0x73, // Home
        0x4b => 0x74, // Page Up
        0x4c => 0x75, // Forward Delete
        0x4d => 0x77, // End
        0x4e => 0x79, // Page Down
        0x4f => 0x7c, // Right
        0x50 => 0x7b, // Left
        0x51 => 0x7d, // Down
        0x52 => 0x7e, // Up
        0x53 => 0x47, // Num Lock, Clear on a Mac
        0x54 => 0x4b, // Keypad /
        0x55 => 0x43, // Keypad *
        0x56 => 0x4e, // Keypad -
        0x57 => 0x45, // Keypad +
        0x58 => 0x4c, // Keypad Enter
        // Keypad 1 to 9, then 0
        0x59..=0x62 => KEYPAD[usize::from(usage - 0x59)],
        0x63 => 0x41, // Keypad .
        0x64 => 0x0a, // The ISO key beside the left Shift
        0x67 => 0x51, // Keypad =
        // F13 to F20
        0x68..=0x6f => [0x69, 0x6b, 0x71, 0x6a, 0x40, 0x4f, 0x50, 0x5a][usize::from(usage - 0x68)],
        0xe0 => 0x3b, // Left Control
        0xe1 => 0x38, // Left Shift
        0xe2 => 0x3a, // Left Option (Alt)
        0xe3 => 0x37, // Left Command (GUI)
        0xe4 => 0x3e, // Right Control
        0xe5 => 0x3c, // Right Shift
        0xe6 => 0x3d, // Right Option
        0xe7 => 0x36, // Right Command
        _ => return None,
    };

    Some(code)
}

const LETTERS: [u16; 26] = [
    0x00, 0x0b, 0x08, 0x02, 0x0e, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2e, 0x2d, 0x1f, 0x23, 0x0c, 0x0f, 0x01,
    0x11, 0x20, 0x09, 0x0d, 0x07, 0x10, 0x06,
];
const DIGITS: [u16; 10] = [0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1a, 0x1c, 0x19, 0x1d];
const FUNCTIONS: [u16; 12] = [0x7a, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6d, 0x67, 0x6f];
const KEYPAD: [u16; 10] = [0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5b, 0x5c, 0x52];

#[cfg(test)]
mod tests {
    use super::{keycode, mac_keycode};

    // The codes Apple's client sent in the 2026-10-19 relay capture: F (for both f and ㄹ), L, D, R, 8, 0, Shift, and
    // Caps Lock.
    #[test]
    fn keys_match_apple() {
        let usages = [0x09, 0x0f, 0x07, 0x15, 0x25, 0x27, 0xe1, 0x39];
        let codes = [0x03, 0x25, 0x02, 0x0f, 0x1c, 0x1d, 0x38, 0x39];
        assert_eq!(usages.map(mac_keycode), codes.map(Some));
    }

    // Alt goes as Command whatever key the client pressed for it.
    #[test]
    fn modifiers_follow_their_keysym() {
        assert_eq!(keycode(0xffe9, Some(0xe2)), Some(0x37));
        assert_eq!(keycode(b'f'.into(), Some(0x09)), Some(0x03));
        assert_eq!(keycode(b'f'.into(), None), None);
    }

    #[test]
    fn keys_without_a_mac_code_have_none() {
        assert_eq!(mac_keycode(0x46), None);
        assert_eq!(mac_keycode(0xff), None);
    }
}
