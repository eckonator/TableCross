//! Linux key codes (evdev, `KEY_…` in input-event-codes.h) ↔ USB HID usages (page 7).
//! X11 key codes are these plus 8; libei uses them as they are.

/// (evdev code, HID usage)
const TABLE: &[(u16, u16)] = &[
    (1, 0x29),   // Esc
    (2, 0x1E),   // 1
    (3, 0x1F),   // 2
    (4, 0x20),   // 3
    (5, 0x21),   // 4
    (6, 0x22),   // 5
    (7, 0x23),   // 6
    (8, 0x24),   // 7
    (9, 0x25),   // 8
    (10, 0x26),  // 9
    (11, 0x27),  // 0
    (12, 0x2D),  // -
    (13, 0x2E),  // =
    (14, 0x2A),  // Backspace
    (15, 0x2B),  // Tab
    (16, 0x14),  // Q
    (17, 0x1A),  // W
    (18, 0x08),  // E
    (19, 0x15),  // R
    (20, 0x17),  // T
    (21, 0x1C),  // Y
    (22, 0x18),  // U
    (23, 0x0C),  // I
    (24, 0x12),  // O
    (25, 0x13),  // P
    (26, 0x2F),  // [
    (27, 0x30),  // ]
    (28, 0x28),  // Enter
    (29, 0xE0),  // left Ctrl
    (30, 0x04),  // A
    (31, 0x16),  // S
    (32, 0x07),  // D
    (33, 0x09),  // F
    (34, 0x0A),  // G
    (35, 0x0B),  // H
    (36, 0x0D),  // J
    (37, 0x0E),  // K
    (38, 0x0F),  // L
    (39, 0x33),  // ;
    (40, 0x34),  // '
    (41, 0x35),  // `
    (42, 0xE1),  // left Shift
    (43, 0x31),  // backslash (also non-US #)
    (44, 0x1D),  // Z
    (45, 0x1B),  // X
    (46, 0x06),  // C
    (47, 0x19),  // V
    (48, 0x05),  // B
    (49, 0x11),  // N
    (50, 0x10),  // M
    (51, 0x36),  // ,
    (52, 0x37),  // .
    (53, 0x38),  // /
    (54, 0xE5),  // right Shift
    (55, 0x55),  // keypad *
    (56, 0xE2),  // left Alt
    (57, 0x2C),  // Space
    (58, 0x39),  // Caps Lock
    (59, 0x3A),  // F1
    (60, 0x3B),  // F2
    (61, 0x3C),  // F3
    (62, 0x3D),  // F4
    (63, 0x3E),  // F5
    (64, 0x3F),  // F6
    (65, 0x40),  // F7
    (66, 0x41),  // F8
    (67, 0x42),  // F9
    (68, 0x43),  // F10
    (69, 0x53),  // Num Lock
    (70, 0x47),  // Scroll Lock
    (71, 0x5F),  // keypad 7
    (72, 0x60),  // keypad 8
    (73, 0x61),  // keypad 9
    (74, 0x56),  // keypad -
    (75, 0x5C),  // keypad 4
    (76, 0x5D),  // keypad 5
    (77, 0x5E),  // keypad 6
    (78, 0x57),  // keypad +
    (79, 0x59),  // keypad 1
    (80, 0x5A),  // keypad 2
    (81, 0x5B),  // keypad 3
    (82, 0x62),  // keypad 0
    (83, 0x63),  // keypad .
    (85, 0x94),  // Zenkaku/Hankaku (LANG5)
    (86, 0x64),  // non-US backslash (key next to left Shift)
    (87, 0x44),  // F11
    (88, 0x45),  // F12
    (89, 0x87),  // Ro
    (90, 0x92),  // Katakana (LANG3)
    (91, 0x93),  // Hiragana (LANG4)
    (92, 0x8A),  // Henkan
    (93, 0x88),  // Katakana/Hiragana
    (94, 0x8B),  // Muhenkan
    (95, 0x85),  // keypad , (JIS)
    (96, 0x58),  // keypad Enter
    (97, 0xE4),  // right Ctrl
    (98, 0x54),  // keypad /
    (99, 0x46),  // Print Screen
    (100, 0xE6), // right Alt / AltGr
    (102, 0x4A), // Home
    (103, 0x52), // up
    (104, 0x4B), // Page Up
    (105, 0x50), // left
    (106, 0x4F), // right
    (107, 0x4D), // End
    (108, 0x51), // down
    (109, 0x4E), // Page Down
    (110, 0x49), // Insert
    (111, 0x4C), // Delete
    (113, 0x7F), // mute
    (114, 0x81), // volume down
    (115, 0x80), // volume up
    (116, 0x66), // power
    (117, 0x67), // keypad =
    (119, 0x48), // Pause
    (122, 0x90), // Hangul (LANG1)
    (123, 0x91), // Hanja (LANG2)
    (124, 0x89), // Yen
    (125, 0xE3), // left Super
    (126, 0xE7), // right Super
    (127, 0x65), // context menu (Compose)
    (138, 0x75), // Help
    (183, 0x68), // F13
    (184, 0x69), // F14
    (185, 0x6A), // F15
    (186, 0x6B), // F16
    (187, 0x6C), // F17
    (188, 0x6D), // F18
    (189, 0x6E), // F19
    (190, 0x6F), // F20
    (191, 0x70), // F21
    (192, 0x71), // F22
    (193, 0x72), // F23
    (194, 0x73), // F24
];

pub fn to_hid(code: u16) -> Option<u16> {
    TABLE
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, u)| *u)
        .filter(|u| tc_protocol::is_key_usage(*u))
}

pub fn from_hid(usage: u16) -> Option<u16> {
    // Non-US # has no own evdev code: it is the backslash key
    let usage = if usage == 0x32 { 0x31 } else { usage };
    TABLE.iter().find(|(_, u)| *u == usage).map(|(c, _)| *c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_no_duplicates() {
        for (code, usage) in TABLE {
            if !tc_protocol::is_key_usage(*usage) {
                continue;
            }
            assert_eq!(to_hid(*code), Some(*usage));
            assert_eq!(from_hid(*usage), Some(*code), "{usage:#x}");
        }
        let mut usages: Vec<_> = TABLE.iter().map(|(_, u)| u).collect();
        usages.sort();
        usages.dedup();
        assert_eq!(usages.len(), TABLE.len());
    }

    #[test]
    fn every_key_of_a_mac_or_pc_keyboard_exists() {
        for usage in (0x04..=0x67).chain(0xE0..=0xE7) {
            assert!(from_hid(usage).is_some(), "{usage:#x}");
        }
        assert_eq!(from_hid(0x32), Some(43));
        assert_eq!(to_hid(121), None);
    }
}
