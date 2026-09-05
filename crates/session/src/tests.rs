//! Two complete sessions in one process, controlled only through commands, with virtual devices.

use std::path::PathBuf;
use std::time::Duration;

use tc_core::{Captured, Point, Side};
use tc_input::{Backend, VirtualDevice, virtual_backend};
use tc_net::{Identity, Node, NodeConfig, PeerStore};
use tc_protocol::{Message, Screen};

use std::sync::Arc;

use tc_clipboard::{MemoryClipboard, Read};
use tc_protocol::ClipContent;

use crate::{Clipboard, Command, Notice, PairTarget, Session, SessionHandle, Skip};

fn screen() -> Screen {
    Screen {
        x: 0,
        y: 0,
        width: 1000,
        height: 800,
        scale: 1.0,
    }
}

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tablecross-session-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Machine {
    session: SessionHandle,
    device: VirtualDevice,
    node: Node,
    clipboard: Arc<MemoryClipboard>,
}

async fn machine(name: &str) -> Machine {
    let mut config = NodeConfig::new(name);
    config.bind = "[::]:0".parse().unwrap();
    config.discovery = false;
    config.screens = vec![screen()];
    let (node, events) = Node::start(config, Identity::generate().unwrap(), PeerStore::in_memory())
        .await
        .unwrap();
    let (capture, inject, device) = virtual_backend(vec![screen()]);
    let backend = Backend {
        capture: Box::new(capture),
        inject,
    };
    let clipboard = Arc::new(MemoryClipboard::new());
    let shared = Clipboard {
        system: clipboard.clone(),
        max_size: 4 * 1024 * 1024,
    };
    let (session, _task) = Session::with_node(node.clone(), events, dir(name), Some(backend), Some(shared), None)
        .await
        .unwrap();
    Machine {
        session,
        device,
        node,
        clipboard,
    }
}

/// Waits until `check` holds, up to five seconds.
async fn until(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("not reached: {what}");
}

#[tokio::test(flavor = "multi_thread")]
async fn pair_place_and_share_through_commands() {
    let a = machine("alpha").await;
    let b = machine("beta").await;

    // Pairing: Beta shows a code, Alpha types it
    b.session.send(Command::StartPairing);
    until("code shown", || b.session.status().pairing.is_some()).await;
    let code = b.session.status().pairing.unwrap().0;
    let b_addr = std::net::SocketAddr::from(([127, 0, 0, 1], b.node.local_addr().port()));
    let mut a_notices = a.session.notices();
    a.session.send(Command::Pair {
        code: code.digits().to_string(),
        target: PairTarget::Address(b_addr),
    });
    until("paired on both sides", || {
        a.session.status().peers.len() == 1 && b.session.status().peers.len() == 1
    })
    .await;
    until("connected", || a.session.status().peers[0].link.is_some()).await;
    assert!(b.session.status().pairing.is_none());
    let mut saw_paired = false;
    while let Ok(n) = a_notices.try_recv() {
        saw_paired |= matches!(n, Notice::Paired(_));
    }
    assert!(saw_paired);

    // Beta stands right of Alpha (both sides say so)
    let (a_fp, b_fp) = (a.node.fingerprint(), b.node.fingerprint());
    a.session.send(Command::Place {
        peer: b_fp,
        side: Some(Side::Right),
    });
    b.session.send(Command::Place {
        peer: a_fp,
        side: Some(Side::Left),
    });
    until("placed", || {
        a.session.status().peers[0].side == Some(Side::Right) && b.session.status().peers[0].side == Some(Side::Left)
    })
    .await;

    // Over the edge, typing, moving
    let crossing = Captured::Motion {
        x: 999.0,
        y: 400.0,
        dx: 5.0,
        dy: 0.0,
    };
    assert!(!a.device.send(crossing));
    until("Alpha controls Beta", || a.session.status().controlling.is_some()).await;
    until("Beta entered", || {
        b.device.injected().iter().any(|m| matches!(m, Message::Enter { .. }))
    })
    .await;
    assert!(!a.device.send(Captured::Key {
        usage: 0x04,
        down: true
    }));
    assert!(!a.device.send(Captured::Key {
        usage: 0x04,
        down: false
    }));
    until("keys arrive", || {
        b.device
            .injected()
            .iter()
            .filter(|m| matches!(m, Message::Key { usage: 0x04, .. }))
            .count()
            == 2
    })
    .await;
    until("Beta shows its controller", || {
        b.session.status().controlled_by.is_some()
    })
    .await;

    // Ctrl+Shift+Alt+Left brings the pointer home
    for usage in [0xE0, 0xE1, 0xE2] {
        a.device.send(Captured::Key { usage, down: true });
    }
    assert!(
        !a.device.send(Captured::Key {
            usage: 0x50,
            down: true
        }),
        "the shortcut is not forwarded"
    );
    until("back home", || !a.device.grabbed()).await;
    assert_eq!(a.device.pointer(), Point::new(500.0, 400.0));
    for usage in [0x50, 0xE0, 0xE1, 0xE2] {
        a.device.send(Captured::Key { usage, down: false });
    }

    // Switching by command, then Beta's own mouse takes over
    a.session.send(Command::SwitchTo(Some(b_fp)));
    until("grabbed again", || a.device.grabbed()).await;
    b.device.send(Captured::Motion {
        x: 500.0,
        y: 500.0,
        dx: 20.0,
        dy: 0.0,
    });
    until("Alpha let go", || !a.device.grabbed()).await;

    // Unpairing removes it on both sides and from the layout
    a.session.send(Command::Unpair(b_fp));
    until("unpaired", || {
        a.session.status().peers.is_empty() && b.session.status().peers.is_empty()
    })
    .await;
    until("layout cleaned", || a.session.status().layout.neighbors.is_empty()).await;

    a.session.send(Command::Shutdown);
    b.session.send(Command::Shutdown);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_code_is_reported() {
    let a = machine("gamma").await;
    let b = machine("delta").await;
    b.session.send(Command::StartPairing);
    until("code shown", || b.session.status().pairing.is_some()).await;
    let b_addr = std::net::SocketAddr::from(([127, 0, 0, 1], b.node.local_addr().port()));
    let shown = b.session.status().pairing.unwrap().0;
    let wrong = if shown.digits() == "111111" { "222222" } else { "111111" };
    let mut notices = a.session.notices();
    a.session.send(Command::Pair {
        code: wrong.into(),
        target: PairTarget::Address(b_addr),
    });
    let notice = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Notice::PairError(e)) = notices.recv().await {
                return e;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(notice, tc_net::PairError::WrongCode);
    until("Beta counts the attempt", || {
        b.session
            .status()
            .notices
            .iter()
            .any(|(_, n)| matches!(n, Notice::PairingFailed(_)))
    })
    .await;
    assert!(a.session.status().peers.is_empty());
}

async fn paired_pair(a_name: &str, b_name: &str) -> (Machine, Machine) {
    let a = machine(a_name).await;
    let b = machine(b_name).await;
    b.session.send(Command::StartPairing);
    until("code shown", || b.session.status().pairing.is_some()).await;
    let code = b.session.status().pairing.unwrap().0;
    let b_addr = std::net::SocketAddr::from(([127, 0, 0, 1], b.node.local_addr().port()));
    a.session.send(Command::Pair {
        code: code.digits().to_string(),
        target: PairTarget::Address(b_addr),
    });
    until("connected", || {
        a.session.status().peers.first().is_some_and(|p| p.link.is_some())
            && b.session.status().peers.first().is_some_and(|p| p.link.is_some())
    })
    .await;
    // Let both clipboard watchers take their first look
    tokio::time::sleep(Duration::from_millis(400)).await;
    (a, b)
}

#[tokio::test(flavor = "multi_thread")]
async fn clipboard_follows_in_both_directions() {
    let (a, b) = paired_pair("clip-a", "clip-b").await;
    let text = ClipContent {
        text: Some("Grüße aus Alpha".into()),
        html: Some("<b>Grüße</b>".into()),
        ..ClipContent::default()
    };
    a.clipboard.set(Read::Content(text.clone()));
    until("text arrives on Beta", || {
        b.clipboard.get() == Read::Content(text.clone())
    })
    .await;

    // And back, with an image larger than what is sent along at once
    let image = ClipContent {
        png: Some((0..2_000_000u32).map(|i| (i % 253) as u8).collect()),
        ..ClipContent::default()
    };
    b.clipboard.set(Read::Content(image.clone()));
    // Alpha is the computer in use (not controlling anyone), so it fetches it right away
    until("image arrives on Alpha", || {
        a.clipboard.get() == Read::Content(image.clone())
    })
    .await;

    // Nothing bounces back and forth
    let (count_a, count_b) = (
        tc_clipboard::SystemClipboard::change_count(a.clipboard.as_ref()),
        tc_clipboard::SystemClipboard::change_count(b.clipboard.as_ref()),
    );
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        tc_clipboard::SystemClipboard::change_count(a.clipboard.as_ref()),
        count_a
    );
    assert_eq!(
        tc_clipboard::SystemClipboard::change_count(b.clipboard.as_ref()),
        count_b
    );

    // Passwords stay where they are
    let before = b.clipboard.get();
    a.clipboard.set(Read::Concealed);
    until("noticed as private", || {
        a.session
            .status()
            .notices
            .iter()
            .any(|(_, n)| *n == Notice::ClipboardSkipped(Skip::Concealed))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(b.clipboard.get(), before);
    a.session.send(Command::Shutdown);
    b.session.send(Command::Shutdown);
}
