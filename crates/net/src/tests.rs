//! Two or more nodes in one process, talking over localhost.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tc_protocol::{Message, PROTOCOL_VERSION};
use tokio::net::UdpSocket;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::timeout;

use crate::link::PEER_TIMEOUT;
use crate::tls::{ALPN_LINK, ALPN_PAIR, client_config};
use crate::wire::{CloseCode, Preamble, read_frame, write_frame};
use crate::*;

struct Peer {
    node: Node,
    events: UnboundedReceiver<Event>,
    identity: Identity,
}

impl Peer {
    async fn start(name: &str, dial: bool) -> Self {
        Self::start_with(name, dial, Identity::generate().unwrap(), PeerStore::in_memory()).await
    }

    async fn start_with(name: &str, dial: bool, identity: Identity, store: PeerStore) -> Self {
        let mut config = NodeConfig::new(name);
        config.bind = "[::]:0".parse().unwrap();
        config.discovery = false;
        config.dial = dial;
        let (node, events) = Node::start(config, identity.clone(), store).await.unwrap();
        Self { node, events, identity }
    }

    /// Where other nodes in this process reach it.
    fn addr(&self) -> SocketAddr {
        local(self.node.local_addr())
    }

    fn fp(&self) -> Fingerprint {
        self.node.fingerprint()
    }

    /// Waits for the first event that `pick` accepts, skipping others.
    async fn expect<T>(&mut self, within: Duration, mut pick: impl FnMut(&Event) -> Option<T>) -> T {
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match timeout(left, self.events.recv()).await {
                Ok(Some(event)) => {
                    if let Some(found) = pick(&event) {
                        return found;
                    }
                }
                Ok(None) => panic!("{}: event channel closed", self.node.name()),
                Err(_) => panic!("{}: expected event did not arrive within {within:?}", self.node.name()),
            }
        }
    }

    async fn connected(&mut self, peer: Fingerprint, within: Duration) {
        self.expect(within, |e| match e {
            Event::Connected { peer: p, .. } if *p == peer => Some(()),
            _ => None,
        })
        .await
    }

    async fn disconnected(&mut self, peer: Fingerprint, within: Duration) -> Disconnect {
        self.expect(within, |e| match e {
            Event::Disconnected { peer: p, reason } if *p == peer => Some(reason.clone()),
            _ => None,
        })
        .await
    }

    /// Collects everything that arrives within `period`.
    async fn drain(&mut self, period: Duration) -> Vec<Event> {
        let mut out = Vec::new();
        let deadline = Instant::now() + period;
        while let Ok(Some(event)) =
            timeout(deadline.saturating_duration_since(Instant::now()), self.events.recv()).await
        {
            out.push(event);
        }
        out
    }
}

fn local(addr: SocketAddr) -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), addr.port())
}

const SHORT: Duration = Duration::from_secs(5);

/// Pairs `typing` with `showing` and waits until both report it.
async fn pair(typing: &mut Peer, showing: &mut Peer) {
    let code = showing.node.start_pairing();
    typing.node.pair(&[showing.addr()], &code).await.unwrap();
    let fp = typing.fp();
    showing
        .expect(SHORT, |e| {
            matches!(e, Event::Paired { peer } if peer.fingerprint == fp).then_some(())
        })
        .await;
}

#[tokio::test]
async fn pairing_connects_and_messages_flow() {
    let mut a = Peer::start("Alpha", true).await;
    let mut b = Peer::start("Beta", true).await;
    pair(&mut b, &mut a).await;
    assert_eq!(a.node.paired()[0].name, "Beta");
    assert_eq!(b.node.paired()[0].name, "Alpha");
    assert_eq!(b.node.paired()[0].addresses, vec![a.addr()]);

    a.connected(b.fp(), SHORT).await;
    b.connected(a.fp(), SHORT).await;

    // Ordered stream
    for usage in 4..40 {
        assert!(a.node.send(&b.fp(), Message::Key { usage, down: true }));
    }
    for usage in 4..40 {
        let got = b
            .expect(SHORT, |e| match e {
                Event::Message { message, .. } => Some(message.clone()),
                _ => None,
            })
            .await;
        assert_eq!(got, Message::Key { usage, down: true });
    }
    // Datagram
    let motion = Message::MouseMove {
        seq: 1,
        x: 10.0,
        y: 20.0,
        dx: 2.5,
        dy: -1.0,
    };
    assert!(b.node.send(&a.fp(), motion.clone()));
    let got = a
        .expect(SHORT, |e| match e {
            Event::Message { message, .. } => Some(message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(got, motion);

    // Bulk data on its own stream, larger than any message
    let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    assert!(a.node.send_blob(&b.fp(), b"clip".to_vec(), data.clone()));
    let (header, got) = b
        .expect(SHORT, |e| match e {
            Event::Blob { header, data, .. } => Some((header.clone(), data.clone())),
            _ => None,
        })
        .await;
    assert_eq!(header, b"clip");
    assert_eq!(got, data);

    // A stream of unknown length, read piece by piece
    let mut out = a.node.open_stream(&b.fp(), b"files").await.unwrap();
    let send_task = tokio::spawn(async move {
        for _ in 0..50 {
            out.write_all(&[7u8; 100_000]).await.unwrap();
        }
        out.finish().await.unwrap();
    });
    let slot = b
        .expect(SHORT, |e| match e {
            Event::Stream { header, stream, .. } if header == b"files" => Some(stream.clone()),
            _ => None,
        })
        .await;
    let mut stream = slot.take().unwrap();
    assert!(slot.take().is_none(), "taken once");
    let mut total = 0;
    let mut buf = vec![0; 65536];
    loop {
        let n = stream.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        assert!(buf[..n].iter().all(|b| *b == 7));
        total += n;
    }
    assert_eq!(total, 5_000_000);
    send_task.await.unwrap();

    // Cancelled by the receiver: the sender notices
    let mut out = a.node.open_stream(&b.fp(), b"cancel").await.unwrap();
    out.write_all(&[1u8; 10]).await.unwrap();
    let slot = b
        .expect(SHORT, |e| match e {
            Event::Stream { header, stream, .. } if header == b"cancel" => Some(stream.clone()),
            _ => None,
        })
        .await;
    slot.take().unwrap().cancel();
    let mut failed = false;
    for _ in 0..200 {
        if out.write_all(&[1u8; 100_000]).await.is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed, "writing after the cancel fails");

    // Latency probes run on their own
    tokio::time::sleep(Duration::from_millis(1600)).await;
    let status = a.node.status();
    let link = status[0].link.as_ref().expect("connected");
    assert!(link.rtt.is_some(), "{link:?}");
    assert!(link.probes_sent >= 2);
    assert_eq!(link.name, "Beta");
    assert!(!a.node.take_rtt_samples(&b.fp()).is_empty());

    // Exactly one link per peer, both sides agree and nothing flaps
    let events = a.drain(Duration::from_secs(2)).await;
    assert!(
        !events.iter().any(|e| matches!(e, Event::Disconnected { .. })),
        "{events:?}"
    );
    a.node.shutdown().await;
    assert_eq!(b.disconnected(a.fp(), SHORT).await, Disconnect::PeerShutdown);
    b.node.shutdown().await;
}

#[tokio::test]
async fn wrong_code_is_refused_and_attempts_are_limited() {
    let mut a = Peer::start("Alpha", true).await;
    let b = Peer::start("Beta", true).await;
    let code = a.node.start_pairing();
    let wrong = PairingCode::parse(if code.digits() == "000000" { "111111" } else { "000000" }).unwrap();
    for left in (0..MAX_ATTEMPTS).rev() {
        assert_eq!(b.node.pair(&[a.addr()], &wrong).await, Err(PairError::WrongCode));
        let reported = a
            .expect(SHORT, |e| match e {
                Event::PairingFailed { attempts_left } => Some(*attempts_left),
                _ => None,
            })
            .await;
        assert_eq!(reported, left);
    }
    // Code is void now, even the right one fails
    assert_eq!(b.node.pair(&[a.addr()], &code).await, Err(PairError::NotShowingCode));
    assert!(a.node.paired().is_empty());
    assert!(b.node.paired().is_empty());
    // A fresh code works again
    let code = a.node.start_pairing();
    b.node.pair(&[a.addr()], &code).await.unwrap();
    assert_eq!(a.node.paired().len(), 1);
}

#[tokio::test]
async fn no_pairing_without_shown_code() {
    let a = Peer::start("Alpha", true).await;
    let b = Peer::start("Beta", true).await;
    let code = PairingCode::random();
    assert_eq!(b.node.pair(&[a.addr()], &code).await, Err(PairError::NotShowingCode));
    let unused: SocketAddr = "127.0.0.1:9".parse().unwrap();
    assert_eq!(b.node.pair(&[unused], &code).await, Err(PairError::Unreachable));
    assert_eq!(b.node.pair(&[], &code).await, Err(PairError::NoAddress));
}

#[tokio::test]
async fn foreign_certificate_is_refused() {
    let mut a = Peer::start("Alpha", true).await;
    let mut b = Peer::start("Beta", true).await;
    pair(&mut b, &mut a).await;
    a.connected(b.fp(), SHORT).await;

    // A third device that knows A's address but was never paired
    let mut stranger_store = PeerStore::in_memory();
    stranger_store.insert(a.fp(), "Alpha", Some(a.addr())).unwrap();
    let mut stranger = Peer::start_with("Stranger", true, Identity::generate().unwrap(), stranger_store).await;
    let address = a
        .expect(SHORT, |e| match e {
            Event::Refused {
                peer: None,
                address,
                reason: Refusal::NotPaired,
            } => Some(*address),
            _ => None,
        })
        .await;
    assert_eq!(address, stranger.addr());
    // The stranger learns that A does not know it and drops A
    let by_peer = stranger
        .expect(SHORT, |e| match e {
            Event::Unpaired { peer, by_peer } if *peer == a.fp() => Some(*by_peer),
            _ => None,
        })
        .await;
    assert!(by_peer);
    assert!(stranger.node.paired().is_empty());
    // The real link was not disturbed
    assert!(a.node.status()[0].link.is_some());
}

#[tokio::test]
async fn impostor_at_known_address_is_not_accepted() {
    let mut a = Peer::start("Alpha", true).await;
    let mut b = Peer::start("Beta", true).await;
    pair(&mut b, &mut a).await;
    let a_fp = a.fp();
    b.connected(a_fp, SHORT).await;
    a.node.shutdown().await;
    b.disconnected(a_fp, SHORT).await;
    drop(a);

    // Another device now answers at a remembered address of A
    let impostor = Peer::start("Impostor", false).await;
    b.node.add_address(&a_fp, impostor.addr()).unwrap();
    let reason = b
        .expect(SHORT, |e| match e {
            Event::Refused {
                peer: Some(p), reason, ..
            } if *p == a_fp => Some(reason.clone()),
            _ => None,
        })
        .await;
    assert_eq!(reason, Refusal::WrongPeer);
    assert!(b.node.status()[0].link.is_none());
}

/// Forwards UDP between a client and a server and can cut the line, like Wi-Fi switched off.
struct Proxy {
    addr: SocketAddr,
    cut: Arc<AtomicBool>,
}

impl Proxy {
    async fn start(server: SocketAddr) -> Self {
        let outside = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let inside = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let addr = outside.local_addr().unwrap();
        let cut = Arc::new(AtomicBool::new(false));
        let client = Arc::new(std::sync::Mutex::new(None::<SocketAddr>));
        {
            let (outside, inside, cut, client) = (outside.clone(), inside.clone(), cut.clone(), client.clone());
            tokio::spawn(async move {
                let mut buf = vec![0; 65536];
                while let Ok((n, from)) = outside.recv_from(&mut buf).await {
                    *client.lock().unwrap() = Some(from);
                    if !cut.load(Ordering::SeqCst) {
                        let _ = inside.send_to(&buf[..n], server).await;
                    }
                }
            });
        }
        {
            let cut = cut.clone();
            tokio::spawn(async move {
                let mut buf = vec![0; 65536];
                while let Ok((n, _)) = inside.recv_from(&mut buf).await {
                    let to = *client.lock().unwrap();
                    if let Some(to) = to
                        && !cut.load(Ordering::SeqCst)
                    {
                        let _ = outside.send_to(&buf[..n], to).await;
                    }
                }
            });
        }
        Self { addr, cut }
    }

    fn set_cut(&self, cut: bool) {
        self.cut.store(cut, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn lost_network_is_noticed_and_link_comes_back() {
    // B only listens, A reaches it only through the proxy
    let mut a = Peer::start("Alpha", true).await;
    let mut b = Peer::start("Beta", false).await;
    pair(&mut a, &mut b).await;
    a.connected(b.fp(), SHORT).await;
    a.node.shutdown().await;
    b.disconnected(a.fp(), SHORT).await;

    let mut store = PeerStore::in_memory();
    store.insert(b.fp(), "Beta", None).unwrap();
    let proxy = Proxy::start(b.addr()).await;
    store.remember_address(&b.fp(), proxy.addr).unwrap();
    let mut a = Peer::start_with("Alpha", true, a.identity.clone(), store).await;
    a.connected(b.fp(), SHORT).await;
    b.connected(a.fp(), SHORT).await;

    proxy.set_cut(true);
    let cut_at = Instant::now();
    assert_eq!(a.disconnected(b.fp(), SHORT).await, Disconnect::Timeout);
    assert_eq!(b.disconnected(a.fp(), SHORT).await, Disconnect::Timeout);
    let noticed = cut_at.elapsed();
    assert!(noticed < PEER_TIMEOUT + Duration::from_secs(1), "{noticed:?}");

    tokio::time::sleep(Duration::from_secs(2)).await;
    proxy.set_cut(false);
    let back_at = Instant::now();
    a.connected(b.fp(), SHORT).await;
    let back = back_at.elapsed();
    assert!(back < Duration::from_secs(5), "reconnect took {back:?}");
    assert_eq!(a.node.status()[0].reconnects, 1);
}

#[tokio::test]
async fn restarted_peer_replaces_its_old_link_at_once() {
    // A dials, B1 only listens
    let mut a = Peer::start("Alpha", true).await;
    let mut b1 = Peer::start("Beta", false).await;
    pair(&mut a, &mut b1).await;
    a.connected(b1.fp(), SHORT).await;
    b1.connected(a.fp(), SHORT).await;

    // B restarts (new process, same identity) while A still believes in the old link
    let mut store = PeerStore::in_memory();
    store.insert(a.fp(), "Alpha", Some(a.addr())).unwrap();
    let started = Instant::now();
    let mut b2 = Peer::start_with("Beta", true, b1.identity.clone(), store).await;
    assert_eq!(a.disconnected(b2.fp(), SHORT).await, Disconnect::Replaced);
    a.connected(b2.fp(), SHORT).await;
    b2.connected(a.fp(), SHORT).await;
    // Much faster than waiting for the heartbeat timeout
    assert!(started.elapsed() < PEER_TIMEOUT, "{:?}", started.elapsed());
    assert_eq!(b1.disconnected(a.fp(), SHORT).await, Disconnect::Replaced);
    assert!(b2.node.status()[0].link.as_ref().unwrap().dialed);
    assert!(!a.node.status()[0].link.as_ref().unwrap().dialed);
}

#[tokio::test]
async fn both_dialing_ends_with_one_link() {
    let mut a = Peer::start("Alpha", true).await;
    let mut b = Peer::start("Beta", true).await;
    pair(&mut b, &mut a).await;
    // Both know each other's address now and both dial whenever there is no link
    a.connected(b.fp(), SHORT).await;
    b.connected(a.fp(), SHORT).await;
    for _ in 0..5 {
        // Both lose the link at the same moment and dial each other at once
        a.node.drop_links_for_tests();
        b.node.drop_links_for_tests();
        let ea = a.drain(Duration::from_secs(2)).await;
        let eb = b.drain(Duration::ZERO).await;
        let (a_status, b_status) = (a.node.status(), b.node.status());
        let a_link = a_status[0].link.as_ref().unwrap_or_else(|| panic!("A: {ea:?}"));
        let b_link = b_status[0].link.as_ref().unwrap_or_else(|| panic!("B: {eb:?}"));
        // The same connection on both sides: exactly one of them dialed it
        assert_ne!(a_link.dialed, b_link.dialed, "A: {ea:?}\nB: {eb:?}");
        // And it stays
        let later = a.drain(Duration::from_millis(1500)).await;
        assert!(
            !later.iter().any(|e| matches!(e, Event::Disconnected { .. })),
            "{later:?}"
        );
    }
}

#[tokio::test]
async fn waking_up_rebuilds_links_at_once() {
    let mut a = Peer::start("Alpha", true).await;
    let mut b = Peer::start("Beta", true).await;
    pair(&mut b, &mut a).await;
    a.connected(b.fp(), SHORT).await;
    b.connected(a.fp(), SHORT).await;
    a.drain(Duration::from_millis(300)).await;

    let woke_at = Instant::now();
    a.node.wake_for_tests();
    assert_eq!(a.disconnected(b.fp(), SHORT).await, Disconnect::Slept);
    a.expect(SHORT, |e| matches!(e, Event::Woke { .. }).then_some(())).await;
    assert_eq!(b.disconnected(a.fp(), SHORT).await, Disconnect::PeerSlept);
    a.connected(b.fp(), SHORT).await;
    b.connected(a.fp(), SHORT).await;
    assert!(woke_at.elapsed() < Duration::from_secs(1), "{:?}", woke_at.elapsed());
}

#[tokio::test]
async fn unpair_removes_both_sides() {
    let mut a = Peer::start("Alpha", true).await;
    let mut b = Peer::start("Beta", true).await;
    pair(&mut b, &mut a).await;
    a.connected(b.fp(), SHORT).await;
    b.connected(a.fp(), SHORT).await;
    a.drain(Duration::from_millis(500)).await;
    assert!(a.node.unpair(&b.fp()).unwrap());
    let a_fp = a.fp();
    b.expect(SHORT, |e| match e {
        Event::Unpaired { peer, by_peer: true } if *peer == a_fp => Some(()),
        _ => None,
    })
    .await;
    assert!(a.node.paired().is_empty());
    assert!(b.node.paired().is_empty());
    // Nobody dials anymore
    let events = a.drain(Duration::from_secs(1)).await;
    assert!(
        !events.iter().any(|e| matches!(e, Event::Connected { .. })),
        "{events:?}"
    );
}

#[tokio::test]
async fn version_mismatch_is_reported_clearly() {
    let mut a = Peer::start("Alpha", false).await;
    let identity = Identity::generate().unwrap();
    // Pair a raw client by hand
    let code = a.node.start_pairing();
    let mut helper = Peer::start_with("Old", false, identity.clone(), PeerStore::in_memory()).await;
    helper.node.pair(&[a.addr()], &code).await.unwrap();
    helper.node.shutdown().await;
    let _ = helper.drain(Duration::ZERO).await;

    let connection = raw_connect(&identity, a.addr(), ALPN_LINK).await;
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    let old = Preamble {
        version: PROTOCOL_VERSION + 1,
        instance: 1,
    };
    old.write(&mut send).await.unwrap();
    let theirs = Preamble::read(&mut recv).await.unwrap();
    assert_eq!(theirs.version, PROTOCOL_VERSION);
    let reason = a
        .expect(SHORT, |e| match e {
            Event::Refused { reason, .. } => Some(reason.clone()),
            _ => None,
        })
        .await;
    assert_eq!(
        reason,
        Refusal::VersionMismatch {
            ours: PROTOCOL_VERSION,
            theirs: PROTOCOL_VERSION + 1
        }
    );
    let closed = connection.closed().await;
    let (code, text) = crate::wire::remote_close(&closed).unwrap();
    assert_eq!(code, Some(CloseCode::VersionMismatch));
    assert_eq!(text, format!("protocol version {PROTOCOL_VERSION}"));
}

async fn raw_connect(identity: &Identity, to: SocketAddr, alpn: &[u8]) -> quinn::Connection {
    let endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
    endpoint
        .connect_with(client_config(identity, alpn), to, crate::identity::CERT_NAME)
        .unwrap()
        .await
        .unwrap()
}

/// Simple xorshift, so the tests need no extra dependency.
fn noise(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    }
}

#[tokio::test]
async fn garbage_does_not_break_the_node() {
    let mut a = Peer::start("Alpha", true).await;
    let mut b = Peer::start("Beta", true).await;
    pair(&mut b, &mut a).await;
    a.connected(b.fp(), SHORT).await;

    // Random UDP packets at the endpoint
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut next = noise(0x9E37_79B9_7F4A_7C15);
    for _ in 0..3000 {
        let len = (next() % 1400) as usize + 1;
        let mut bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        // Some of them look like QUIC long headers
        if next().is_multiple_of(2) {
            bytes[0] = 0xC0 | (bytes[0] & 0x0F);
            if bytes.len() > 5 {
                bytes[1..5].copy_from_slice(&1u32.to_be_bytes());
            }
        }
        let _ = socket.send_to(&bytes, a.addr()).await;
    }

    // A paired device that sends garbage after a valid start
    let identity = Identity::generate().unwrap();
    let code = a.node.start_pairing();
    let helper = Peer::start_with("Helper", false, identity.clone(), PeerStore::in_memory()).await;
    helper.node.pair(&[a.addr()], &code).await.unwrap();
    helper.node.shutdown().await;
    for round in 0..20u64 {
        let connection = raw_connect(&identity, a.addr(), ALPN_LINK).await;
        let (mut send, _recv) = connection.open_bi().await.unwrap();
        let mut next = noise(round + 1);
        if round.is_multiple_of(2) {
            Preamble::ours(round).write(&mut send).await.unwrap();
        }
        let garbage: Vec<u8> = (0..(next() % 200)).map(|_| next() as u8).collect();
        let _ = write_frame(&mut send, &garbage).await;
        let _ = send.write_all(&garbage).await;
        let _ = timeout(Duration::from_secs(2), connection.closed()).await;
    }
    // Garbage on the pairing protocol
    for round in 0..10u64 {
        let connection = raw_connect(&identity, a.addr(), ALPN_PAIR).await;
        if let Ok((mut send, _recv)) = connection.open_bi().await {
            let mut next = noise(round + 100);
            let garbage: Vec<u8> = (0..(next() % 300)).map(|_| next() as u8).collect();
            let _ = write_frame(&mut send, &garbage).await;
        }
        let _ = timeout(Duration::from_secs(2), connection.closed()).await;
    }

    // The real link still works
    assert!(b.node.send(&a.fp(), Message::Key { usage: 4, down: true }));
    let got = a
        .expect(SHORT, |e| match e {
            Event::Message { message, .. } => Some(message.clone()),
            _ => None,
        })
        .await;
    assert_eq!(got, Message::Key { usage: 4, down: true });
    let status = a.node.status();
    let b_status = status.iter().find(|s| s.peer.fingerprint == b.fp()).unwrap();
    assert!(b_status.link.is_some());
    let events = a.drain(Duration::ZERO).await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::Disconnected { peer, .. } if *peer == b.fp())),
        "{events:?}"
    );
}

#[tokio::test]
async fn oversized_frame_ends_only_that_link() {
    let mut a = Peer::start("Alpha", false).await;
    let identity = Identity::generate().unwrap();
    let code = a.node.start_pairing();
    let helper = Peer::start_with("Helper", false, identity.clone(), PeerStore::in_memory()).await;
    helper.node.pair(&[a.addr()], &code).await.unwrap();
    helper.node.shutdown().await;
    let peer = identity.fingerprint();

    let connection = raw_connect(&identity, a.addr(), ALPN_LINK).await;
    let (mut send, mut recv) = connection.open_bi().await.unwrap();
    Preamble::ours(7).write(&mut send).await.unwrap();
    let hello = Message::Hello {
        version: PROTOCOL_VERSION,
        name: "Raw".into(),
        screens: vec![],
    };
    write_frame(&mut send, &tc_protocol::encode(&hello)).await.unwrap();
    Preamble::read(&mut recv).await.unwrap();
    read_frame(&mut recv).await.unwrap().unwrap();
    a.connected(peer, SHORT).await;
    // Length prefix far beyond the limit
    send.write_all(&u32::MAX.to_le_bytes()).await.unwrap();
    let reason = a.disconnected(peer, SHORT).await;
    assert!(matches!(reason, Disconnect::ProtocolError(_)), "{reason:?}");
}
