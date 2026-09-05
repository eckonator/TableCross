//! Framing on QUIC streams, the version preamble and connection close codes.

use std::time::Duration;

use quinn::{RecvStream, SendStream, VarInt};
use tc_protocol::{MAX_MESSAGE_LEN, PROTOCOL_VERSION};

/// Waiting longer than this for the other side during setup counts as failure.
pub(crate) const SETUP_TIMEOUT: Duration = Duration::from_secs(5);

const MAGIC: [u8; 4] = *b"TBLX";
const PREAMBLE_LEN: usize = 14;

/// First bytes on every control stream. Independent of the message format, so even a peer with a
/// completely different protocol version can be told what is wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Preamble {
    pub version: u16,
    /// Random per process start. A new connection with a different instance means the peer restarted
    /// and the old connection is dead.
    pub instance: u64,
}

impl Preamble {
    pub fn ours(instance: u64) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            instance,
        }
    }

    pub fn to_bytes(self) -> [u8; PREAMBLE_LEN] {
        let mut out = [0; PREAMBLE_LEN];
        out[..4].copy_from_slice(&MAGIC);
        out[4..6].copy_from_slice(&self.version.to_le_bytes());
        out[6..].copy_from_slice(&self.instance.to_le_bytes());
        out
    }

    pub fn from_bytes(bytes: &[u8; PREAMBLE_LEN]) -> Option<Self> {
        if bytes[..4] != MAGIC {
            return None;
        }
        Some(Self {
            version: u16::from_le_bytes([bytes[4], bytes[5]]),
            instance: u64::from_le_bytes(bytes[6..].try_into().expect("8 bytes")),
        })
    }

    pub async fn read(recv: &mut RecvStream) -> Result<Self, WireError> {
        let mut bytes = [0; PREAMBLE_LEN];
        recv.read_exact(&mut bytes).await.map_err(|_| WireError::Closed)?;
        Self::from_bytes(&bytes).ok_or(WireError::Malformed)
    }

    pub async fn write(self, send: &mut SendStream) -> Result<(), WireError> {
        send.write_all(&self.to_bytes()).await.map_err(|_| WireError::Closed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum WireError {
    #[error("stream closed")]
    Closed,
    #[error("malformed data")]
    Malformed,
    #[error("frame too long")]
    TooLong,
}

/// Frame: length as u32 little endian, then the bytes. At most `MAX_MESSAGE_LEN`.
pub(crate) async fn write_frame(send: &mut SendStream, bytes: &[u8]) -> Result<(), WireError> {
    if bytes.len() > MAX_MESSAGE_LEN {
        return Err(WireError::TooLong);
    }
    let len = u32::try_from(bytes.len()).expect("checked above");
    let mut buf = Vec::with_capacity(4 + bytes.len());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(bytes);
    send.write_all(&buf).await.map_err(|_| WireError::Closed)
}

/// Reads one frame. A clean end of the stream before a frame starts is `Ok(None)`.
pub(crate) async fn read_frame(recv: &mut RecvStream) -> Result<Option<Vec<u8>>, WireError> {
    let mut len = [0; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(_) => return Err(WireError::Closed),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_MESSAGE_LEN {
        return Err(WireError::TooLong);
    }
    let mut buf = vec![0; len];
    recv.read_exact(&mut buf).await.map_err(|_| WireError::Closed)?;
    Ok(Some(buf))
}

/// Application close codes, so the other side can say why a connection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum CloseCode {
    Shutdown = 0,
    NotPaired = 1,
    VersionMismatch = 2,
    Duplicate = 3,
    Timeout = 4,
    ProtocolError = 5,
    Unpaired = 6,
    WrongCode = 7,
    PairingClosed = 8,
    Paired = 9,
    WrongPeer = 10,
    Replaced = 11,
    Sleep = 12,
}

impl CloseCode {
    pub fn var_int(self) -> VarInt {
        VarInt::from_u32(self as u32)
    }

    pub fn from_var_int(code: VarInt) -> Option<Self> {
        use CloseCode::*;
        Some(match code.into_inner() {
            0 => Shutdown,
            1 => NotPaired,
            2 => VersionMismatch,
            3 => Duplicate,
            4 => Timeout,
            5 => ProtocolError,
            6 => Unpaired,
            7 => WrongCode,
            8 => PairingClosed,
            9 => Paired,
            10 => WrongPeer,
            11 => Replaced,
            12 => Sleep,
            _ => return None,
        })
    }

    pub fn close(self, connection: &quinn::Connection, reason: &str) {
        connection.close(self.var_int(), reason.as_bytes());
    }
}

/// Close code and reason the other side sent, if the connection ended that way.
pub(crate) fn remote_close(error: &quinn::ConnectionError) -> Option<(Option<CloseCode>, String)> {
    match error {
        quinn::ConnectionError::ApplicationClosed(close) => Some((
            CloseCode::from_var_int(close.error_code),
            String::from_utf8_lossy(&close.reason).into_owned(),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preamble_round_trip() {
        let p = Preamble {
            version: 513,
            instance: 0x0123_4567_89AB_CDEF,
        };
        assert_eq!(Preamble::from_bytes(&p.to_bytes()), Some(p));
        let mut bad = p.to_bytes();
        bad[0] = b'X';
        assert_eq!(Preamble::from_bytes(&bad), None);
    }

    #[test]
    fn close_codes_round_trip() {
        for n in 0..=20u32 {
            if let Some(code) = CloseCode::from_var_int(VarInt::from_u32(n)) {
                assert_eq!(code as u32, n);
            }
        }
        assert_eq!(CloseCode::from_var_int(VarInt::from_u32(12)), Some(CloseCode::Sleep));
        assert_eq!(CloseCode::from_var_int(VarInt::from_u32(999)), None);
    }
}
