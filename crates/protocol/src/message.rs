use serde::{Deserialize, Serialize};

/// Bumped on every incompatible change; both sides compare it in `Message::Hello`.
pub const PROTOCOL_VERSION: u16 = 1;

/// Largest message in bytes. Larger payloads (clipboard, files) are sent in chunks.
pub const MAX_MESSAGE_LEN: usize = 64 * 1024;

/// One screen of a peer, in points (logical pixels) of its system.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Pixels per point (Retina 2.0, Windows 150 % = 1.5)
    pub scale: f32,
}

/// Screen edge the pointer crosses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// First message after connecting, sent in both directions.
    Hello {
        version: u16,
        name: String,
        screens: Vec<Screen>,
    },
    /// The pointer enters the receiver at `edge`; `along` is the position along that edge (0 to 1).
    Enter {
        edge: Edge,
        along: f32,
    },
    /// The pointer leaves the receiver again. The receiver releases all keys.
    Leave,
    /// Relative motion in the sender's points.
    MouseMove {
        dx: f32,
        dy: f32,
    },
    MouseButton {
        button: MouseButton,
        down: bool,
    },
    /// Scrolling in points (trackpad) or detents × 1.0.
    Wheel {
        dx: f32,
        dy: f32,
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
            Message::Enter { along, .. } => along.is_finite(),
            Message::MouseMove { dx, dy } | Message::Wheel { dx, dy } => dx.is_finite() && dy.is_finite(),
            _ => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooLong(usize),
    Malformed,
    NotFinite,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::TooLong(n) => write!(f, "message too long ({n} bytes)"),
            DecodeError::Malformed => write!(f, "malformed message"),
            DecodeError::NotFinite => write!(f, "message contains non-finite numbers"),
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
            },
            Message::Enter {
                edge: Edge::Left,
                along: 0.25,
            },
            Message::Leave,
            Message::MouseMove { dx: -3.5, dy: 12.0 },
            Message::MouseButton {
                button: MouseButton::Right,
                down: true,
            },
            Message::Wheel { dx: 0.0, dy: -1.0 },
            Message::Key {
                usage: 0x04,
                down: true,
            },
            Message::ReleaseAll,
            Message::Heartbeat { seq: u32::MAX },
            Message::Ping { seq: 7 },
            Message::Pong { seq: 7 },
            Message::Unpair,
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
        // Mouse motion will be sent as individual datagrams
        assert!(encode(&Message::MouseMove { dx: 1.0, dy: 1.0 }).len() <= 9);
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
            Message::MouseMove { dx: f32::NAN, dy: 0.0 },
            Message::Wheel {
                dx: 0.0,
                dy: f32::INFINITY,
            },
            Message::Enter {
                edge: Edge::Top,
                along: f32::NAN,
            },
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
            },
        ] {
            assert_eq!(decode(&encode(&m)), Err(DecodeError::NotFinite), "{m:?}");
        }
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
