//! Conversation between TableCross and its macOS system helper over a Unix socket.
//!
//! The helper runs as root and can read the keyboard below the window server, also while another
//! program holds Secure Input. It takes the keyboards away from the system only while the app asks
//! for it, and gives them back as soon as the app stops sending `Alive` for `DEADLINE`, disconnects
//! or crashes. Plain types and framing, so the rules are tested on every system.

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Changed on incompatible changes; app and helper must match.
pub const HELPER_VERSION: u16 = 1;
pub const SOCKET_PATH: &str = "/var/run/de.eckonator.tablecross.helper.sock";

/// `TC_HELPER_SOCKET` overrides the path (tests).
pub fn socket_path() -> String {
    std::env::var("TC_HELPER_SOCKET").unwrap_or_else(|_| SOCKET_PATH.to_string())
}
pub const LABEL: &str = "de.eckonator.tablecross.helper";
/// The app sends `Alive` at least this often while keyboards are taken.
pub const ALIVE_INTERVAL: Duration = Duration::from_millis(250);
/// Without `Alive` for this long the helper gives the keyboards back.
pub const DEADLINE: Duration = Duration::from_secs(1);
const MAX_FRAME: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToHelper {
    Hello {
        version: u16,
    },
    /// Take the keyboards away from the system and send their keys (`true`), or give them back.
    Seize(bool),
    Alive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FromHelper {
    Hello {
        version: u16,
        /// The helper has the Input Monitoring permission.
        access: bool,
    },
    /// A key of a taken keyboard, as USB HID usage (page 7).
    Key { usage: u16, down: bool },
    /// Current state after every change.
    Status {
        seized: bool,
        /// Keyboards taken away from the system.
        devices: u16,
        /// Keyboards that could not be taken (their keys also reach the system).
        failed: u16,
    },
}

/// Length (u16 little endian) and postcard bytes.
pub fn write_frame<T: Serialize>(out: &mut impl Write, message: &T) -> io::Result<()> {
    let bytes = postcard::to_allocvec(message).map_err(io::Error::other)?;
    let len = u16::try_from(bytes.len()).map_err(|_| io::Error::other("message too long"))?;
    let mut frame = Vec::with_capacity(bytes.len() + 2);
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&bytes);
    out.write_all(&frame)
}

pub fn read_frame<T: for<'de> Deserialize<'de>>(input: &mut impl Read) -> io::Result<T> {
    let mut len = [0; 2];
    input.read_exact(&mut len)?;
    let len = usize::from(u16::from_le_bytes(len));
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too long"));
    }
    let mut bytes = vec![0; len];
    input.read_exact(&mut bytes)?;
    match postcard::take_from_bytes(&bytes) {
        Ok((message, [])) => Ok(message),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "malformed message")),
    }
}

/// The helper's rule for when keyboards are taken: only while the app wants it and is alive.
#[derive(Debug)]
pub struct DeadManSwitch {
    wanted: bool,
    last_alive: Instant,
}

impl DeadManSwitch {
    pub fn new(now: Instant) -> Self {
        Self {
            wanted: false,
            last_alive: now,
        }
    }

    pub fn message(&mut self, message: &ToHelper, now: Instant) {
        self.last_alive = now;
        if let ToHelper::Seize(wanted) = message {
            self.wanted = *wanted;
        }
    }

    /// Whether the keyboards should be taken right now.
    pub fn should_seize(&self, now: Instant) -> bool {
        self.wanted && now.duration_since(self.last_alive) < DEADLINE
    }

    /// The app is gone.
    pub fn disconnected(&mut self) {
        self.wanted = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut buf = Vec::new();
        write_frame(&mut buf, &ToHelper::Seize(true)).unwrap();
        write_frame(&mut buf, &FromHelper::Key { usage: 4, down: true }).unwrap();
        let mut cursor = io::Cursor::new(buf);
        assert_eq!(read_frame::<ToHelper>(&mut cursor).unwrap(), ToHelper::Seize(true));
        assert_eq!(
            read_frame::<FromHelper>(&mut cursor).unwrap(),
            FromHelper::Key { usage: 4, down: true }
        );
        assert!(read_frame::<ToHelper>(&mut cursor).is_err());
    }

    #[test]
    fn garbage_is_rejected() {
        let mut too_long = io::Cursor::new(vec![0xFF, 0xFF, 0]);
        assert!(read_frame::<ToHelper>(&mut too_long).is_err());
        let mut nonsense = io::Cursor::new(vec![3, 0, 9, 9, 9]);
        assert!(read_frame::<ToHelper>(&mut nonsense).is_err());
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        for _ in 0..5000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let bytes = state.to_le_bytes();
            let _ = read_frame::<ToHelper>(&mut io::Cursor::new(&bytes[..(state % 9) as usize]));
        }
    }

    #[test]
    fn keyboards_come_back_without_sign_of_life() {
        let t0 = Instant::now();
        let mut switch = DeadManSwitch::new(t0);
        assert!(!switch.should_seize(t0));
        switch.message(&ToHelper::Seize(true), t0);
        assert!(switch.should_seize(t0 + Duration::from_millis(900)));
        // Silent for a second: give back
        assert!(!switch.should_seize(t0 + DEADLINE));
        // Alive again: take again, the wish is still there
        switch.message(&ToHelper::Alive, t0 + Duration::from_secs(2));
        assert!(switch.should_seize(t0 + Duration::from_secs(2)));
        switch.message(&ToHelper::Seize(false), t0 + Duration::from_secs(2));
        assert!(!switch.should_seize(t0 + Duration::from_secs(2)));
        switch.message(&ToHelper::Seize(true), t0 + Duration::from_secs(3));
        switch.disconnected();
        assert!(!switch.should_seize(t0 + Duration::from_secs(3)));
    }
}
