use serde::{Deserialize, Serialize};

/// Bumped on every incompatible change; both sides compare it in `Message::Hello`.
pub const PROTOCOL_VERSION: u16 = 2;

/// Largest message in bytes. Larger payloads (clipboard, files) are sent in chunks.
pub const MAX_MESSAGE_LEN: usize = 64 * 1024;

/// One screen of a peer, in the coordinates its system uses for the pointer ("native units"):
/// points on macOS, physical pixels on Windows and X11.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Native units per logical point: 1.0 on macOS (already points), Windows at 150 % = 1.5.
    /// Movement is converted through logical points, so the pointer moves equally fast everywhere.
    pub scale: f32,
}

/// Side of a screen or computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
            Side::Top => Side::Bottom,
            Side::Bottom => Side::Top,
        }
    }

    pub fn is_horizontal(self) -> bool {
        matches!(self, Side::Left | Side::Right)
    }
}

/// "`machine` stands on `side` of `of`"; computers are named by the first 16 hex digits of their
/// fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub machine: String,
    pub side: Side,
    pub of: String,
}

/// Most placements and names in one `Message::Layout`.
pub const MAX_PLACEMENTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// Clipboard formats that travel between systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipFormat {
    Text,
    Html,
    Rtf,
    /// Images, always as PNG.
    Png,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// First message after connecting, sent in both directions.
    Hello {
        version: u16,
        name: String,
        screens: Vec<Screen>,
        /// Version of the program, e.g. "0.1.0", shown to the user.
        app: String,
    },
    /// The pointer enters the receiver at `(x, y)` in the receiver's native units; from now on the
    /// sender controls it.
    Enter {
        x: f32,
        y: f32,
    },
    /// The pointer left the receiver. The receiver releases everything it got from the sender.
    Leave,
    /// New pointer position in the receiver's native units (datagram). Absolute, so a lost datagram
    /// costs nothing and no drift builds up; `seq` lets the receiver drop reordered old positions.
    /// `dx`/`dy` is the same motion in logical points, for programs that read relative motion.
    MouseMove {
        seq: u32,
        x: f32,
        y: f32,
        dx: f32,
        dy: f32,
    },
    MouseButton {
        button: MouseButton,
        down: bool,
    },
    /// Scrolling as the user saw it on the sender: positive `dy` shows more of the top of the content,
    /// positive `dx` more of the right side. `precise`: in logical points (trackpad), otherwise
    /// in detents of a wheel (fractions allowed).
    Wheel {
        dx: f32,
        dy: f32,
        precise: bool,
    },
    /// Key as USB HID usage (page 7), independent of the keyboard layout.
    Key {
        usage: u16,
        down: bool,
    },
    /// Release all keys and mouse buttons (safety net against stuck keys).
    ReleaseAll,
    /// Keep-alive; when it stops arriving the connection counts as lost.
    Heartbeat {
        seq: u32,
    },
    /// Latency probe, sent as a datagram; answered with `Pong` carrying the same `seq`.
    Ping {
        seq: u32,
    },
    Pong {
        seq: u32,
    },
    /// The sender removed the pairing; the receiver forgets the sender as well.
    Unpair,
    /// Someone used the sender's own mouse or keyboard: the sender controls itself again, the receiver
    /// stops sending input to it.
    TakeOver,
    /// The sender's clipboard changed (`size` in bytes over all formats). The content follows on a
    /// stream of its own right away if it is small, else when asked for.
    ClipboardOffer {
        id: u64,
        formats: Vec<ClipFormat>,
        size: u64,
    },
    /// Please send the content of offer `id`.
    ClipboardRequest {
        id: u64,
    },
    /// Files and folders were copied on the sender: `names` of the top level (at most a few, for
    /// display), `count` files in all, `size` bytes in all. They follow on a stream when asked for.
    FilesOffer {
        id: u64,
        names: Vec<String>,
        count: u64,
        size: u64,
    },
    FilesRequest {
        id: u64,
    },
    /// Where all computers stand, so every keyboard reaches every screen. Each computer keeps the
    /// arrangement with the highest `(revision, editor)` and passes it on. `names` maps machine ids
    /// to display names, for computers the receiver is not paired with.
    Layout {
        revision: u64,
        editor: String,
        placements: Vec<Placement>,
        names: Vec<(String, String)>,
    },
}

impl Message {
    /// Messages that may travel as unreliable datagrams. Everything else needs the ordered stream.
    pub fn is_datagram(&self) -> bool {
        matches!(
            self,
            Message::MouseMove { .. } | Message::Wheel { .. } | Message::Ping { .. } | Message::Pong { .. }
        )
    }
}

impl Message {
    /// Numbers must be finite. NaN or infinity would throw off the pointer on the receiving side.
    fn values_finite(&self) -> bool {
        match self {
            Message::Hello { screens, .. } => screens.iter().all(|s| s.scale.is_finite() && s.scale > 0.0),
            Message::Enter { x, y } => x.is_finite() && y.is_finite(),
            Message::MouseMove { x, y, dx, dy, .. } => [x, y, dx, dy].iter().all(|v| v.is_finite()),
            Message::Wheel { dx, dy, .. } => dx.is_finite() && dy.is_finite(),
            _ => true,
        }
    }

    fn lists_bounded(&self) -> bool {
        match self {
            Message::Layout { placements, names, .. } => {
                placements.len() <= MAX_PLACEMENTS && names.len() <= MAX_PLACEMENTS
            }
            _ => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooLong(usize),
    Malformed,
    NotFinite,
    TooMany,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::TooLong(n) => write!(f, "message too long ({n} bytes)"),
            DecodeError::Malformed => write!(f, "malformed message"),
            DecodeError::NotFinite => write!(f, "message contains non-finite numbers"),
            DecodeError::TooMany => write!(f, "message contains too many entries"),
        }
    }
}

impl std::error::Error for DecodeError {}

pub fn encode(message: &Message) -> Vec<u8> {
    postcard::to_allocvec(message).expect("messages always encode")
}

/// Reads exactly one message; trailing bytes are an error.
pub fn decode(bytes: &[u8]) -> Result<Message, DecodeError> {
    if bytes.len() > MAX_MESSAGE_LEN {
        return Err(DecodeError::TooLong(bytes.len()));
    }
    let (message, rest) = postcard::take_from_bytes::<Message>(bytes).map_err(|_| DecodeError::Malformed)?;
    if !rest.is_empty() {
        return Err(DecodeError::Malformed);
    }
    if !message.values_finite() {
        return Err(DecodeError::NotFinite);
    }
    if !message.lists_bounded() {
        return Err(DecodeError::TooMany);
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<Message> {
        vec![
            Message::Hello {
                version: PROTOCOL_VERSION,
                name: "Office Mac".into(),
                screens: vec![Screen {
                    x: 0,
                    y: 0,
                    width: 1512,
                    height: 982,
                    scale: 2.0,
                }],
                app: "0.1.0".into(),
            },
            Message::Enter { x: 10.0, y: 0.25 },
            Message::Leave,
            Message::MouseMove {
                seq: 1,
                x: 100.0,
                y: 200.5,
                dx: -3.5,
                dy: 12.0,
            },
            Message::MouseButton {
                button: MouseButton::Right,
                down: true,
            },
            Message::Wheel {
                dx: 0.0,
                dy: -1.0,
                precise: false,
            },
            Message::Key {
                usage: 0x04,
                down: true,
            },
            Message::ReleaseAll,
            Message::Heartbeat { seq: u32::MAX },
            Message::Ping { seq: 7 },
            Message::Pong { seq: 7 },
            Message::Unpair,
            Message::TakeOver,
            Message::ClipboardOffer {
                id: 9,
                formats: vec![ClipFormat::Text, ClipFormat::Png],
                size: 1234,
            },
            Message::ClipboardRequest { id: 9 },
            Message::FilesOffer {
                id: 3,
                names: vec!["Fotos".into()],
                count: 1200,
                size: 2_000_000_000,
            },
            Message::FilesRequest { id: 3 },
            Message::Layout {
                revision: 1_792_234_331_000,
                editor: "0123456789abcdef".into(),
                placements: vec![Placement {
                    machine: "fedcba9876543210".into(),
                    side: Side::Right,
                    of: "0123456789abcdef".into(),
                }],
                names: vec![("fedcba9876543210".into(), "Office PC".into())],
            },
        ]
    }

    #[test]
    fn every_message_survives_the_round_trip() {
        for m in samples() {
            assert_eq!(decode(&encode(&m)), Ok(m));
        }
    }

    #[test]
    fn input_messages_stay_small() {
        // Mouse motion is sent as individual datagrams
        let motion = Message::MouseMove {
            seq: u32::MAX,
            x: 1.0,
            y: 1.0,
            dx: 1.0,
            dy: 1.0,
        };
        assert!(encode(&motion).len() <= 22);
        assert!(
            encode(&Message::Key {
                usage: 0xE7,
                down: false
            })
            .len()
                <= 4
        );
    }

    #[test]
    fn non_finite_numbers_are_rejected() {
        for m in [
            Message::MouseMove {
                seq: 0,
                x: 0.0,
                y: f32::NAN,
                dx: 0.0,
                dy: 0.0,
            },
            Message::Wheel {
                dx: 0.0,
                dy: f32::INFINITY,
                precise: true,
            },
            Message::Enter { x: f32::NAN, y: 0.0 },
            Message::Hello {
                version: 1,
                name: String::new(),
                screens: vec![Screen {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    scale: 0.0,
                }],
                app: String::new(),
            },
        ] {
            assert_eq!(decode(&encode(&m)), Err(DecodeError::NotFinite), "{m:?}");
        }
    }

    #[test]
    fn long_layouts_are_rejected() {
        let placement = Placement {
            machine: "a".into(),
            side: Side::Left,
            of: "b".into(),
        };
        let m = Message::Layout {
            revision: 1,
            editor: String::new(),
            placements: vec![placement; MAX_PLACEMENTS + 1],
            names: Vec::new(),
        };
        assert_eq!(decode(&encode(&m)), Err(DecodeError::TooMany));
    }

    #[test]
    fn trailing_bytes_and_oversize_are_rejected() {
        let mut bytes = encode(&Message::Leave);
        bytes.push(0);
        assert_eq!(decode(&bytes), Err(DecodeError::Malformed));
        assert_eq!(
            decode(&vec![0; MAX_MESSAGE_LEN + 1]),
            Err(DecodeError::TooLong(MAX_MESSAGE_LEN + 1))
        );
    }

    #[test]
    fn random_bytes_never_panic() {
        // Simple xorshift generator so the test needs no extra dependency
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let len = (next() % 40) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            let _ = decode(&bytes);
        }
        for m in samples() {
            let bytes = encode(&m);
            for cut in 0..bytes.len() {
                let _ = decode(&bytes[..cut]);
            }
        }
    }
}
