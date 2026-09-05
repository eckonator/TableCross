//! macOS virtual key codes (`kVK_…`) ↔ USB HID usages (page 7).
//!
//! On ISO keyboards macOS swaps two codes: the key left of "1" reports `kVK_ISO_Section` (0x0A) and
//! the key next to the left Shift reports `kVK_ANSI_Grave` (0x32). The tables are for ANSI; `iso`
//! applies the swap so the HID usage always names the physical position.

/// (macOS key code, HID usage), ANSI keyboard.
const TABLE: &[(u16, u16)] = &[
    (0x00, 0x04), // A
    (0x01, 0x16), // S
    (0x02, 0x07), // D
    (0x03, 0x09), // F
    (0x04, 0x0B), // H
    (0x05, 0x0A), // G
    (0x06, 0x1D), // Z
    (0x07, 0x1B), // X
    (0x08, 0x06), // C
    (0x09, 0x19), // V
    (0x0A, 0x64), // ISO section → non-US backslash
    (0x0B, 0x05), // B
    (0x0C, 0x14), // Q
    (0x0D, 0x1A), // W
    (0x0E, 0x08), // E
    (0x0F, 0x15), // R
    (0x10, 0x1C), // Y
    (0x11, 0x17), // T
    (0x12, 0x1E), // 1
    (0x13, 0x1F), // 2
    (0x14, 0x20), // 3
    (0x15, 0x21), // 4
    (0x16, 0x23), // 6
    (0x17, 0x22), // 5
    (0x18, 0x2E), // =
    (0x19, 0x26), // 9
    (0x1A, 0x24), // 7
    (0x1B, 0x2D), // -
    (0x1C, 0x25), // 8
    (0x1D, 0x27), // 0
    (0x1E, 0x30), // ]
    (0x1F, 0x12), // O
    (0x20, 0x18), // U
    (0x21, 0x2F), // [
    (0x22, 0x0C), // I
    (0x23, 0x13), // P
    (0x24, 0x28), // Return
    (0x25, 0x0F), // L
    (0x26, 0x0D), // J
    (0x27, 0x34), // '
    (0x28, 0x0E), // K
    (0x29, 0x33), // ;
    (0x2A, 0x31), // backslash
    (0x2B, 0x36), // ,
    (0x2C, 0x38), // /
    (0x2D, 0x11), // N
    (0x2E, 0x10), // M
    (0x2F, 0x37), // .
    (0x30, 0x2B), // Tab
    (0x31, 0x2C), // Space
    (0x32, 0x35), // `
    (0x33, 0x2A), // Backspace
    (0x35, 0x29), // Escape
    (0x36, 0xE7), // right ⌘
    (0x37, 0xE3), // left ⌘
    (0x38, 0xE1), // left Shift
    (0x39, 0x39), // Caps Lock
    (0x3A, 0xE2), // left Option
    (0x3B, 0xE0), // left Control
    (0x3C, 0xE5), // right Shift
    (0x3D, 0xE6), // right Option
    (0x3E, 0xE4), // right Control
    (0x40, 0x6C), // F17
    (0x41, 0x63), // keypad .
    (0x43, 0x55), // keypad *
    (0x45, 0x57), // keypad +
    (0x47, 0x53), // keypad Clear / Num Lock
    (0x48, 0x80), // volume up
    (0x49, 0x81), // volume down
    (0x4A, 0x7F), // mute
    (0x4B, 0x54), // keypad /
    (0x4C, 0x58), // keypad Enter
    (0x4E, 0x56), // keypad -
    (0x4F, 0x6D), // F18
    (0x50, 0x6E), // F19
    (0x51, 0x67), // keypad =
    (0x52, 0x62), // keypad 0
    (0x53, 0x59), // keypad 1
    (0x54, 0x5A), // keypad 2
    (0x55, 0x5B), // keypad 3
    (0x56, 0x5C), // keypad 4
    (0x57, 0x5D), // keypad 5
    (0x58, 0x5E), // keypad 6
    (0x59, 0x5F), // keypad 7
    (0x5A, 0x6F), // F20
    (0x5B, 0x60), // keypad 8
    (0x5C, 0x61), // keypad 9
    (0x5D, 0x89), // JIS Yen
    (0x5E, 0x87), // JIS underscore
    (0x5F, 0x85), // JIS keypad comma
    (0x60, 0x3E), // F5
    (0x61, 0x3F), // F6
    (0x62, 0x40), // F7
    (0x63, 0x3C), // F3
    (0x64, 0x41), // F8
    (0x65, 0x42), // F9
    (0x66, 0x91), // JIS Eisu
    (0x67, 0x44), // F11
    (0x68, 0x90), // JIS Kana
    (0x69, 0x68), // F13
    (0x6A, 0x6B), // F16
    (0x6B, 0x69), // F14
    (0x6D, 0x43), // F10
    (0x6E, 0x65), // context menu
    (0x6F, 0x45), // F12
    (0x71, 0x6A), // F15
    (0x72, 0x49), // Help, where PC keyboards have Insert
    (0x73, 0x4A), // Home
    (0x74, 0x4B), // Page Up
    (0x75, 0x4C), // forward delete
    (0x76, 0x3D), // F4
    (0x77, 0x4D), // End
    (0x78, 0x3B), // F2
    (0x79, 0x4E), // Page Down
    (0x7A, 0x3A), // F1
    (0x7B, 0x50), // left
    (0x7C, 0x4F), // right
    (0x7D, 0x51), // down
    (0x7E, 0x52), // up
];

/// HID usages without their own Mac key, sent to the key a Mac uses for them: (Mac code, HID usage).
const HID_ONLY: &[(u16, u16)] = &[
    (0x2A, 0x32), // non-US # → backslash key
    (0x69, 0x46), // Print Screen → F13
    (0x6B, 0x47), // Scroll Lock → F14
    (0x71, 0x48), // Pause → F15
];

const ISO_SECTION: u16 = 0x0A;
const ANSI_GRAVE: u16 = 0x32;

pub fn to_hid(code: u16, iso: bool) -> Option<u16> {
    let code = match code {
        ISO_SECTION if iso => ANSI_GRAVE,
        ANSI_GRAVE if iso => ISO_SECTION,
        other => other,
    };
    TABLE.iter().find(|(c, _)| *c == code).map(|(_, u)| *u)
}

pub fn from_hid(usage: u16, iso: bool) -> Option<u16> {
    let code = TABLE
        .iter()
        .chain(HID_ONLY)
        .find(|(_, u)| *u == usage)
        .map(|(c, _)| *c)?;
    Some(match code {
        ISO_SECTION if iso => ANSI_GRAVE,
        ANSI_GRAVE if iso => ISO_SECTION,
        other => other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_no_duplicates() {
        for iso in [false, true] {
            for (code, _) in TABLE {
                let usage = to_hid(*code, iso).unwrap();
                assert!(tc_protocol::is_key_usage(usage), "{usage:#x}");
                assert_eq!(from_hid(usage, iso), Some(*code), "{code:#x} iso={iso}");
            }
        }
        let mut codes: Vec<_> = TABLE.iter().map(|(c, _)| c).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), TABLE.len());
        let mut usages: Vec<_> = TABLE.iter().map(|(_, u)| u).collect();
        usages.sort();
        usages.dedup();
        assert_eq!(usages.len(), TABLE.len());
    }

    #[test]
    fn iso_swap() {
        // The key left of 1 is the grave position on every layout
        assert_eq!(to_hid(0x0A, true), Some(0x35));
        assert_eq!(to_hid(0x32, false), Some(0x35));
        // The key next to left Shift exists only on ISO
        assert_eq!(to_hid(0x32, true), Some(0x64));
        assert_eq!(from_hid(0x64, true), Some(0x32));
        assert_eq!(from_hid(0x35, true), Some(0x0A));
    }

    #[test]
    fn pc_keys_without_mac_equivalent() {
        assert_eq!(from_hid(0x46, false), Some(0x69));
        assert_eq!(from_hid(0x32, true), Some(0x2A));
        assert_eq!(to_hid(0x3F, false), None, "Fn is not forwarded");
    }
}
