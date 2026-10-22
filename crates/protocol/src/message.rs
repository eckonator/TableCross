use serde::{Deserialize, Serialize};

/// Bumped on every incompatible change; both sides compare it in `Message::Hello`.
pub const PROTOCOL_VERSION: u16 = 6;

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

/// Keys for volume, playback and brightness. They travel apart from the other keys because systems
/// report them apart (macOS as system events, not as key codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaKey {
    VolumeUp,
    VolumeDown,
    Mute,
    PlayPause,
    Next,
    Previous,
    BrightnessUp,
    BrightnessDown,
}

/// "`machine` stands on `side` of `of`"; computers are named by the first 16 hex digits of their
/// fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub machine: String,
    pub side: Side,
    pub of: String,
}

/// Where `machine` stands on the desk, in tiles: every computer is one tile of 1 × 1, `(0, 0)` is
/// arbitrary. Computers whose tiles touch are neighbors where the edges overlap; computers used
/// in turn at the same place have the same position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tile {
    pub machine: String,
    pub x: f32,
    pub y: f32,
}

/// Positions of tiles stay within this distance from `(0, 0)`.
pub const MAX_TILE_POS: f32 = 1024.0;

/// Where `machine` stands on the desk and how large it is, in thousands of logical points: an
/// area as large as the screens of the computer (from 0.4.2 on). Areas that touch are neighbors
/// where their edges overlap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub machine: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The size was taken from the computer's screens (else a stand-in until they are known).
    pub measured: bool,
}

/// A part of one side of a computer, `from` to `to` along it (0.0 to 1.0), where the pointer
/// crosses to a neighbor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeMark {
    pub side: Side,
    pub from: f32,
    pub to: f32,
}

/// A keyboard layout (input source): how keys turn into characters on a computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardLayout {
    /// The system's own name for it ("com.apple.keylayout.German", a Windows layout handle);
    /// only meaningful on the same kind of system.
    pub id: String,
    /// Language and maybe region it is for ("de", "de-CH").
    pub language: String,
    /// For people.
    pub name: String,
}

/// Longest text in a `KeyboardLayout`.
pub const MAX_LAYOUT_TEXT: usize = 256;

/// Largest area side, in thousands of logical points.
pub const MAX_AREA_SIZE: f32 = 100.0;

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
    /// A media key was pressed (once per press and per auto-repeat; there is no release).
    Media {
        key: MediaKey,
    },
    /// Release all keys and mouse buttons (safety net against stuck keys).
    ReleaseAll,
    /// Lock the screen (someone locked all computers at once).
    LockScreen,
    /// Shut the computer down (someone shut down all computers at once). Applications may still ask
    /// to save their documents.
    PowerOff,
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
        /// Dragged over the edge (not copied): the receiver shows them when they arrived.
        dropped: bool,
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
    /// The sender collects the logs of its peers for troubleshooting (`on`) or stops doing so.
    LogSubscribe {
        on: bool,
    },
    /// New lines of the sender's log, for a peer that subscribed. At most `MAX_LOG_TEXT` bytes.
    LogLines {
        text: String,
    },
    /// The arrangement as tiles (from version 0.4.0 on; older versions only know `Layout` and end
    /// the connection on this message, so it only goes to peers that announced 0.4.0 or later).
    Tiles {
        revision: u64,
        editor: String,
        tiles: Vec<Tile>,
        names: Vec<(String, String)>,
    },
    /// The arrangement as areas of real size (from 0.4.2 on, only to peers that announced it).
    Desk {
        revision: u64,
        editor: String,
        areas: Vec<Area>,
        names: Vec<(String, String)>,
    },
    /// Show where the pointer crosses on the receiver's screens while the arrangement is being
    /// changed elsewhere (from 0.4.2 on). Empty: hide them. Shown for a moment after the last one.
    ShowEdges {
        marks: Vec<EdgeMark>,
    },
    /// The sender's screens changed (a monitor came or went, or they became known only after the
    /// connection was made). From 0.4.7 on, only to peers that announced it.
    Screens {
        screens: Vec<Screen>,
    },
    /// Physical size of the sender's screens in millimeters, in the order of its screens; `(0, 0)`
    /// where unknown. Sent with every change of the screens, from 0.4.10 on only to peers that
    /// announced it.
    ScreenSizes {
        mm: Vec<(u32, u32)>,
    },
    /// The keyboard layout in use on the computer that controls the receiver, sent when the
    /// pointer arrives and when the layout changes. From 0.4.36 on, only to peers that announced it.
    KeyboardLayout {
        layout: KeyboardLayout,
    },
}

/// Longest text in one `Message::LogLines`.
pub const MAX_LOG_TEXT: usize = 32 * 1024;

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
            Message::Hello { screens, .. } | Message::Screens { screens } => {
                screens.iter().all(|s| s.scale.is_finite() && s.scale > 0.0)
            }
            Message::Enter { x, y } => x.is_finite() && y.is_finite(),
            Message::MouseMove { x, y, dx, dy, .. } => [x, y, dx, dy].iter().all(|v| v.is_finite()),
            Message::Wheel { dx, dy, .. } => dx.is_finite() && dy.is_finite(),
            Message::Desk { areas, .. } => areas.iter().all(|a| {
                [a.x, a.y].iter().all(|v| v.is_finite() && v.abs() <= MAX_TILE_POS)
                    && [a.w, a.h]
                        .iter()
                        .all(|v| v.is_finite() && *v > 0.0 && *v <= MAX_AREA_SIZE)
            }),
            Message::ShowEdges { marks } => marks
                .iter()
                .all(|m| [m.from, m.to].iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v))),
            Message::Tiles { tiles, .. } => tiles
                .iter()
                .all(|t| [t.x, t.y].iter().all(|v| v.is_finite() && v.abs() <= MAX_TILE_POS)),
            _ => true,
        }
    }

    fn lists_bounded(&self) -> bool {
        match self {
            Message::Layout { placements, names, .. } => {
                placements.len() <= MAX_PLACEMENTS && names.len() <= MAX_PLACEMENTS
            }
            Message::Tiles { tiles, names, .. } => tiles.len() <= MAX_PLACEMENTS && names.len() <= MAX_PLACEMENTS,
            Message::Desk { areas, names, .. } => areas.len() <= MAX_PLACEMENTS && names.len() <= MAX_PLACEMENTS,
            Message::ShowEdges { marks } => marks.len() <= MAX_PLACEMENTS,
            Message::Screens { screens } => screens.len() <= MAX_PLACEMENTS,
            Message::ScreenSizes { mm } => mm.len() <= MAX_PLACEMENTS,
            Message::LogLines { text } => text.len() <= MAX_LOG_TEXT,
            Message::KeyboardLayout { layout } => [&layout.id, &layout.language, &layout.name]
                .iter()
                .all(|t| t.len() <= MAX_LAYOUT_TEXT),
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
            Message::Media {
                key: MediaKey::VolumeUp,
            },
            Message::ReleaseAll,
            Message::LockScreen,
            Message::PowerOff,
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
            Message::LogSubscribe { on: true },
            Message::LogLines {
                text: "2026-10-19T07:55:13Z INFO input is local\n".into(),
            },
            Message::FilesOffer {
                id: 3,
                names: vec!["Fotos".into()],
                count: 1200,
                size: 2_000_000_000,
                dropped: false,
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
            Message::Tiles {
                revision: 1_792_234_331_001,
                editor: "0123456789abcdef".into(),
                tiles: vec![
                    Tile {
                        machine: "0123456789abcdef".into(),
                        x: 0.0,
                        y: 0.0,
                    },
                    Tile {
                        machine: "fedcba9876543210".into(),
                        x: 0.5,
                        y: 1.0,
                    },
                ],
                names: vec![("fedcba9876543210".into(), "Office PC".into())],
            },
            Message::Desk {
                revision: 1_792_234_331_002,
                editor: "0123456789abcdef".into(),
                areas: vec![Area {
                    machine: "0123456789abcdef".into(),
                    x: -0.864,
                    y: 1.152,
                    w: 1.728,
                    h: 1.117,
                    measured: true,
                }],
                names: vec![("0123456789abcdef".into(), "MacBook".into())],
            },
            Message::ShowEdges {
                marks: vec![EdgeMark {
                    side: Side::Top,
                    from: 0.5,
                    to: 1.0,
                }],
            },
            Message::Screens {
                screens: vec![Screen {
                    x: 0,
                    y: 0,
                    width: 5120,
                    height: 1440,
                    scale: 1.0,
                }],
            },
            Message::ScreenSizes {
                mm: vec![(1196, 336), (0, 0)],
            },
            Message::KeyboardLayout {
                layout: KeyboardLayout {
                    id: "com.apple.keylayout.German".into(),
                    language: "de".into(),
                    name: "Deutsch".into(),
                },
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
    fn tiles_far_away_or_not_finite_are_rejected() {
        for x in [f32::NAN, f32::INFINITY, MAX_TILE_POS * 2.0] {
            let m = Message::Tiles {
                revision: 1,
                editor: "a".into(),
                tiles: vec![Tile {
                    machine: "a".into(),
                    x,
                    y: 0.0,
                }],
                names: Vec::new(),
            };
            assert_eq!(decode(&encode(&m)), Err(DecodeError::NotFinite), "{x}");
        }
    }

    #[test]
    fn areas_without_size_or_far_away_are_rejected() {
        let area = |x: f32, w: f32| Message::Desk {
            revision: 1,
            editor: "a".into(),
            areas: vec![Area {
                machine: "a".into(),
                x,
                y: 0.0,
                w,
                h: 1.0,
                measured: false,
            }],
            names: Vec::new(),
        };
        for (x, w) in [
            (0.0, 0.0),
            (0.0, -1.0),
            (0.0, f32::NAN),
            (MAX_TILE_POS * 2.0, 1.0),
            (0.0, 1000.0),
        ] {
            assert_eq!(decode(&encode(&area(x, w))), Err(DecodeError::NotFinite), "{x} {w}");
        }
        assert!(decode(&encode(&area(1.5, 5.12))).is_ok());
        let mark = Message::ShowEdges {
            marks: vec![EdgeMark {
                side: Side::Left,
                from: 0.0,
                to: 1.5,
            }],
        };
        assert_eq!(decode(&encode(&mark)), Err(DecodeError::NotFinite));
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
