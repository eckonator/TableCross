//! Conversation between TableCross and its macOS system helper over a Unix socket.
//!
//! The helper runs as root and can read the keyboard below the window server, also while another
//! program holds Secure Input. It takes the keyboards away from the system only while the app asks
//! for it, and gives them back as soon as the app stops sending `Alive` for `DEADLINE`, disconnects
//! or crashes. Plain types and framing, so the rules are tested on every system.

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Changed on incompatible changes. The app also works with a helper down to `OLDEST_HELPER`
/// (without what came later); an older helper is offered for update.
pub const HELPER_VERSION: u16 = 5;
pub const OLDEST_HELPER: u16 = 3;
/// The first helper that pauses AWDL (`ToHelper::QuietRadio`).
pub const QUIET_RADIO_SINCE: u16 = 4;
/// The first helper that sets up control on the login screen (`ToHelper::LoginScreen`).
pub const LOGIN_SCREEN_SINCE: u16 = 5;
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

/// Media keys on the HID consumer page (0x0C).
pub const CONSUMER_KEYS: [(u32, tc_protocol::MediaKey); 8] = [
    (0xE9, tc_protocol::MediaKey::VolumeUp),
    (0xEA, tc_protocol::MediaKey::VolumeDown),
    (0xE2, tc_protocol::MediaKey::Mute),
    (0xCD, tc_protocol::MediaKey::PlayPause),
    (0xB5, tc_protocol::MediaKey::Next),
    (0xB6, tc_protocol::MediaKey::Previous),
    (0x6F, tc_protocol::MediaKey::BrightnessUp),
    (0x70, tc_protocol::MediaKey::BrightnessDown),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToHelper {
    Hello {
        version: u16,
    },
    /// Take the keyboards away from the system and send their keys (`true`), or give them back.
    Seize(bool),
    Alive,
    /// Pause AWDL, the radio link of AirDrop, Handoff and AirPlay, while the pointer is shared
    /// (`true`), or let it run again. It leaves the Wi-Fi channel every few seconds, and packets
    /// wait meanwhile. Only from helper version `QUIET_RADIO_SINCE`.
    QuietRadio(bool),
    /// Let this Mac be controlled on the login screen (`Some`: as this name on this port), or no
    /// longer. The helper copies the identity and pairings of the connected user for it, also
    /// again later when they changed. Only from helper version `LOGIN_SCREEN_SINCE`.
    LoginScreen(Option<LoginAgent>),
}

/// What the agent on the login screen needs besides the copied files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginAgent {
    pub name: String,
    pub port: u16,
}

/// The program that runs on the login screen (a copy of the command line tool, put there with
/// the helper by the administrator).
pub const LOGIN_BINARY: &str = "/Library/PrivilegedHelperTools/tablecross-loginwindow";
pub const LOGIN_LABEL: &str = "de.eckonator.tablecross.loginwindow";
pub const LOGIN_PLIST: &str = "/Library/LaunchAgents/de.eckonator.tablecross.loginwindow.plist";
/// Settings of the agent: identity and pairings copied from the user, `LOGIN_AGENT_FILE`.
pub const LOGIN_DIR: &str = "/Library/Application Support/TableCross/LoginWindow";
pub const LOGIN_AGENT_FILE: &str = "agent.json";
/// The files copied from the user's settings folder.
pub const LOGIN_FILES: [&str; 4] = ["device.key", "device.crt", "peers.json", "layout.json"];
pub const LOGIN_LOG: &str = "/Library/Logs/TableCross/loginwindow.log";
/// Log of the system helper itself.
pub const HELPER_LOG: &str = "/Library/Logs/TableCross/helper.log";

/// The launch agent that starts `LOGIN_BINARY` on the login screen only (macOS ends it when
/// someone logs in).
pub fn login_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LOGIN_LABEL}</string>
    <key>LimitLoadToSessionType</key>
    <string>LoginWindow</string>
    <key>ProgramArguments</key>
    <array>
        <string>{LOGIN_BINARY}</string>
        <string>--config-dir</string>
        <string>{LOGIN_DIR}</string>
        <string>-v</string>
        <string>login-agent</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ThrottleInterval</key>
    <integer>10</integer>
    <key>StandardErrorPath</key>
    <string>{LOGIN_LOG}</string>
</dict>
</plist>
"#
    )
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
    /// A media key of a taken keyboard (consumer page).
    Media { key: tc_protocol::MediaKey, down: bool },
    /// F1–F12 of an Apple keyboard. Taken away from the system it reports the top row as plain
    /// function keys; whether a key means volume, brightness … is decided by `fn_held` and the
    /// user's setting, see [`top_row`].
    FunctionKey { usage: u16, down: bool, fn_held: bool },
    /// Current state after every change.
    Status {
        seized: bool,
        /// Keyboards taken away from the system.
        devices: u16,
        /// Keyboards that could not be taken (their keys also reach the system).
        failed: u16,
    },
}

/// F1 … F12 (USB HID usages).
pub const F1: u16 = 0x3A;
pub const F12: u16 = 0x45;

/// What the top row of Apple keyboards does without fn (F1 … F12); `None` keeps the function key.
const TOP_ROW: [Option<tc_protocol::MediaKey>; 12] = {
    use tc_protocol::MediaKey::*;
    [
        Some(BrightnessDown),
        Some(BrightnessUp),
        None,
        None,
        None,
        None,
        Some(Previous),
        Some(PlayPause),
        Some(Next),
        Some(Mute),
        Some(VolumeDown),
        Some(VolumeUp),
    ]
};

/// The media key meant by F1 … F12 of an Apple keyboard, or `None` for the plain function key.
/// `fn_keys_standard` is the setting "Use F1, F2, etc. keys as standard function keys": then fn
/// selects the media key, otherwise it selects the function key.
pub fn top_row(usage: u16, fn_held: bool, fn_keys_standard: bool) -> Option<tc_protocol::MediaKey> {
    if !(F1..=F12).contains(&usage) || fn_held != fn_keys_standard {
        return None;
    }
    TOP_ROW[usize::from(usage - F1)]
}

/// AWDL's interface on macOS.
pub const AWDL: &str = "awdl0";

/// Whether `ifconfig <interface>` reports the interface as up ("flags=8843<UP,BROADCAST,…>");
/// `None` when the output says nothing about it.
pub fn interface_up(ifconfig: &str) -> Option<bool> {
    let flags = ifconfig.lines().next()?.split_once("flags=")?.1;
    let list = flags.split_once('<')?.1.split_once('>')?.0;
    Some(list.split(',').any(|f| f == "UP"))
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
    fn apple_top_row_follows_fn_and_the_setting() {
        use tc_protocol::MediaKey;
        const F11: u16 = 0x44;
        assert_eq!(top_row(F12, false, false), Some(MediaKey::VolumeUp));
        assert_eq!(top_row(F11, false, false), Some(MediaKey::VolumeDown));
        assert_eq!(top_row(F1, false, false), Some(MediaKey::BrightnessDown));
        assert_eq!(top_row(F11, true, false), None);
        assert_eq!(top_row(F11, false, true), None);
        assert_eq!(top_row(F11, true, true), Some(MediaKey::VolumeDown));
        // Mission Control and friends stay function keys
        assert_eq!(top_row(F1 + 2, false, false), None);
        assert_eq!(top_row(0x04, false, false), None);
    }

    #[test]
    fn the_login_agent_runs_only_on_the_login_screen() {
        let plist = login_plist();
        assert!(plist.contains("<string>LoginWindow</string>"));
        assert!(plist.contains(LOGIN_BINARY) && plist.contains(LOGIN_DIR));
        let mut buf = Vec::new();
        let message = ToHelper::LoginScreen(Some(LoginAgent {
            name: "MacBook".into(),
            port: 24800,
        }));
        write_frame(&mut buf, &message).unwrap();
        assert_eq!(read_frame::<ToHelper>(&mut io::Cursor::new(buf)).unwrap(), message);
    }

    #[test]
    fn reads_whether_an_interface_is_up() {
        let up =
            "awdl0: flags=8943<UP,BROADCAST,RUNNING,PROMISC,SIMPLEX,MULTICAST> mtu 1500\n\toptions=400<CHANNEL_IO>";
        let down = "awdl0: flags=8902<BROADCAST,PROMISC,SIMPLEX,MULTICAST> mtu 1500";
        assert_eq!(interface_up(up), Some(true));
        assert_eq!(interface_up(down), Some(false));
        assert_eq!(interface_up("ifconfig: interface awdl0 does not exist"), None);
        assert_eq!(interface_up(""), None);
        // "UP" only as a whole flag
        assert_eq!(interface_up("x: flags=1<LOWER_UP,POINTOPOINT> mtu 1"), Some(false));
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
