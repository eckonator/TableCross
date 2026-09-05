//! `run`: shares mouse and keyboard with the paired computers, arranged by `layout.json`.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tc_core::{Action, Captured, Engine, Layout, LayoutFile, Role, Side};
use tc_input::Inject;
use tc_net::{Event, Fingerprint, Node};
use tokio::sync::mpsc::UnboundedReceiver;

const LAYOUT_FILE: &str = "layout.json";
const ESCAPE: u16 = 0x29;
/// Ctrl, Shift and Alt (either side) together with Escape releases everything.
const EMERGENCY: [(u16, u16); 3] = [(0xE0, 0xE4), (0xE1, 0xE5), (0xE2, 0xE6)];

pub fn load_layout_file(dir: &Path) -> Result<LayoutFile> {
    let path = dir.join(LAYOUT_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("{} is not valid", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(LayoutFile::default()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

pub fn save_layout_file(dir: &Path, file: &LayoutFile) -> Result<()> {
    let path = dir.join(LAYOUT_FILE);
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, serde_json::to_vec_pretty(file)?).with_context(|| format!("cannot write {}", path.display()))
}

/// Machine names in the layout file → ids (fingerprints as text).
fn resolver(node: &Node) -> impl Fn(&str) -> Option<String> {
    let me = (node.name().to_lowercase(), node.fingerprint().to_string());
    let peers: Vec<_> = node
        .paired()
        .into_iter()
        .map(|p| (p.name.to_lowercase(), p.fingerprint.to_string()))
        .collect();
    move |name: &str| {
        let name = name.to_lowercase();
        std::iter::once(&me)
            .chain(peers.iter())
            .find(|(n, fp)| *n == name || (name.len() >= 8 && fp.starts_with(&name)))
            .map(|(_, fp)| fp.clone())
    }
}

/// Carries out the engine's actions.
fn execute(actions: Vec<Action>, node: &Node, inject: &dyn Inject) {
    for action in actions {
        let result = match action {
            Action::Grab { park } => inject.grab(park),
            Action::Ungrab { at } => inject.ungrab(at),
            Action::Inject(message) => inject.inject(&message),
            Action::Send { to, message } => {
                if let Ok(peer) = to.parse::<Fingerprint>() {
                    node.send(&peer, message);
                }
                Ok(())
            }
        };
        if let Err(e) = result {
            tracing::warn!("input: {e}");
        }
    }
}

fn role_text(role: &Role, node: &Node) -> String {
    let name = |id: &str| {
        id.parse::<Fingerprint>()
            .ok()
            .and_then(|fp| node.paired().into_iter().find(|p| p.fingerprint == fp))
            .map_or_else(|| id.chars().take(8).collect(), |p| p.name)
    };
    match role {
        Role::Idle => "local".into(),
        Role::Controlling(id) => format!("controlling {}", name(id)),
        Role::ControlledBy(id) => format!("controlled by {}", name(id)),
    }
}

pub async fn run(node: &Node, events: UnboundedReceiver<Event>, dir: &Path, backend: tc_input::Backend) -> Result<()> {
    let file = load_layout_file(dir)?;
    let layout = Layout::from_file(&file, resolver(node))?;
    let me = node.fingerprint().to_string();
    let has_neighbor = [Side::Left, Side::Right, Side::Top, Side::Bottom]
        .iter()
        .any(|s| layout.neighbor(&me, *s).is_some());
    if !has_neighbor {
        bail!(
            "no neighbor of this computer in {}; place one first, e.g. `tablecross-cli place <NAME> right`",
            dir.join(LAYOUT_FILE).display()
        );
    }
    println!("Running. Ctrl+Shift+Alt+Esc releases everything; Ctrl+C in this window quits.");
    share(node, events, layout, backend, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}

/// Connects network, engine and input until `stop` completes.
pub async fn share(
    node: &Node,
    mut events: UnboundedReceiver<Event>,
    layout: Layout,
    backend: tc_input::Backend,
    stop: impl std::future::Future<Output = ()>,
) -> Result<()> {
    let me = node.fingerprint().to_string();
    let inject = backend.inject.clone();
    let mut capture = backend.capture;
    let mut screens = inject.screens();
    if screens.is_empty() {
        bail!("no screens found");
    }
    println!("Screens: {screens:?}");
    let engine = Arc::new(Mutex::new(Engine::new(me.clone(), &screens, layout)));

    // The capture thread asks the engine about every event of the local devices
    {
        let engine = engine.clone();
        let node = node.clone();
        let inject = inject.clone();
        let mut held = BTreeSet::new();
        capture.start(Box::new(move |event: Captured| {
            if let Captured::Key { usage, down } = event {
                if down {
                    held.insert(usage);
                } else {
                    held.remove(&usage);
                }
                let emergency =
                    usage == ESCAPE && down && EMERGENCY.iter().all(|(l, r)| held.contains(l) || held.contains(r));
                if emergency {
                    let mut engine = engine.lock().expect("engine lock");
                    execute(engine.release_everything(), &node, inject.as_ref());
                    inject.release_all();
                    eprintln!("Emergency release: everything released, input is local again.");
                    return true;
                }
            }
            let mut engine = engine.lock().expect("engine lock");
            let actions = engine.captured(event);
            let grabbed = matches!(engine.role(), Role::Controlling(_));
            execute(actions, &node, inject.as_ref());
            !grabbed
        }))?;
    }

    let started = Instant::now();
    let stamp = move || format!("{:>8.3}s", started.elapsed().as_secs_f64());
    let mut role = Role::Idle;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut screen_check = tokio::time::interval(Duration::from_secs(2));
    tokio::pin!(stop);
    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                let mut engine = engine.lock().expect("engine lock");
                match event {
                    Event::Connected { peer, name, screens, address } => {
                        println!("{} connected to {name} at {address}, screens {screens:?}", stamp());
                        engine.peer_online(&peer.to_string(), &screens);
                    }
                    Event::Disconnected { peer, reason } => {
                        println!("{} disconnected from {}: {reason:?}", stamp(), peer.short());
                        let actions = engine.peer_offline(&peer.to_string());
                        execute(actions, node, inject.as_ref());
                    }
                    Event::Message { peer, message } => {
                        let actions = engine.received(&peer.to_string(), message);
                        execute(actions, node, inject.as_ref());
                    }
                    Event::Refused { address, reason, .. } => println!("{} refused {address}: {reason:?}", stamp()),
                    Event::Woke { .. } => println!("{} woke up, reconnecting", stamp()),
                    _ => {}
                }
            }
            _ = tick.tick() => {
                let mut engine = engine.lock().expect("engine lock");
                let actions = engine.tick();
                execute(actions, node, inject.as_ref());
                let now = engine.role();
                if now != role {
                    println!("{} {}", stamp(), role_text(&now, node));
                    role = now;
                }
            }
            _ = screen_check.tick() => {
                let now = inject.screens();
                if !now.is_empty() && now != screens {
                    println!("{} screens changed: {now:?}", stamp());
                    engine.lock().expect("engine lock").set_local_screens(&now);
                    screens = now;
                }
            }
            _ = &mut stop => break,
        }
    }
    let actions = engine.lock().expect("engine lock").release_everything();
    execute(actions, node, inject.as_ref());
    inject.release_all();
    capture.stop();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tc_core::Point;
    use tc_net::{Identity, NodeConfig, PeerStore};
    use tc_protocol::{Message, Screen};

    fn screen() -> Screen {
        Screen {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
            scale: 1.0,
        }
    }

    async fn node(name: &str) -> (Node, UnboundedReceiver<Event>) {
        let mut config = NodeConfig::new(name);
        config.bind = "[::]:0".parse().unwrap();
        config.discovery = false;
        config.screens = vec![screen()];
        Node::start(config, Identity::generate().unwrap(), PeerStore::in_memory())
            .await
            .unwrap()
    }

    /// Waits until `check` holds, up to two seconds.
    async fn until(mut check: impl FnMut() -> bool) {
        for _ in 0..200 {
            if check() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition not reached");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_computers_share_mouse_and_keyboard() {
        let (a, a_events) = node("Alpha").await;
        let (b, b_events) = node("Beta").await;
        let code = b.start_pairing();
        let b_addr = std::net::SocketAddr::from(([127, 0, 0, 1], b.local_addr().port()));
        a.pair(&[b_addr], &code).await.unwrap();
        let (a_fp, b_fp) = (a.fingerprint().to_string(), b.fingerprint().to_string());
        let layout = Layout::default().with(&b_fp, Side::Right, &a_fp);

        let (a_capture, a_inject, a_device) = tc_input::virtual_backend(vec![screen()]);
        let (b_capture, b_inject, b_device) = tc_input::virtual_backend(vec![screen()]);
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let (a2, b2, layout2) = (a.clone(), b.clone(), layout.clone());
        let a_task = tokio::spawn(async move {
            let backend = tc_input::Backend {
                capture: Box::new(a_capture),
                inject: a_inject,
            };
            share(&a2, a_events, layout2, backend, async {
                let _ = stop_rx.await;
            })
            .await
        });
        let b_task = tokio::spawn(async move {
            let backend = tc_input::Backend {
                capture: Box::new(b_capture),
                inject: b_inject,
            };
            share(&b2, b_events, layout, backend, std::future::pending()).await
        });
        until(|| a.status().first().is_some_and(|s| s.link.is_some())).await;
        // Give both engines a moment to learn about the other's screens
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Over the right edge of Alpha
        let crossing = Captured::Motion {
            x: 999.0,
            y: 400.0,
            dx: 5.0,
            dy: 0.0,
        };
        assert!(!a_device.send(crossing), "the crossing motion stays away from Alpha");
        assert!(a_device.grabbed());
        until(|| b_device.injected().iter().any(|m| matches!(m, Message::Enter { .. }))).await;
        assert_eq!(b_device.pointer(), Point::new(0.0, 400.0));

        assert!(!a_device.send(Captured::Key {
            usage: 0x04,
            down: true
        }));
        assert!(!a_device.send(Captured::Key {
            usage: 0x04,
            down: false
        }));
        let moved = Captured::Motion {
            x: 999.0,
            y: 400.0,
            dx: 30.0,
            dy: 10.0,
        };
        assert!(!a_device.send(moved));
        until(|| b_device.pointer() == Point::new(30.0, 410.0)).await;
        until(|| {
            b_device
                .injected()
                .iter()
                .filter(|m| matches!(m, Message::Key { usage: 0x04, .. }))
                .count()
                == 2
        })
        .await;

        // Someone uses Beta's own mouse: Beta takes over, Alpha lets go
        assert!(b_device.send(Captured::Motion {
            x: 500.0,
            y: 500.0,
            dx: 20.0,
            dy: 0.0,
        }));
        until(|| !a_device.grabbed()).await;
        assert!(
            a_device.send(Captured::Key {
                usage: 0x05,
                down: true
            }),
            "typing is local again"
        );

        let _ = stop_tx.send(());
        a_task.await.unwrap().unwrap();
        b_task.abort();
        a.shutdown().await;
        b.shutdown().await;
    }
}
