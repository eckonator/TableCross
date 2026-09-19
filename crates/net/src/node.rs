//! A running TableCross network node: one QUIC endpoint for listening and dialing.
//!
//! Duplicate connections: there is at most one link per peer. When a second one is established, the
//! newer one replaces the older one if the peer restarted (different instance) or the older one went
//! silent; otherwise both sides keep the connection dialed by the peer with the smaller fingerprint.
//! Both sides apply the same rule, so they agree without further messages.

use std::collections::{HashMap, HashSet};
use std::io;
use std::net::{Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use quinn::Connection;
use tc_protocol::{Message, PROTOCOL_VERSION, Screen, encode};
use tokio::sync::{mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{sleep, timeout};

use crate::DEFAULT_PORT;
use crate::discovery::{Discovered, Discovery};
use crate::identity::{CERT_NAME, Fingerprint, Identity};
use crate::link::{self, LinkEnd, LinkHost, LinkInfo, LinkStats, SetupError};
use crate::pairing::{Exchange, PairMsg, PairingCode, PairingWindow};
use crate::store::{PairedPeer, PeerStore};
use crate::tls::{self, ALPN_LINK, ALPN_PAIR};
use crate::wire::{CloseCode, SETUP_TIMEOUT, read_frame, remote_close, write_frame};

const BACKOFF_MIN: Duration = Duration::from_millis(200);
const BACKOFF_MAX: Duration = Duration::from_secs(2);
/// Addresses are tried in parallel, each one this much later than the previous.
const DIAL_STAGGER: Duration = Duration::from_millis(150);
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
/// The peer with the larger fingerprint waits this long before dialing, so that usually only one side
/// dials after a link was lost (both dialing at once works, but costs an extra connection).
const HEAD_START: Duration = Duration::from_millis(300);
/// A link without any sign of life for this long may be replaced by a new connection.
const STALE: Duration = Duration::from_secs(1);
/// A gap this large between one-second ticks means the computer was asleep.
const SLEEP_GAP: Duration = Duration::from_secs(4);

#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Shown to other computers.
    pub name: String,
    /// Default `[::]:24800` (IPv6 and IPv4).
    pub bind: SocketAddr,
    /// Announce and search with mDNS.
    pub discovery: bool,
    /// Dial paired peers. Without it the node only accepts connections.
    pub dial: bool,
    pub screens: Vec<Screen>,
    /// Program version shown to other computers.
    pub app_version: String,
}

impl NodeConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            bind: SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), DEFAULT_PORT),
            discovery: true,
            dial: true,
            screens: Vec::new(),
            app_version: env!("CARGO_PKG_VERSION").into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("cannot open UDP port {port}: {source} (is TableCross already running?)")]
    Bind { port: u16, source: io::Error },
    #[error("cannot start network discovery: {0}")]
    Discovery(#[from] mdns_sd::Error),
}

/// Why a link ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disconnect {
    /// No sign of life for 2 seconds.
    Timeout,
    /// The other side quit.
    PeerShutdown,
    /// The other side went to sleep or woke up.
    PeerSlept,
    /// This computer woke from sleep; the link is rebuilt.
    Slept,
    /// A newer connection to the same peer took over.
    Replaced,
    /// The pairing was removed (here or on the other side).
    Unpaired,
    /// This node shuts down.
    Shutdown,
    /// The other side sent something it should not have.
    ProtocolError(String),
    /// Network error.
    Lost(String),
}

/// Why a connection was refused before it became a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A computer that is not paired with this one tried to connect.
    NotPaired,
    /// Different protocol versions; the older side needs an update.
    VersionMismatch {
        ours: u16,
        theirs: u16,
    },
    /// Another device answered at the remembered address.
    WrongPeer,
    ProtocolError(String),
}

#[derive(Debug, Clone)]
pub enum Event {
    /// A link to a paired peer is up. Also sent when a link replaces an older one (after
    /// `Disconnected { reason: Replaced }`); the receiver should treat the peer as freshly connected.
    Connected {
        peer: Fingerprint,
        name: String,
        screens: Vec<Screen>,
        address: SocketAddr,
    },
    /// The link is gone. Whoever controls input on behalf of this peer must release everything.
    Disconnected { peer: Fingerprint, reason: Disconnect },
    /// A message from the peer (stream or datagram), except the ones the network layer handles itself.
    Message { peer: Fingerprint, message: Message },
    /// Bulk data from the peer (clipboard content), sent with `Node::send_blob`.
    Blob {
        peer: Fingerprint,
        header: Vec<u8>,
        data: Vec<u8>,
    },
    /// A stream of unknown length from the peer (files), opened with `Node::open_stream`.
    Stream {
        peer: Fingerprint,
        header: Vec<u8>,
        stream: StreamSlot,
    },
    /// Pairing succeeded (on either side).
    Paired { peer: PairedPeer },
    /// Someone typed a wrong code for the code shown here.
    PairingFailed { attempts_left: u8 },
    /// The shown code is no longer valid (too many attempts).
    PairingClosed,
    /// A connection was refused. `peer` is set when it is known.
    Refused {
        peer: Option<Fingerprint>,
        address: SocketAddr,
        reason: Refusal,
    },
    /// The pairing with `peer` was removed, here (`by_peer: false`) or on the other side.
    Unpaired { peer: Fingerprint, by_peer: bool },
    /// This computer woke from sleep after about `slept`; links are being rebuilt.
    Woke { slept: Duration },
}

/// Holds an incoming stream inside an event (events are cloneable, the stream is taken once).
#[derive(Clone)]
pub struct StreamSlot(Arc<Mutex<Option<link::InStream>>>);

impl StreamSlot {
    pub fn take(&self) -> Option<link::InStream> {
        self.0.lock().expect("stream slot").take()
    }
}

impl std::fmt::Debug for StreamSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StreamSlot")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PairError {
    #[error("no address to try")]
    NoAddress,
    #[error("the other computer cannot be reached")]
    Unreachable,
    #[error("the other computer is not showing a pairing code (expired, or too many wrong attempts)")]
    NotShowingCode,
    #[error("wrong code")]
    WrongCode,
    #[error("the other computer uses protocol version {theirs}, this one {ours}; update the older one")]
    VersionMismatch { ours: u16, theirs: u16 },
    #[error("pairing failed: {0}")]
    Protocol(String),
    #[error("cannot save the pairing: {0}")]
    Store(String),
}

#[derive(Debug, Clone)]
pub struct PeerStatus {
    pub peer: PairedPeer,
    /// `Some` while connected.
    pub link: Option<LinkInfo>,
    /// Links established after the first one.
    pub reconnects: u32,
    /// Why the last connection attempt was refused, until the next success.
    pub refusal: Option<Refusal>,
}

struct LinkEntry {
    id: u64,
    connection: Connection,
    outgoing: mpsc::UnboundedSender<Message>,
    dialer: Fingerprint,
    peer_instance: u64,
    stats: Arc<Mutex<LinkStats>>,
    since: Instant,
    name: String,
    screens: Vec<Screen>,
    app_version: String,
    /// The blob being sent; a new one replaces it.
    blob: Option<JoinHandle<()>>,
}

struct State {
    store: PeerStore,
    links: HashMap<Fingerprint, LinkEntry>,
    supervisors: HashMap<Fingerprint, JoinHandle<()>>,
    pairing: Option<PairingWindow>,
    connected_before: HashSet<Fingerprint>,
    reconnects: HashMap<Fingerprint, u32>,
    refusals: HashMap<Fingerprint, Refusal>,
    shut_down: bool,
}

struct Inner {
    identity: Identity,
    config: NodeConfig,
    instance: u64,
    endpoint: quinn::Endpoint,
    local_addr: SocketAddr,
    link_client: quinn::ClientConfig,
    pair_client: quinn::ClientConfig,
    state: Mutex<State>,
    events: mpsc::UnboundedSender<Event>,
    /// Bumped on every change of links, peers or discovery results.
    changed: watch::Sender<()>,
    /// Bumped when the computer woke from sleep.
    woke: watch::Sender<u64>,
    discovery: Option<Discovery>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    next_link_id: AtomicU64,
}

impl LinkHost for Inner {
    fn deliver(&self, peer: Fingerprint, message: Message) {
        let _ = self.events.send(Event::Message { peer, message });
    }

    fn deliver_blob(&self, peer: Fingerprint, header: Vec<u8>, data: Vec<u8>) {
        let _ = self.events.send(Event::Blob { peer, header, data });
    }

    fn deliver_stream(&self, peer: Fingerprint, header: Vec<u8>, stream: link::InStream) {
        let _ = self.events.send(Event::Stream {
            peer,
            header,
            stream: StreamSlot(Arc::new(Mutex::new(Some(stream)))),
        });
    }
}

/// Handle to a running node; cheap to clone.
#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
}

impl Node {
    /// Opens the endpoint, starts discovery and dials all paired peers.
    /// Must run inside a Tokio runtime.
    pub async fn start(
        config: NodeConfig,
        identity: Identity,
        store: PeerStore,
    ) -> Result<(Node, mpsc::UnboundedReceiver<Event>), NodeError> {
        let socket = bind_patiently(config.bind).await.map_err(|source| NodeError::Bind {
            port: config.bind.port(),
            source,
        })?;
        let endpoint = quinn::Endpoint::new(
            quinn::EndpointConfig::default(),
            Some(tls::server_config(&identity)),
            socket,
            Arc::new(quinn::TokioRuntime),
        )
        .map_err(|source| NodeError::Bind {
            port: config.bind.port(),
            source,
        })?;
        let local_addr = endpoint.local_addr().map_err(|source| NodeError::Bind {
            port: config.bind.port(),
            source,
        })?;
        let (events, events_rx) = mpsc::unbounded_channel();
        let (changed, _) = watch::channel(());
        let discovery = if config.discovery {
            Some(Discovery::start(
                identity.fingerprint(),
                &config.name,
                local_addr.port(),
                changed.clone(),
            )?)
        } else {
            None
        };
        let mut instance = [0u8; 8];
        getrandom::fill(&mut instance).expect("system random generator");
        let inner = Arc::new(Inner {
            link_client: tls::client_config(&identity, ALPN_LINK),
            pair_client: tls::client_config(&identity, ALPN_PAIR),
            identity,
            config,
            instance: u64::from_le_bytes(instance),
            endpoint,
            local_addr,
            state: Mutex::new(State {
                store,
                links: HashMap::new(),
                supervisors: HashMap::new(),
                pairing: None,
                connected_before: HashSet::new(),
                reconnects: HashMap::new(),
                refusals: HashMap::new(),
                shut_down: false,
            }),
            events,
            changed,
            woke: watch::channel(0).0,
            discovery,
            tasks: Mutex::new(Vec::new()),
            next_link_id: AtomicU64::new(1),
        });
        let accept = tokio::spawn(accept_loop(inner.clone()));
        let sleep_watch = tokio::spawn(watch_sleep(inner.clone()));
        inner.tasks.lock().expect("tasks lock").extend([accept, sleep_watch]);
        let peers: Vec<_> = inner.lock().store.peers().iter().map(|p| p.fingerprint).collect();
        for peer in peers {
            start_supervisor(&inner, peer);
        }
        Ok((Node { inner }, events_rx))
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.inner.identity.fingerprint()
    }

    pub fn name(&self) -> &str {
        &self.inner.config.name
    }

    /// The bound address (port 0 in the configuration gives a random port).
    pub fn local_addr(&self) -> SocketAddr {
        self.inner.local_addr
    }

    /// Shows a new code for the next `PAIRING_WINDOW`; a previous code becomes invalid.
    pub fn start_pairing(&self) -> PairingCode {
        let code = PairingCode::random();
        let window = PairingWindow::new(code.clone());
        let until = window.until;
        self.inner.lock().pairing = Some(window);
        self.inner.announce(true);
        // Stop announcing the pairing flag once the code expires
        let inner = self.inner.clone();
        let task = tokio::spawn(async move {
            tokio::time::sleep_until(until.into()).await;
            let mut state = inner.lock();
            if state.pairing.as_ref().is_some_and(|w| w.until == until) {
                state.pairing = None;
                drop(state);
                inner.announce(false);
            }
        });
        self.inner.tasks.lock().expect("tasks lock").push(task);
        code
    }

    pub fn stop_pairing(&self) {
        if self.inner.lock().pairing.take().is_some() {
            self.inner.announce(false);
        }
    }

    /// Pairs with the computer showing `code`, trying the given addresses.
    pub async fn pair(&self, addresses: &[SocketAddr], code: &PairingCode) -> Result<PairedPeer, PairError> {
        pair_with(&self.inner, addresses, code).await
    }

    /// Removes the pairing here and, if connected, on the other side.
    pub fn unpair(&self, peer: &Fingerprint) -> io::Result<bool> {
        let mut state = self.inner.lock();
        let removed = state.store.remove(peer)?;
        if let Some(entry) = state.links.remove(peer) {
            let _ = entry.outgoing.send(Message::Unpair);
            let connection = entry.connection.clone();
            let _ = self.inner.events.send(Event::Disconnected {
                peer: *peer,
                reason: Disconnect::Unpaired,
            });
            // Give the message a moment on the wire, the close code says the same anyway
            tokio::spawn(async move {
                sleep(Duration::from_millis(200)).await;
                CloseCode::Unpaired.close(&connection, "unpaired");
            });
        }
        if let Some(task) = state.supervisors.remove(peer) {
            task.abort();
        }
        drop(state);
        if removed {
            let _ = self.inner.events.send(Event::Unpaired {
                peer: *peer,
                by_peer: false,
            });
        }
        self.inner.changed.send_replace(());
        Ok(removed)
    }

    /// Adds an address to try for a paired peer (e.g. typed in by the user).
    pub fn add_address(&self, peer: &Fingerprint, address: SocketAddr) -> io::Result<()> {
        let result = self.inner.lock().store.remember_address(peer, address);
        self.inner.changed.send_replace(());
        result
    }

    /// Sends a message to a connected peer: datagram messages unreliably, everything else in order.
    /// `false` if the peer is not connected.
    pub fn send(&self, peer: &Fingerprint, message: Message) -> bool {
        let state = self.inner.lock();
        let Some(entry) = state.links.get(peer) else {
            return false;
        };
        if message.is_datagram() {
            let _ = entry.connection.send_datagram(encode(&message).into());
            true
        } else {
            entry.outgoing.send(message).is_ok()
        }
    }

    /// Sends bulk data on a stream of its own; a blob still on its way to that peer is abandoned.
    /// `false` if the peer is not connected or the data is too large.
    pub fn send_blob(&self, peer: &Fingerprint, header: Vec<u8>, data: Vec<u8>) -> bool {
        if data.len() > link::MAX_BLOB {
            return false;
        }
        let mut state = self.inner.lock();
        let Some(entry) = state.links.get_mut(peer) else {
            return false;
        };
        if let Some(previous) = entry.blob.take() {
            previous.abort();
        }
        let connection = entry.connection.clone();
        entry.blob = Some(tokio::spawn(async move {
            if let Err(e) = link::write_blob(&connection, &header, &data).await {
                tracing::debug!("blob not sent: {e}");
            }
        }));
        true
    }

    /// Opens a stream of unknown length to a connected peer (file transfers).
    pub async fn open_stream(&self, peer: &Fingerprint, header: &[u8]) -> io::Result<link::OutStream> {
        let connection = self
            .inner
            .lock()
            .links
            .get(peer)
            .map(|e| e.connection.clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "peer not connected"))?;
        link::open_stream(&connection, header)
            .await
            .map_err(|e| io::Error::other(e.to_string()))
    }

    pub fn paired(&self) -> Vec<PairedPeer> {
        self.inner.lock().store.peers().to_vec()
    }

    pub fn status(&self) -> Vec<PeerStatus> {
        let state = self.inner.lock();
        state
            .store
            .peers()
            .iter()
            .map(|peer| PeerStatus {
                peer: peer.clone(),
                link: state.links.get(&peer.fingerprint).map(|e| {
                    link::info(
                        &e.connection,
                        e.dialer == self.inner.me(),
                        e.since,
                        &e.stats.lock().expect("stats lock"),
                        &e.name,
                        &e.screens,
                        &e.app_version,
                    )
                }),
                reconnects: state.reconnects.get(&peer.fingerprint).copied().unwrap_or(0),
                refusal: state.refusals.get(&peer.fingerprint).cloned(),
            })
            .collect()
    }

    /// Round trips measured since the last call, for long-running statistics.
    pub fn take_rtt_samples(&self, peer: &Fingerprint) -> Vec<Duration> {
        self.inner
            .lock()
            .links
            .get(peer)
            .map(|e| e.stats.lock().expect("stats lock").take_samples())
            .unwrap_or_default()
    }

    /// TableCross instances found on the network (empty without discovery).
    pub fn discovered(&self) -> Vec<Discovered> {
        self.inner.discovery.as_ref().map(Discovery::found).unwrap_or_default()
    }

    /// Closes all links, tells the peers, and stops.
    pub async fn shutdown(&self) {
        let inner = &self.inner;
        let links: Vec<_> = {
            let mut state = inner.lock();
            if state.shut_down {
                return;
            }
            state.shut_down = true;
            for (_, task) in state.supervisors.drain() {
                task.abort();
            }
            state.links.drain().collect()
        };
        for (peer, entry) in links {
            CloseCode::Shutdown.close(&entry.connection, "shutdown");
            let _ = inner.events.send(Event::Disconnected {
                peer,
                reason: Disconnect::Shutdown,
            });
        }
        for task in inner.tasks.lock().expect("tasks lock").drain(..) {
            task.abort();
        }
        if let Some(discovery) = &inner.discovery {
            discovery.shutdown();
        }
        inner.endpoint.close(CloseCode::Shutdown.var_int(), b"shutdown");
        let _ = timeout(Duration::from_secs(1), inner.endpoint.wait_idle()).await;
    }
}

#[cfg(test)]
impl Node {
    /// Drops all links as if the network failed, without telling the peers.
    pub(crate) fn drop_links_for_tests(&self) {
        let links: Vec<_> = self.inner.lock().links.drain().collect();
        for (peer, entry) in links {
            CloseCode::Timeout.close(&entry.connection, "test");
            let _ = self.inner.events.send(Event::Disconnected {
                peer,
                reason: Disconnect::Timeout,
            });
        }
        self.inner.changed.send_replace(());
    }

    /// Acts as if the computer just woke from sleep.
    pub(crate) fn wake_for_tests(&self) {
        woke_up(&self.inner, Duration::from_secs(60));
    }
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("node state lock")
    }

    fn me(&self) -> Fingerprint {
        self.identity.fingerprint()
    }

    fn hello(&self) -> Message {
        Message::Hello {
            version: PROTOCOL_VERSION,
            name: self.config.name.clone(),
            screens: self.config.screens.clone(),
            app: self.config.app_version.clone(),
        }
    }

    fn announce(&self, pairing: bool) {
        if let Some(discovery) = &self.discovery
            && let Err(e) = discovery.announce(pairing)
        {
            tracing::warn!("cannot announce on the network: {e}");
        }
    }

    fn refuse(&self, peer: Option<Fingerprint>, address: SocketAddr, reason: Refusal) {
        if let Some(peer) = peer {
            let mut state = self.lock();
            if state.refusals.get(&peer) == Some(&reason) {
                return; // Already reported, do not repeat it on every retry
            }
            state.refusals.insert(peer, reason.clone());
        }
        let _ = self.events.send(Event::Refused {
            peer,
            address: crate::canonical(address),
            reason,
        });
    }

    /// The other side removed the pairing.
    fn unpaired_by_peer(&self, peer: Fingerprint) {
        let mut state = self.lock();
        let removed = state.store.remove(&peer).unwrap_or_else(|e| {
            tracing::warn!("cannot save the peer list: {e}");
            true
        });
        if let Some(entry) = state.links.remove(&peer) {
            CloseCode::Unpaired.close(&entry.connection, "unpaired");
            let _ = self.events.send(Event::Disconnected {
                peer,
                reason: Disconnect::Unpaired,
            });
        }
        drop(state);
        if removed {
            let _ = self.events.send(Event::Unpaired { peer, by_peer: true });
        }
        self.changed.send_replace(());
    }

    /// Registers a new link, applying the duplicate rule. `false` if it must be closed.
    fn register(&self, peer: Fingerprint, entry: LinkEntry, address: SocketAddr) -> bool {
        let mut state = self.lock();
        if state.shut_down || !state.store.contains(&peer) {
            return false;
        }
        if let Some(old) = state.links.get(&peer) {
            let preferred = self.me().min(peer);
            let stale = old.stats.lock().expect("stats lock").last_seen.elapsed() > STALE;
            let replace = old.peer_instance != entry.peer_instance
                || stale
                || (entry.dialer == preferred && old.dialer != preferred);
            if !replace {
                return false;
            }
            let old = state.links.remove(&peer).expect("checked above");
            CloseCode::Replaced.close(&old.connection, "replaced by a newer connection");
            let _ = self.events.send(Event::Disconnected {
                peer,
                reason: Disconnect::Replaced,
            });
        }
        if !state.connected_before.insert(peer) {
            *state.reconnects.entry(peer).or_default() += 1;
        }
        state.refusals.remove(&peer);
        if let Err(e) = state.store.remember_address(&peer, address) {
            tracing::warn!("cannot save the peer list: {e}");
        }
        if let Err(e) = state.store.set_name(&peer, &entry.name) {
            tracing::warn!("cannot save the peer list: {e}");
        }
        let _ = self.events.send(Event::Connected {
            peer,
            name: entry.name.clone(),
            screens: entry.screens.clone(),
            address: crate::canonical(address),
        });
        state.links.insert(peer, entry);
        drop(state);
        self.changed.send_replace(());
        true
    }

    /// Removes the link if it is still `id` and reports why it ended.
    fn unregister(&self, peer: Fingerprint, id: u64, end: LinkEnd) {
        let is_current = |state: &State| state.links.get(&peer).is_some_and(|e| e.id == id);
        if !is_current(&self.lock()) {
            return; // Already replaced or removed, and reported there
        }
        if matches!(
            end,
            LinkEnd::UnpairedByPeer | LinkEnd::Remote(Some(CloseCode::Unpaired | CloseCode::NotPaired), _)
        ) {
            self.unpaired_by_peer(peer);
            return;
        }
        let mut state = self.lock();
        if !is_current(&state) {
            return;
        }
        state.links.remove(&peer);
        drop(state);
        let reason = match end {
            LinkEnd::Timeout | LinkEnd::Remote(Some(CloseCode::Timeout), _) => Disconnect::Timeout,
            LinkEnd::Remote(Some(CloseCode::Shutdown), _) => Disconnect::PeerShutdown,
            LinkEnd::Remote(Some(CloseCode::Sleep), _) => Disconnect::PeerSlept,
            LinkEnd::Remote(Some(CloseCode::Replaced | CloseCode::Duplicate), _) => Disconnect::Replaced,
            LinkEnd::Remote(code, reason) => {
                Disconnect::ProtocolError(format!("closed by the peer ({code:?}): {reason}"))
            }
            LinkEnd::Local => Disconnect::Shutdown,
            LinkEnd::Lost(e) => Disconnect::Lost(e),
            LinkEnd::Protocol(e) => Disconnect::ProtocolError(e),
            LinkEnd::UnpairedByPeer => unreachable!("handled above"),
        };
        let _ = self.events.send(Event::Disconnected { peer, reason });
        self.changed.send_replace(());
    }
}

async fn accept_loop(inner: Arc<Inner>) {
    while let Some(incoming) = inner.endpoint.accept().await {
        let inner = inner.clone();
        tokio::spawn(async move {
            let Ok(connection) = incoming.await else {
                return;
            };
            match tls::alpn(&connection).as_deref() {
                Some(ALPN_LINK) => {
                    run_link(inner, connection, false).await;
                }
                Some(ALPN_PAIR) => serve_pairing(inner, connection).await,
                _ => CloseCode::ProtocolError.close(&connection, "unknown protocol"),
            }
        });
    }
}

/// Sets up and runs a link on an established connection. `true` if it became the peer's link.
async fn run_link(inner: Arc<Inner>, connection: Connection, dialed: bool) -> bool {
    let address = connection.remote_address();
    let Some(peer) = tls::peer_fingerprint(&connection) else {
        CloseCode::ProtocolError.close(&connection, "no certificate");
        return false;
    };
    if !inner.lock().store.contains(&peer) {
        CloseCode::NotPaired.close(&connection, "not paired");
        if !dialed {
            inner.refuse(None, address, Refusal::NotPaired);
        }
        return false;
    }
    let id = inner.next_link_id.fetch_add(1, Ordering::Relaxed);
    let stats = Arc::new(Mutex::new(LinkStats::new()));
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    // Only the registered entry may hold the sender: when the node drops it, the link loop ends
    let mut out_tx = Some(out_tx);
    let mut registered = false;
    let mut register = |established: &link::Established| {
        let entry = LinkEntry {
            id,
            connection: connection.clone(),
            outgoing: out_tx.take().expect("registered once"),
            dialer: if dialed { inner.me() } else { peer },
            peer_instance: established.instance,
            stats: stats.clone(),
            since: Instant::now(),
            name: established.name.clone(),
            screens: established.screens.clone(),
            app_version: established.app_version.clone(),
            blob: None,
        };
        registered = inner.register(peer, entry, address);
        if !registered {
            CloseCode::Duplicate.close(&connection, "already connected");
        }
        registered
    };
    let hello = inner.hello();
    let result = link::setup(&connection, dialed, inner.instance, &hello, &mut register).await;
    let result = match result {
        Ok(established) if dialed && !register(&established) => return false,
        other => other,
    };
    drop(register);
    match result {
        Ok(established) => {
            let host: Arc<dyn LinkHost> = inner.clone();
            let end = link::run(connection.clone(), established, peer, stats, out_rx, host).await;
            inner.unregister(peer, id, end);
            true
        }
        Err(error) => {
            if registered {
                inner.unregister(peer, id, LinkEnd::Lost("setup failed".into()));
            }
            match error {
                SetupError::Version(theirs) => inner.refuse(
                    Some(peer),
                    address,
                    Refusal::VersionMismatch {
                        ours: PROTOCOL_VERSION,
                        theirs,
                    },
                ),
                SetupError::Closed(Some(CloseCode::VersionMismatch), reason) => inner.refuse(
                    Some(peer),
                    address,
                    Refusal::VersionMismatch {
                        ours: PROTOCOL_VERSION,
                        theirs: version_from_reason(&reason),
                    },
                ),
                SetupError::Closed(Some(CloseCode::NotPaired), _) if dialed => inner.unpaired_by_peer(peer),
                SetupError::Protocol(what) => inner.refuse(Some(peer), address, Refusal::ProtocolError(what.into())),
                SetupError::Closed(..) | SetupError::Lost | SetupError::Rejected => {}
            }
            false
        }
    }
}

/// Close reasons for version mismatches read "protocol version N".
fn version_from_reason(reason: &str) -> u16 {
    reason.rsplit(' ').next().and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn start_supervisor(inner: &Arc<Inner>, peer: Fingerprint) {
    if !inner.config.dial {
        return;
    }
    let mut state = inner.lock();
    if state.shut_down || state.supervisors.get(&peer).is_some_and(|t| !t.is_finished()) {
        return;
    }
    state
        .supervisors
        .insert(peer, tokio::spawn(supervise(inner.clone(), peer)));
}

/// Keeps a link to `peer` alive: dials whenever there is none, with backoff.
async fn supervise(inner: Arc<Inner>, peer: Fingerprint) {
    let mut changed = inner.changed.subscribe();
    let mut woke = inner.woke.subscribe();
    let mut backoff = BACKOFF_MIN;
    let mut gave_head_start = false;
    loop {
        changed.borrow_and_update();
        let discovered = inner.discovery.as_ref().map(Discovery::found).unwrap_or_default();
        let candidates = {
            let state = inner.lock();
            if state.shut_down {
                return;
            }
            let Some(paired) = state.store.get(&peer) else {
                return;
            };
            (!state.links.contains_key(&peer)).then(|| candidates(paired, &discovered))
        };
        let Some(candidates) = candidates else {
            // Connected: wait until that changes
            gave_head_start = false;
            if changed.changed().await.is_err() {
                return;
            }
            continue;
        };
        if !gave_head_start && inner.me() > peer {
            gave_head_start = true;
            tokio::select! {
                _ = sleep(HEAD_START) => {}
                result = changed.changed() => if result.is_err() { return },
            }
            continue;
        }
        if !candidates.is_empty()
            && let Some(connection) = dial(&inner, peer, &candidates).await
            && run_link(inner.clone(), connection, true).await
        {
            backoff = BACKOFF_MIN;
            continue;
        }
        let mut reset = false;
        tokio::select! {
            _ = sleep(backoff) => {}
            _ = woke.changed() => reset = true,
            result = changed.changed() => if result.is_err() { return },
        }
        backoff = if reset {
            BACKOFF_MIN
        } else {
            (backoff * 2).min(BACKOFF_MAX)
        };
    }
}

/// Remembered addresses first (latest success first), then what discovery found.
fn candidates(paired: &PairedPeer, discovered: &[Discovered]) -> Vec<SocketAddr> {
    let mut list = paired.addresses.clone();
    for found in discovered.iter().filter(|d| d.fingerprint == paired.fingerprint) {
        for address in &found.addresses {
            if !list.contains(address) {
                list.push(*address);
            }
        }
    }
    list
}

/// Tries all addresses, staggered; the first connection to the right device wins.
async fn dial(inner: &Arc<Inner>, peer: Fingerprint, candidates: &[SocketAddr]) -> Option<Connection> {
    let mut attempts = JoinSet::new();
    for (i, &address) in candidates.iter().enumerate() {
        let inner = inner.clone();
        attempts.spawn(async move {
            sleep(DIAL_STAGGER * i as u32).await;
            let connecting = inner
                .endpoint
                .connect_with(inner.link_client.clone(), address, CERT_NAME)
                .ok()?;
            let connection = connecting.await.ok()?;
            if tls::peer_fingerprint(&connection) == Some(peer) {
                Some(connection)
            } else {
                CloseCode::WrongPeer.close(&connection, "unexpected device");
                inner.refuse(Some(peer), address, Refusal::WrongPeer);
                None
            }
        });
    }
    timeout(DIAL_TIMEOUT, async {
        while let Some(result) = attempts.join_next().await {
            if let Ok(Some(connection)) = result {
                return Some(connection);
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

fn store_pairing(
    inner: &Arc<Inner>,
    peer: Fingerprint,
    name: &str,
    address: SocketAddr,
) -> Result<PairedPeer, PairError> {
    let mut state = inner.lock();
    state
        .store
        .insert(peer, name, Some(address))
        .map_err(|e| PairError::Store(e.to_string()))?;
    state.refusals.remove(&peer);
    let paired = state.store.get(&peer).cloned().expect("just inserted");
    drop(state);
    let _ = inner.events.send(Event::Paired { peer: paired.clone() });
    start_supervisor(inner, peer);
    inner.changed.send_replace(());
    Ok(paired)
}

/// The typing side of pairing.
async fn pair_with(inner: &Arc<Inner>, addresses: &[SocketAddr], code: &PairingCode) -> Result<PairedPeer, PairError> {
    if addresses.is_empty() {
        return Err(PairError::NoAddress);
    }
    let mut attempts = JoinSet::new();
    for (i, &address) in addresses.iter().enumerate() {
        let endpoint = inner.endpoint.clone();
        let config = inner.pair_client.clone();
        attempts.spawn(async move {
            sleep(DIAL_STAGGER * i as u32).await;
            let connection = endpoint.connect_with(config, address, CERT_NAME).ok()?.await.ok()?;
            Some((connection, address))
        });
    }
    let connected = timeout(DIAL_TIMEOUT, async {
        while let Some(result) = attempts.join_next().await {
            if let Ok(Some(found)) = result {
                return Some(found);
            }
        }
        None
    })
    .await;
    drop(attempts);
    let Ok(Some((connection, address))) = connected else {
        return Err(PairError::Unreachable);
    };
    match timeout(SETUP_TIMEOUT * 2, pair_exchange(inner, &connection, address, code)).await {
        Ok(result) => result,
        Err(_) => {
            CloseCode::Timeout.close(&connection, "pairing took too long");
            Err(PairError::Protocol("no answer".into()))
        }
    }
}

async fn pair_exchange(
    inner: &Arc<Inner>,
    connection: &Connection,
    address: SocketAddr,
    code: &PairingCode,
) -> Result<PairedPeer, PairError> {
    let closed = |connection: &Connection| match connection.close_reason().as_ref().and_then(remote_close) {
        Some((Some(CloseCode::PairingClosed), _)) => PairError::NotShowingCode,
        Some((Some(CloseCode::WrongCode), _)) => PairError::WrongCode,
        Some((Some(CloseCode::VersionMismatch), reason)) => PairError::VersionMismatch {
            ours: PROTOCOL_VERSION,
            theirs: version_from_reason(&reason),
        },
        Some((_, reason)) => PairError::Protocol(reason),
        None => PairError::Unreachable,
    };
    let protocol_error = |what: &str| {
        CloseCode::ProtocolError.close(connection, what);
        PairError::Protocol(what.into())
    };
    let showing = tls::peer_fingerprint(connection).ok_or_else(|| protocol_error("no certificate"))?;
    let (mut send, mut recv) = connection.open_bi().await.map_err(|_| closed(connection))?;
    let exchange = Exchange::typing(code, inner.me(), showing);
    let start = PairMsg::Start {
        version: PROTOCOL_VERSION,
        name: inner.config.name.clone(),
        spake: exchange.outgoing.clone(),
    };
    write_frame(&mut send, &start.encode())
        .await
        .map_err(|_| closed(connection))?;
    let frame = read_frame(&mut recv)
        .await
        .ok()
        .flatten()
        .ok_or_else(|| closed(connection))?;
    let Some(PairMsg::Reply { name, spake, confirm }) = PairMsg::decode(&frame) else {
        return Err(protocol_error("unexpected pairing message"));
    };
    let keys = exchange
        .finish(&spake)
        .ok_or_else(|| protocol_error("invalid key exchange"))?;
    if !keys.check_showing(&confirm) {
        CloseCode::WrongCode.close(connection, "wrong code");
        return Err(PairError::WrongCode);
    }
    let confirm = PairMsg::Confirm {
        confirm: keys.typing_proof(),
    };
    write_frame(&mut send, &confirm.encode())
        .await
        .map_err(|_| closed(connection))?;
    // The showing side stores us and then closes with `Paired`; only then do we store it
    let _ = connection.closed().await;
    match connection.close_reason().as_ref().and_then(remote_close) {
        Some((Some(CloseCode::Paired), _)) => store_pairing(inner, showing, &name, address),
        _ => Err(closed(connection)),
    }
}

/// The showing side of pairing.
async fn serve_pairing(inner: Arc<Inner>, connection: Connection) {
    let address = connection.remote_address();
    let work = async {
        let typing = tls::peer_fingerprint(&connection)?;
        let (mut send, mut recv) = connection.accept_bi().await.ok()?;
        let frame = read_frame(&mut recv).await.ok()??;
        let Some(PairMsg::Start { version, name, spake }) = PairMsg::decode(&frame) else {
            CloseCode::ProtocolError.close(&connection, "unexpected pairing message");
            return None;
        };
        if version != PROTOCOL_VERSION {
            CloseCode::VersionMismatch.close(&connection, &format!("protocol version {PROTOCOL_VERSION}"));
            inner.refuse(
                None,
                address,
                Refusal::VersionMismatch {
                    ours: PROTOCOL_VERSION,
                    theirs: version,
                },
            );
            return None;
        }
        let code = inner.lock().pairing.as_mut().and_then(PairingWindow::take_attempt);
        let Some(code) = code else {
            CloseCode::PairingClosed.close(&connection, "no pairing code shown");
            return None;
        };
        let exchange = Exchange::showing(&code, typing, inner.me());
        let outgoing = exchange.outgoing.clone();
        let Some(keys) = exchange.finish(&spake) else {
            CloseCode::ProtocolError.close(&connection, "invalid key exchange");
            pairing_attempt_failed(&inner);
            return None;
        };
        let reply = PairMsg::Reply {
            name: inner.config.name.clone(),
            spake: outgoing,
            confirm: keys.showing_proof(),
        };
        write_frame(&mut send, &reply.encode()).await.ok()?;
        let confirmed = match read_frame(&mut recv).await {
            Ok(Some(frame)) => {
                matches!(PairMsg::decode(&frame), Some(PairMsg::Confirm { confirm }) if keys.check_typing(&confirm))
            }
            _ => false,
        };
        if !confirmed {
            CloseCode::WrongCode.close(&connection, "wrong code");
            pairing_attempt_failed(&inner);
            return None;
        }
        inner.lock().pairing = None;
        inner.announce(false);
        match store_pairing(&inner, typing, &name, address) {
            Ok(_) => CloseCode::Paired.close(&connection, "paired"),
            Err(e) => CloseCode::ProtocolError.close(&connection, &e.to_string()),
        }
        Some(())
    };
    if timeout(SETUP_TIMEOUT * 2, work).await.is_err() {
        CloseCode::Timeout.close(&connection, "pairing took too long");
    }
}

fn pairing_attempt_failed(inner: &Arc<Inner>) {
    let mut state = inner.lock();
    let Some(attempts_left) = state.pairing.as_ref().map(|w| w.attempts_left) else {
        return;
    };
    if attempts_left == 0 {
        state.pairing = None;
    }
    drop(state);
    let _ = inner.events.send(Event::PairingFailed { attempts_left });
    if attempts_left == 0 {
        inner.announce(false);
        let _ = inner.events.send(Event::PairingClosed);
    }
}

/// Notices sleep by a jump of the clocks between one-second ticks. Afterwards every link is stale
/// (the peer gave up on it long ago), so all are closed and redialed at once instead of waiting for
/// the timeout.
async fn watch_sleep(inner: Arc<Inner>) {
    let mut last_mono = Instant::now();
    let mut last_wall = SystemTime::now();
    loop {
        sleep(Duration::from_secs(1)).await;
        let mono = last_mono.elapsed();
        let wall = SystemTime::now().duration_since(last_wall).unwrap_or_default();
        last_mono = Instant::now();
        last_wall = SystemTime::now();
        let gap = mono.max(wall);
        if gap >= SLEEP_GAP {
            woke_up(&inner, gap);
        }
    }
}

fn woke_up(inner: &Arc<Inner>, slept: Duration) {
    tracing::info!("woke from sleep after about {} s", slept.as_secs());
    let links: Vec<_> = inner.lock().links.drain().collect();
    for (peer, entry) in links {
        CloseCode::Sleep.close(&entry.connection, "sleep");
        let _ = inner.events.send(Event::Disconnected {
            peer,
            reason: Disconnect::Slept,
        });
    }
    let _ = inner.events.send(Event::Woke { slept });
    inner.woke.send_modify(|n| *n += 1);
    inner.changed.send_replace(());
}

/// How long a port still held by a node that is shutting down (restart after a settings change)
/// is waited for.
const BIND_PATIENCE: Duration = Duration::from_secs(5);

/// Like `bind`, but tries again for a while if the port is in use.
async fn bind_patiently(address: SocketAddr) -> io::Result<std::net::UdpSocket> {
    let started = Instant::now();
    loop {
        match bind(address) {
            Err(e) if e.kind() == io::ErrorKind::AddrInUse && started.elapsed() < BIND_PATIENCE => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
                return Err(io::Error::new(
                    e.kind(),
                    format!("port {} is used by another program", address.port()),
                ));
            }
            other => {
                if other.is_ok() && started.elapsed() > Duration::from_millis(150) {
                    tracing::info!(
                        "port {} free after {} ms",
                        address.port(),
                        started.elapsed().as_millis()
                    );
                }
                return other;
            }
        }
    }
}

/// Binds the UDP socket; `[::]` accepts IPv4 as well. Falls back to IPv4 only without IPv6.
fn bind(address: SocketAddr) -> io::Result<std::net::UdpSocket> {
    match bind_once(address) {
        Err(e) if address.ip() == Ipv6Addr::UNSPECIFIED && e.kind() != io::ErrorKind::AddrInUse => {
            tracing::info!("IPv6 is not available ({e}), using IPv4 only");
            bind_once(SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), address.port()))
        }
        other => other,
    }
}

fn bind_once(address: SocketAddr) -> io::Result<std::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::for_address(address), Type::DGRAM, Some(Protocol::UDP))?;
    if address.is_ipv6() {
        socket.set_only_v6(false)?;
    }
    socket.bind(&address.into())?;
    Ok(socket.into())
}
