//! One live connection to a paired peer: setup (version, hello), heartbeat, latency probes, routing.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quinn::{Connection, RecvStream, SendStream};
use tc_protocol::{Message, PROTOCOL_VERSION, Screen, decode, encode};
use tokio::sync::mpsc;
use tokio::time::{MissedTickBehavior, timeout};

use crate::identity::Fingerprint;
use crate::wire::{CloseCode, Preamble, SETUP_TIMEOUT, WireError, read_frame, remote_close, write_frame};

pub(crate) const HEARTBEAT: Duration = Duration::from_millis(500);
/// Without any sign of life for this long the peer counts as lost.
pub(crate) const PEER_TIMEOUT: Duration = Duration::from_secs(2);
/// Probes without answer after this long count as lost.
const PING_LOST_AFTER: Duration = Duration::from_secs(2);
const RECENT_SAMPLES: usize = 64;
const MAX_UNREAD_SAMPLES: usize = 100_000;

/// Measurements of a link.
#[derive(Debug)]
pub(crate) struct LinkStats {
    pub last_seen: Instant,
    pings_sent: u64,
    pongs: u64,
    lost: u64,
    next_ping: u32,
    outstanding: VecDeque<(u32, Instant)>,
    recent: VecDeque<Duration>,
    unread: Vec<Duration>,
}

impl LinkStats {
    pub fn new() -> Self {
        Self {
            last_seen: Instant::now(),
            pings_sent: 0,
            pongs: 0,
            lost: 0,
            next_ping: 0,
            outstanding: VecDeque::new(),
            recent: VecDeque::new(),
            unread: Vec::new(),
        }
    }

    fn ping(&mut self, now: Instant) -> u32 {
        // Unanswered probes older than the limit are lost
        while let Some((_, sent)) = self.outstanding.front() {
            if now.duration_since(*sent) < PING_LOST_AFTER {
                break;
            }
            self.outstanding.pop_front();
            self.lost += 1;
        }
        let seq = self.next_ping;
        self.next_ping = self.next_ping.wrapping_add(1);
        self.outstanding.push_back((seq, now));
        self.pings_sent += 1;
        seq
    }

    fn pong(&mut self, seq: u32, now: Instant) {
        let Some(pos) = self.outstanding.iter().position(|(s, _)| *s == seq) else {
            return;
        };
        let (_, sent) = self.outstanding.remove(pos).expect("position is valid");
        let rtt = now.duration_since(sent);
        self.pongs += 1;
        if self.recent.len() == RECENT_SAMPLES {
            self.recent.pop_front();
        }
        self.recent.push_back(rtt);
        if self.unread.len() < MAX_UNREAD_SAMPLES {
            self.unread.push(rtt);
        }
    }

    pub fn take_samples(&mut self) -> Vec<Duration> {
        std::mem::take(&mut self.unread)
    }

    fn median_rtt(&self) -> Option<Duration> {
        median(self.recent.iter().copied().collect())
    }
}

pub fn median(mut samples: Vec<Duration>) -> Option<Duration> {
    if samples.is_empty() {
        return None;
    }
    samples.sort();
    Some(samples[samples.len() / 2])
}

/// State of a connected peer, for status displays.
#[derive(Debug, Clone)]
pub struct LinkInfo {
    pub address: SocketAddr,
    /// This side dialed the connection.
    pub dialed: bool,
    pub connected_since: Instant,
    /// Median round trip of the recent latency probes (application level, datagrams).
    pub rtt: Option<Duration>,
    /// Round trip as estimated by QUIC.
    pub quic_rtt: Duration,
    pub probes_sent: u64,
    pub probes_lost: u64,
    /// Name, screens and program version from the peer's `Hello`.
    pub name: String,
    pub screens: Vec<Screen>,
    pub app_version: String,
}

/// What `setup` learned about the other side.
pub(crate) struct Established {
    pub send: SendStream,
    pub recv: RecvStream,
    pub instance: u64,
    pub name: String,
    pub screens: Vec<Screen>,
    pub app_version: String,
}

#[derive(Debug)]
pub(crate) enum SetupError {
    /// The peer speaks another protocol version.
    Version(u16),
    /// The peer closed the connection on purpose.
    Closed(Option<CloseCode>, String),
    Protocol(&'static str),
    Lost,
    /// The acceptor refused it as duplicate (only on the accepting side).
    Rejected,
}

fn setup_err(connection: &Connection, e: WireError) -> SetupError {
    match connection.close_reason().as_ref().and_then(remote_close) {
        Some((code, reason)) => SetupError::Closed(code, reason),
        None if e == WireError::Closed => SetupError::Lost,
        None => SetupError::Protocol("malformed data during setup"),
    }
}

fn stream_err(connection: &Connection) -> SetupError {
    match connection.close_reason().as_ref().and_then(remote_close) {
        Some((code, reason)) => SetupError::Closed(code, reason),
        None => SetupError::Lost,
    }
}

/// Version check and `Hello` in both directions. The dialer speaks first; the acceptor calls
/// `admit` after reading the dialer's `Hello` and only answers if it returns `true`, so a
/// connection it refuses never looks established on the dialer's side.
pub(crate) async fn setup(
    connection: &Connection,
    dialer: bool,
    instance: u64,
    hello: &Message,
    admit: impl FnOnce(&Established) -> bool,
) -> Result<Established, SetupError> {
    let work = async {
        let (mut send, mut recv) = if dialer {
            connection.open_bi().await.map_err(|_| stream_err(connection))?
        } else {
            connection.accept_bi().await.map_err(|_| stream_err(connection))?
        };
        if dialer {
            // Our hello goes along right away; the acceptor checks the version before it decodes it
            Preamble::ours(instance)
                .write(&mut send)
                .await
                .map_err(|e| setup_err(connection, e))?;
            write_frame(&mut send, &encode(hello))
                .await
                .map_err(|e| setup_err(connection, e))?;
        }
        let theirs = Preamble::read(&mut recv).await.map_err(|e| setup_err(connection, e))?;
        if !dialer {
            Preamble::ours(instance)
                .write(&mut send)
                .await
                .map_err(|e| setup_err(connection, e))?;
        }
        if theirs.version != PROTOCOL_VERSION {
            // Let our preamble arrive first, so the other side learns our version from it as well
            let _ = send.finish();
            let _ = timeout(Duration::from_secs(1), send.stopped()).await;
            CloseCode::VersionMismatch.close(connection, &format!("protocol version {PROTOCOL_VERSION}"));
            return Err(SetupError::Version(theirs.version));
        }
        let frame = read_frame(&mut recv)
            .await
            .map_err(|e| setup_err(connection, e))?
            .ok_or(SetupError::Protocol("stream ended before hello"))?;
        let Ok(Message::Hello { name, screens, app, .. }) = decode(&frame) else {
            CloseCode::ProtocolError.close(connection, "expected hello");
            return Err(SetupError::Protocol("expected hello"));
        };
        let established = Established {
            send,
            recv,
            instance: theirs.instance,
            name,
            screens,
            app_version: app,
        };
        if !dialer {
            if !admit(&established) {
                return Err(SetupError::Rejected);
            }
            let mut send = established.send;
            write_frame(&mut send, &encode(hello))
                .await
                .map_err(|e| setup_err(connection, e))?;
            return Ok(Established { send, ..established });
        }
        Ok(established)
    };
    match timeout(SETUP_TIMEOUT, work).await {
        Ok(result) => result,
        Err(_) => {
            CloseCode::Timeout.close(connection, "setup took too long");
            Err(SetupError::Lost)
        }
    }
}

/// Why a running link ended, as seen from this side.
#[derive(Debug)]
pub(crate) enum LinkEnd {
    Timeout,
    Remote(Option<CloseCode>, String),
    Local,
    Lost(String),
    Protocol(String),
    UnpairedByPeer,
}

/// Hooks from the link into the node.
pub(crate) trait LinkHost: Send + Sync + 'static {
    fn deliver(&self, peer: Fingerprint, message: Message);
    fn deliver_blob(&self, peer: Fingerprint, header: Vec<u8>, data: Vec<u8>);
    fn deliver_stream(&self, peer: Fingerprint, header: Vec<u8>, stream: InStream);
}

/// Data length that marks a stream without a fixed size (files): read until it ends.
const STREAMED: u64 = u64::MAX;
/// Application code when a stream is cancelled.
const CANCELLED: u32 = 1;

/// Bulk data of unknown length from the peer (a file transfer). Dropping it without reading to the
/// end cancels the transfer on the sender's side.
pub struct InStream(RecvStream);

impl InStream {
    /// Reads some bytes; 0 at the end.
    pub async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.0.read(buf).await {
            Ok(Some(n)) => Ok(n),
            Ok(None) => Ok(0),
            Err(e) => Err(std::io::Error::other(e)),
        }
    }

    pub fn cancel(mut self) {
        let _ = self.0.stop(quinn::VarInt::from_u32(CANCELLED));
    }
}

/// Bulk data of unknown length to the peer.
pub struct OutStream(SendStream);

impl OutStream {
    pub async fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.0.write_all(data).await.map_err(std::io::Error::other)
    }

    /// Ends the stream and waits until the peer has everything.
    pub async fn finish(mut self) -> std::io::Result<()> {
        self.0.finish().map_err(std::io::Error::other)?;
        let _ = self.0.stopped().await;
        Ok(())
    }

    pub fn cancel(mut self) {
        let _ = self.0.reset(quinn::VarInt::from_u32(CANCELLED));
    }
}

pub(crate) async fn open_stream(connection: &Connection, header: &[u8]) -> Result<OutStream, WireError> {
    if header.len() > MAX_BLOB_HEADER {
        return Err(WireError::TooLong);
    }
    let mut send = connection.open_uni().await.map_err(|_| WireError::Closed)?;
    let mut prefix = Vec::with_capacity(12 + header.len());
    prefix.extend_from_slice(&(header.len() as u32).to_le_bytes());
    prefix.extend_from_slice(header);
    prefix.extend_from_slice(&STREAMED.to_le_bytes());
    send.write_all(&prefix).await.map_err(|_| WireError::Closed)?;
    Ok(OutStream(send))
}

enum UniStream {
    Blob(Vec<u8>, Vec<u8>),
    Stream(Vec<u8>, RecvStream),
}

/// Largest header of a blob.
const MAX_BLOB_HEADER: usize = 64 * 1024;
/// Largest blob (clipboard content, later file chunks).
pub const MAX_BLOB: usize = tc_protocol::MAX_CLIP_LEN + 1024;

/// A blob on its own unidirectional stream: header length (u32), header, data length (u64), data.
pub(crate) async fn write_blob(connection: &Connection, header: &[u8], data: &[u8]) -> Result<(), WireError> {
    if header.len() > MAX_BLOB_HEADER || data.len() > MAX_BLOB {
        return Err(WireError::TooLong);
    }
    let mut send = connection.open_uni().await.map_err(|_| WireError::Closed)?;
    let mut prefix = Vec::with_capacity(12 + header.len());
    prefix.extend_from_slice(&(header.len() as u32).to_le_bytes());
    prefix.extend_from_slice(header);
    prefix.extend_from_slice(&(data.len() as u64).to_le_bytes());
    send.write_all(&prefix).await.map_err(|_| WireError::Closed)?;
    send.write_all(data).await.map_err(|_| WireError::Closed)?;
    send.finish().map_err(|_| WireError::Closed)?;
    // Wait until the peer has everything, so dropping the connection right after loses nothing
    let _ = send.stopped().await;
    Ok(())
}

async fn read_blob(mut recv: RecvStream) -> Result<UniStream, WireError> {
    let mut len = [0; 4];
    recv.read_exact(&mut len).await.map_err(|_| WireError::Closed)?;
    let header_len = u32::from_le_bytes(len) as usize;
    if header_len > MAX_BLOB_HEADER {
        return Err(WireError::TooLong);
    }
    let mut header = vec![0; header_len];
    recv.read_exact(&mut header).await.map_err(|_| WireError::Closed)?;
    let mut len = [0; 8];
    recv.read_exact(&mut len).await.map_err(|_| WireError::Closed)?;
    if u64::from_le_bytes(len) == STREAMED {
        return Ok(UniStream::Stream(header, recv));
    }
    let data_len = usize::try_from(u64::from_le_bytes(len)).map_err(|_| WireError::TooLong)?;
    if data_len > MAX_BLOB {
        return Err(WireError::TooLong);
    }
    // Grows with what really arrives, so a lying length cannot reserve memory up front
    let mut data = Vec::new();
    let mut chunk = vec![0; 64 * 1024];
    while data.len() < data_len {
        let want = (data_len - data.len()).min(chunk.len());
        match recv.read(&mut chunk[..want]).await {
            Ok(Some(n)) => data.extend_from_slice(&chunk[..n]),
            _ => return Err(WireError::Closed),
        }
    }
    Ok(UniStream::Blob(header, data))
}

/// Runs a link until it ends. `outgoing` carries stream messages from the node.
pub(crate) async fn run(
    connection: Connection,
    mut established: Established,
    peer: Fingerprint,
    stats: Arc<Mutex<LinkStats>>,
    mut outgoing: mpsc::UnboundedReceiver<Message>,
    host: Arc<dyn LinkHost>,
) -> LinkEnd {
    enum Incoming {
        Message(Message),
        End(LinkEnd),
    }
    let (in_tx, mut in_rx) = mpsc::channel::<Incoming>(1024);

    // Control stream reader
    let mut recv = established.recv;
    let tx = in_tx.clone();
    let stream_reader = tokio::spawn(async move {
        loop {
            let item = match read_frame(&mut recv).await {
                Ok(Some(frame)) => match decode(&frame) {
                    Ok(message) if !message.is_datagram() => Incoming::Message(message),
                    Ok(other) => {
                        tracing::debug!("ignoring {other:?} on the control stream");
                        continue;
                    }
                    Err(e) => Incoming::End(LinkEnd::Protocol(format!("bad message: {e}"))),
                },
                Ok(None) => Incoming::End(LinkEnd::Protocol("control stream ended".into())),
                Err(WireError::TooLong) => Incoming::End(LinkEnd::Protocol("frame too long".into())),
                Err(_) => break,
            };
            let end = matches!(item, Incoming::End(_));
            if tx.send(item).await.is_err() || end {
                break;
            }
        }
    });

    // Datagram reader; anything that is not meant for datagrams is dropped. Latency probes are
    // handled right here, so the measurement does not include the time in the main loop.
    let conn = connection.clone();
    let tx = in_tx;
    let probe_stats = stats.clone();
    let datagram_reader = tokio::spawn(async move {
        while let Ok(bytes) = conn.read_datagram().await {
            let message = match decode(&bytes) {
                Ok(message) if message.is_datagram() => message,
                _ => {
                    tracing::debug!("dropping an invalid datagram");
                    continue;
                }
            };
            let now = Instant::now();
            match message {
                Message::Ping { seq } => {
                    let _ = conn.send_datagram(encode(&Message::Pong { seq }).into());
                }
                Message::Pong { seq } => probe_stats.lock().expect("stats lock").pong(seq, now),
                _ => {}
            }
            probe_stats.lock().expect("stats lock").last_seen = now;
            if tx.send(Incoming::Message(message)).await.is_err() {
                break;
            }
        }
    });

    // Blobs on unidirectional streams, each read on its own so a big one never holds up the rest
    let conn = connection.clone();
    let blob_host = host.clone();
    let blob_stats = stats.clone();
    let blob_reader = tokio::spawn(async move {
        while let Ok(recv) = conn.accept_uni().await {
            let host = blob_host.clone();
            let stats = blob_stats.clone();
            tokio::spawn(async move {
                match read_blob(recv).await {
                    Ok(UniStream::Blob(header, data)) => {
                        stats.lock().expect("stats lock").last_seen = Instant::now();
                        host.deliver_blob(peer, header, data);
                    }
                    Ok(UniStream::Stream(header, recv)) => host.deliver_stream(peer, header, InStream(recv)),
                    Err(e) => tracing::debug!("dropping an incomplete blob: {e}"),
                }
            });
        }
    });

    let mut tick = tokio::time::interval(HEARTBEAT);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut heartbeat = 0u32;
    let send = &mut established.send;

    let end = loop {
        tokio::select! {
            biased;
            item = in_rx.recv() => {
                let message = match item {
                    Some(Incoming::Message(m)) => m,
                    Some(Incoming::End(end)) => break end,
                    None => break connection_end(&connection),
                };
                stats.lock().expect("stats lock").last_seen = Instant::now();
                match message {
                    // Probes were answered and measured by the datagram reader
                    Message::Heartbeat { .. } | Message::Hello { .. } | Message::Ping { .. } | Message::Pong { .. } => {}
                    Message::Unpair => break LinkEnd::UnpairedByPeer,
                    other => host.deliver(peer, other),
                }
            }
            message = outgoing.recv() => {
                let Some(message) = message else {
                    // The node dropped this link (replaced, unpaired, shut down) and closed it
                    break LinkEnd::Local;
                };
                if write_frame(send, &encode(&message)).await.is_err() {
                    break connection_end(&connection);
                }
            }
            _ = tick.tick() => {
                let now = Instant::now();
                let silent = now.duration_since(stats.lock().expect("stats lock").last_seen);
                if silent > PEER_TIMEOUT {
                    CloseCode::Timeout.close(&connection, "no heartbeat");
                    break LinkEnd::Timeout;
                }
                heartbeat = heartbeat.wrapping_add(1);
                if write_frame(send, &encode(&Message::Heartbeat { seq: heartbeat })).await.is_err() {
                    break connection_end(&connection);
                }
                let seq = stats.lock().expect("stats lock").ping(now);
                let _ = connection.send_datagram(encode(&Message::Ping { seq }).into());
            }
        }
    };
    stream_reader.abort();
    datagram_reader.abort();
    blob_reader.abort();
    end
}

fn connection_end(connection: &Connection) -> LinkEnd {
    match connection.close_reason() {
        Some(quinn::ConnectionError::LocallyClosed) => LinkEnd::Local,
        Some(quinn::ConnectionError::TimedOut) => LinkEnd::Timeout,
        Some(e) => match remote_close(&e) {
            Some((code, reason)) => LinkEnd::Remote(code, reason),
            None => LinkEnd::Lost(e.to_string()),
        },
        None => LinkEnd::Lost("stream failed".into()),
    }
}

pub(crate) fn info(
    connection: &Connection,
    dialed: bool,
    since: Instant,
    stats: &LinkStats,
    name: &str,
    screens: &[Screen],
    app_version: &str,
) -> LinkInfo {
    LinkInfo {
        address: crate::canonical(connection.remote_address()),
        dialed,
        connected_since: since,
        rtt: stats.median_rtt(),
        quic_rtt: connection.rtt(),
        probes_sent: stats.pings_sent,
        probes_lost: stats.lost,
        name: name.to_string(),
        screens: screens.to_vec(),
        app_version: app_version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_statistics() {
        let mut stats = LinkStats::new();
        let t0 = Instant::now();
        let a = stats.ping(t0);
        let b = stats.ping(t0);
        stats.pong(a, t0 + Duration::from_millis(3));
        stats.pong(a, t0 + Duration::from_millis(4)); // duplicate answer is ignored
        stats.pong(999, t0); // unknown answer is ignored
        assert_eq!(stats.pongs, 1);
        assert_eq!(stats.median_rtt(), Some(Duration::from_millis(3)));
        // b never gets an answer and counts as lost once the limit passed
        stats.ping(t0 + PING_LOST_AFTER + Duration::from_millis(1));
        assert_eq!(stats.lost, 1);
        stats.pong(b, t0 + PING_LOST_AFTER * 2);
        assert_eq!(stats.pongs, 1);
        assert_eq!(stats.take_samples(), vec![Duration::from_millis(3)]);
        assert!(stats.take_samples().is_empty());
    }

    #[test]
    fn median_of_samples() {
        let ms = Duration::from_millis;
        assert_eq!(median(vec![]), None);
        assert_eq!(median(vec![ms(5), ms(1), ms(3)]), Some(ms(3)));
    }
}
