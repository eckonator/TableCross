/// Real keys on USB HID page 7 (A up to right ⌘/Win). Below are error codes, above is nothing
/// a keyboard reports.
pub fn is_key_usage(usage: u16) -> bool {
    (0x04..=0xE7).contains(&usage)
}

/// Tracks which keys are currently held and smooths out inconsistencies of the source:
///
/// - "up" without a preceding "down" is dropped. This happens on macOS when Secure Input ends
///   while keys are held.
/// - "down" for a key that is already held is auto-repeat and passes through.
/// - `release_all` returns everything still held, so the receiver never keeps a key pressed after a
///   disconnect or when the pointer leaves its screen.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PressedKeys {
    down: [u64; 4],
}

impl PressedKeys {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` if the event should be forwarded.
    pub fn press(&mut self, usage: u16) -> bool {
        if !is_key_usage(usage) {
            return false;
        }
        let (word, bit) = Self::slot(usage);
        self.down[word] |= bit;
        true
    }

    /// `true` if the key was held and the release should be forwarded.
    pub fn release(&mut self, usage: u16) -> bool {
        if !self.is_down(usage) {
            return false;
        }
        let (word, bit) = Self::slot(usage);
        self.down[word] &= !bit;
        true
    }

    pub fn is_down(&self, usage: u16) -> bool {
        if !is_key_usage(usage) {
            return false;
        }
        let (word, bit) = Self::slot(usage);
        self.down[word] & bit != 0
    }

    pub fn is_empty(&self) -> bool {
        self.down.iter().all(|w| *w == 0)
    }

    /// Releases everything and returns the previously held keys in ascending order.
    pub fn release_all(&mut self) -> Vec<u16> {
        let pressed = (0x04..=0xE7).filter(|u| self.is_down(*u)).collect();
        self.down = [0; 4];
        pressed
    }

    fn slot(usage: u16) -> (usize, u64) {
        (usize::from(usage / 64), 1u64 << (usage % 64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u16 = 0x04;
    const LEFT_SHIFT: u16 = 0xE1;
    const RIGHT_GUI: u16 = 0xE7;

    #[test]
    fn release_without_press_is_dropped() {
        let mut keys = PressedKeys::new();
        assert!(!keys.release(A));
        assert!(keys.press(A));
        assert!(keys.release(A));
        assert!(!keys.release(A), "a second release must not pass");
    }

    #[test]
    fn repeat_passes_through() {
        let mut keys = PressedKeys::new();
        assert!(keys.press(A));
        assert!(keys.press(A));
        assert!(keys.release(A));
        assert!(keys.is_empty());
    }

    #[test]
    fn release_all_returns_everything_once() {
        let mut keys = PressedKeys::new();
        for u in [RIGHT_GUI, A, LEFT_SHIFT] {
            keys.press(u);
        }
        assert_eq!(keys.release_all(), vec![A, LEFT_SHIFT, RIGHT_GUI]);
        assert!(keys.is_empty());
        assert!(keys.release_all().is_empty());
    }

    #[test]
    fn invalid_usages_are_ignored() {
        let mut keys = PressedKeys::new();
        for u in [0, 1, 3, 0xE8, 0xFF, 0x100, u16::MAX] {
            assert!(!keys.press(u), "{u:#x}");
            assert!(!keys.release(u));
        }
        assert!(keys.is_empty());
    }
}
