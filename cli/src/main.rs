//! Developer and test tool: pair and connect TableCross peers without a user interface.

#[cfg(target_os = "macos")]
mod helper {
    use anyhow::Result;
    use tc_input::helper_protocol::HELPER_VERSION;
    use tc_session::helper_install::{self, HelperState, LOG_FILE};

    pub fn install() -> Result<()> {
        helper_install::install()?;
        println!("Installed. The helper now asks for the Input Monitoring permission:");
        println!("System Settings → Privacy & Security → Input Monitoring → enable “de.eckonator.tablecross.helper”.");
        println!(
            "It picks the permission up by itself within half a minute; check with `tablecross-cli helper status`."
        );
        Ok(())
    }

    pub fn uninstall() -> Result<()> {
        helper_install::uninstall()?;
        println!("Removed.");
        Ok(())
    }

    pub fn restart() -> Result<()> {
        helper_install::restart()?;
        println!("Restarted.");
        Ok(())
    }

    pub fn status() -> Result<()> {
        match helper_install::status() {
            HelperState::NotRunning => println!("Not running. Install with `tablecross-cli helper install`."),
            HelperState::Running { version, access } => {
                let outdated = if version == HELPER_VERSION {
                    ""
                } else {
                    " (outdated, install again)"
                };
                println!("Running, version {version}{outdated}.");
                if access {
                    println!("Input Monitoring: granted.");
                } else {
                    println!("Input Monitoring: missing. System Settings → Privacy & Security → Input Monitoring.");
                }
            }
            HelperState::Unknown(e) => println!("Something answers, but not as expected: {e}"),
        }
        println!("Log: {LOG_FILE}");
        Ok(())
    }
}

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use tc_net::{Disconnect, Event, Fingerprint, Identity, Node, NodeConfig, PairingCode, PeerStore, Refusal};
use tc_session::{Command as SessionCommand, Session, SessionConfig, layout_file};
use tokio::sync::mpsc::UnboundedReceiver;

#[derive(Parser)]
#[command(name = "tablecross-cli", version, about)]
struct Cli {
    /// Settings folder (default: the TableCross settings folder, or TC_CONFIG_DIR)
    #[arg(long, global = true)]
    config_dir: Option<PathBuf>,
    /// Name shown to other computers (default: host name)
    #[arg(long, global = true)]
    name: Option<String>,
    /// UDP port
    #[arg(long, global = true, default_value_t = tc_net::DEFAULT_PORT)]
    port: u16,
    /// Do not announce or search on the local network (mDNS)
    #[arg(long, global = true)]
    no_discovery: bool,
    /// More log output (-v info, -vv debug)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show this computer's name and fingerprint
    Id,
    /// List paired computers and the ones found on the network
    Peers,
    /// Without CODE: show a code and wait. With CODE: pair with the computer showing it
    Pair {
        code: Option<String>,
        /// Address of the computer showing the code (default: found on the network)
        #[arg(long)]
        addr: Vec<SocketAddr>,
    },
    /// Remove a pairing (name or start of the fingerprint)
    Unpair { peer: String },
    /// Only accept connections and print what happens
    Listen,
    /// Accept and dial paired computers and print what happens
    Connect {
        /// Extra address for a peer: NAME=IP:PORT
        #[arg(long, value_parser = parse_peer_addr)]
        addr: Vec<(String, SocketAddr)>,
    },
    /// Like connect, plus round trip and losses every second; summary on Ctrl+C
    Ping {
        #[arg(long, value_parser = parse_peer_addr)]
        addr: Vec<(String, SocketAddr)>,
    },
    /// Say where a paired computer stands, seen from this one (stored in layout.json)
    Place {
        /// Name or start of the fingerprint
        peer: String,
        side: PlaceSide,
        /// Swap ⌘ and Ctrl while controlling that computer from here
        #[arg(long)]
        swap_cmd_ctrl: bool,
    },
    /// Show the screens as TableCross sees them
    Screens,
    /// The macOS system helper that keeps the keyboard working during Secure Input
    Helper {
        #[command(subcommand)]
        action: HelperAction,
    },
    /// Share mouse and keyboard with the placed computers
    Run {
        #[arg(long, value_parser = parse_peer_addr)]
        addr: Vec<(String, SocketAddr)>,
    },
}

#[derive(Subcommand)]
enum HelperAction {
    /// Install or update (asks for the administrator password)
    Install,
    /// Remove (asks for the administrator password)
    Uninstall,
    /// Restart, e.g. after granting a permission
    Restart,
    /// Is it running, does it have its permission?
    Status,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum PlaceSide {
    Left,
    Right,
    Above,
    Below,
}

fn parse_peer_addr(s: &str) -> Result<(String, SocketAddr), String> {
    let (name, addr) = s.split_once('=').ok_or("expected NAME=IP:PORT")?;
    Ok((name.to_string(), addr.parse().map_err(|e| format!("{e}"))?))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let level = match cli.verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
        )
        .with_writer(std::io::stderr)
        .init();

    let dir = match &cli.config_dir {
        Some(dir) => dir.clone(),
        None => tc_net::default_config_dir().context("no settings folder found; use --config-dir")?,
    };
    let identity = Identity::load_or_create(&dir)?;
    let store = PeerStore::load(&dir).with_context(|| format!("cannot read the peer list in {}", dir.display()))?;
    let name = cli.name.clone().unwrap_or_else(host_name);

    match &cli.command {
        Command::Id => {
            println!("Name:         {name}");
            println!("Fingerprint:  {}", identity.fingerprint());
            println!("Settings:     {}", dir.display());
            return Ok(());
        }
        Command::Place {
            peer,
            side,
            swap_cmd_ctrl,
        } => {
            return place(&dir, &identity, &store, peer, *side, *swap_cmd_ctrl);
        }
        Command::Helper { action } => {
            #[cfg(target_os = "macos")]
            return match action {
                HelperAction::Install => helper::install(),
                HelperAction::Uninstall => helper::uninstall(),
                HelperAction::Restart => helper::restart(),
                HelperAction::Status => helper::status(),
            };
            #[cfg(not(target_os = "macos"))]
            {
                let _ = action;
                bail!("the system helper is only needed on macOS");
            }
        }
        Command::Screens => {
            let backend = tc_input::platform()?;
            for s in backend.inject.screens() {
                println!("  {}×{} at ({}, {}), scale {}", s.width, s.height, s.x, s.y, s.scale);
            }
            return Ok(());
        }
        Command::Peers if cli.no_discovery => {
            println!("Paired:");
            print_paired(store.peers());
            return Ok(());
        }
        _ => {}
    }

    let mut config = NodeConfig::new(name);
    config.bind.set_port(cli.port);
    config.discovery = !cli.no_discovery;
    config.dial = !matches!(cli.command, Command::Listen | Command::Pair { .. } | Command::Peers);
    if let Command::Run { addr } = &cli.command {
        drop(store);
        return share(dir, config.name, cli.port, !cli.no_discovery, addr).await;
    }
    let (node, events) = Node::start(config, identity, store).await?;
    let result = match cli.command {
        Command::Id | Command::Place { .. } | Command::Screens | Command::Helper { .. } | Command::Run { .. } => {
            unreachable!("handled above")
        }
        Command::Peers => peers(&node).await,
        Command::Pair { code: None, .. } => show_code(&node, events).await,
        Command::Pair { code: Some(code), addr } => type_code(&node, &code, addr).await,
        Command::Unpair { peer } => unpair(&node, &peer).await,
        Command::Listen => run(&node, events, false).await,
        Command::Connect { addr } => {
            add_addresses(&node, &addr)?;
            run(&node, events, false).await
        }
        Command::Ping { addr } => {
            add_addresses(&node, &addr)?;
            run(&node, events, true).await
        }
    };
    node.shutdown().await;
    result
}

fn place(
    dir: &std::path::Path,
    identity: &Identity,
    store: &PeerStore,
    query: &str,
    side: PlaceSide,
    swap_cmd_ctrl: bool,
) -> Result<()> {
    let query_lower = query.to_lowercase();
    let matches: Vec<_> = store
        .peers()
        .iter()
        .filter(|p| p.name.to_lowercase() == query_lower || p.fingerprint.to_string().starts_with(&query_lower))
        .collect();
    let peer = match matches.as_slice() {
        [one] => *one,
        [] => bail!("no paired computer matches \"{query}\" (see: tablecross-cli peers)"),
        _ => bail!("\"{query}\" matches several computers; use more of the fingerprint"),
    };
    let (side, words) = match side {
        PlaceSide::Left => (tc_core::Side::Left, "left of"),
        PlaceSide::Right => (tc_core::Side::Right, "right of"),
        PlaceSide::Above => (tc_core::Side::Top, "above"),
        PlaceSide::Below => (tc_core::Side::Bottom, "below"),
    };
    let mut file = layout_file::load(dir)?;
    layout_file::place(&mut file, &identity.fingerprint(), &peer.fingerprint, Some(side));
    layout_file::set_swap(&mut file, &peer.fingerprint, swap_cmd_ctrl);
    layout_file::save(dir, &file)?;
    println!("{} is now {words} this computer.", peer.name);
    Ok(())
}

/// Shares mouse and keyboard until Ctrl+C.
async fn share(
    dir: PathBuf,
    name: String,
    port: u16,
    discovery: bool,
    addresses: &[(String, SocketAddr)],
) -> Result<()> {
    let backend = tc_input::platform()?;
    let config = SessionConfig {
        dir,
        name,
        port,
        discovery,
    };
    // Clipboard content up to 50 MB is shared
    let clipboard = tc_clipboard::platform().map(|system| tc_session::Clipboard {
        system: system.into(),
        max_size: 50 * 1024 * 1024,
    });
    let (session, task) = Session::start(config, Some(backend), clipboard, None).await?;
    let mut notices = session.notices();
    let status = session.status();
    if !status.peers.iter().any(|p| p.side.is_some()) {
        session.send(SessionCommand::Shutdown);
        let _ = task.await;
        bail!("no computer placed next to this one; e.g. `tablecross-cli place <NAME> right`");
    }
    for (query, address) in addresses {
        let query = query.to_lowercase();
        match status
            .peers
            .iter()
            .find(|p| p.name.to_lowercase() == query || p.fingerprint.to_string().starts_with(&query))
        {
            Some(p) => session.send(SessionCommand::AddAddress {
                peer: p.fingerprint,
                address: *address,
            }),
            None => bail!("no paired computer matches \"{query}\""),
        }
    }
    println!("Screens: {:?}", status.screens);
    println!("Running. Ctrl+Shift+Alt+Esc releases everything, Ctrl+Shift+Alt+arrow switches; Ctrl+C quits.");
    let started = Instant::now();
    loop {
        tokio::select! {
            notice = notices.recv() => match notice {
                Ok(notice) => println!("{:>8.3}s {notice}", started.elapsed().as_secs_f64()),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    session.send(SessionCommand::Shutdown);
    let _ = task.await;
    Ok(())
}

/// Host name without domain ("office-mac.fritz.box" → "office-mac").
fn host_name() -> String {
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    name.split('.').next().unwrap_or_default().to_string()
}

fn find_peer(node: &Node, query: &str) -> Result<Fingerprint> {
    let query_lower = query.to_lowercase();
    let matches: Vec<_> = node
        .paired()
        .into_iter()
        .filter(|p| p.name.to_lowercase() == query_lower || p.fingerprint.to_string().starts_with(&query_lower))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.fingerprint),
        [] => bail!("no paired computer matches \"{query}\" (see: tablecross-cli peers)"),
        _ => bail!("\"{query}\" matches several computers; use more of the fingerprint"),
    }
}

fn add_addresses(node: &Node, addresses: &[(String, SocketAddr)]) -> Result<()> {
    for (peer, address) in addresses {
        node.add_address(&find_peer(node, peer)?, *address)?;
    }
    Ok(())
}

fn print_paired(peers: &[tc_net::PairedPeer]) {
    if peers.is_empty() {
        println!("  none");
    }
    for peer in peers {
        let addresses: Vec<_> = peer.addresses.iter().map(ToString::to_string).collect();
        println!(
            "  {}  {:<24}  {}",
            peer.fingerprint.short(),
            peer.name,
            addresses.join(", ")
        );
    }
}

async fn peers(node: &Node) -> Result<()> {
    println!("Paired:");
    print_paired(&node.paired());
    tokio::time::sleep(Duration::from_secs(3)).await;
    println!("On the network:");
    let found = node.discovered();
    if found.is_empty() {
        println!("  nothing found (other computers running? same network? firewall?)");
    }
    for d in found {
        let addresses: Vec<_> = d.addresses.iter().map(ToString::to_string).collect();
        println!(
            "  {}  {:<24}  {}{}",
            d.fingerprint.short(),
            d.name,
            addresses.join(", "),
            if d.pairing { "  [shows a pairing code]" } else { "" }
        );
    }
    Ok(())
}

async fn show_code(node: &Node, mut events: UnboundedReceiver<Event>) -> Result<()> {
    let code = node.start_pairing();
    println!("Pairing code:  {code}");
    println!("On the other computer run:  tablecross-cli pair {}", code.digits());
    println!("Valid for {} minutes. Waiting…", tc_net::PAIRING_WINDOW.as_secs() / 60);
    let deadline = tokio::time::sleep(tc_net::PAIRING_WINDOW);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(Event::Paired { peer }) => {
                    println!("Paired with {} ({}).", peer.name, peer.fingerprint.short());
                    return Ok(());
                }
                Some(Event::PairingFailed { attempts_left }) => {
                    println!("Wrong code entered on the other computer ({attempts_left} attempts left).");
                }
                Some(Event::PairingClosed) => bail!("too many wrong attempts; start again for a new code"),
                Some(Event::Refused { address, reason, .. }) => println!("{address}: {}", refusal_text(&reason)),
                Some(_) => {}
                None => bail!("node stopped"),
            },
            _ = &mut deadline => bail!("the code expired"),
            _ = tokio::signal::ctrl_c() => bail!("cancelled"),
        }
    }
}

async fn type_code(node: &Node, code: &str, addresses: Vec<SocketAddr>) -> Result<()> {
    let code = PairingCode::parse(code).context("the code has 6 digits")?;
    let addresses = if addresses.is_empty() {
        println!("Searching the network for a computer showing a code…");
        let started = Instant::now();
        loop {
            let showing: Vec<_> = node.discovered().into_iter().filter(|d| d.pairing).collect();
            match showing.as_slice() {
                [one] => {
                    println!("Found {} ({}).", one.name, one.fingerprint.short());
                    break one.addresses.clone();
                }
                [] if started.elapsed() < Duration::from_secs(10) => {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                [] => bail!("no computer showing a code found; pass its address with --addr IP:PORT"),
                several => {
                    let names: Vec<_> = several.iter().map(|d| d.name.as_str()).collect();
                    bail!("several computers show a code ({}); pass --addr", names.join(", "));
                }
            }
        }
    } else {
        addresses
    };
    let peer = node.pair(&addresses, &code).await?;
    println!("Paired with {} ({}).", peer.name, peer.fingerprint.short());
    Ok(())
}

async fn unpair(node: &Node, query: &str) -> Result<()> {
    let peer = find_peer(node, query)?;
    // Give a running peer a moment to connect, so it learns about it as well
    tokio::time::sleep(Duration::from_secs(1)).await;
    node.unpair(&peer)?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    println!("Removed.");
    Ok(())
}

/// Per-peer numbers for the summary of `ping`.
#[derive(Default)]
struct Tally {
    name: String,
    samples: Vec<Duration>,
    offline_since: Option<Instant>,
    longest_offline: Duration,
    reconnects: u32,
}

async fn run(node: &Node, mut events: UnboundedReceiver<Event>, ping: bool) -> Result<()> {
    let started = Instant::now();
    let stamp = move || format!("{:>8.3}s", started.elapsed().as_secs_f64());
    println!(
        "{} {} ({}) on port {}. Ctrl+C to stop.",
        stamp(),
        node.name(),
        node.fingerprint().short(),
        node.local_addr().port()
    );
    if node.paired().is_empty() {
        println!("No paired computers yet; run `tablecross-cli pair` first.");
    }
    let mut tallies: HashMap<Fingerprint, Tally> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                let tally = |peer: &Fingerprint, tallies: &mut HashMap<Fingerprint, Tally>| {
                    tallies.entry(*peer).or_default().name.clone_from(&peer_name(node, peer));
                };
                match event {
                    Event::Connected { peer, name, address, .. } => {
                        tally(&peer, &mut tallies);
                        let t = tallies.get_mut(&peer).expect("just added");
                        let offline = t.offline_since.take().map(|since| since.elapsed());
                        match offline {
                            Some(offline) => {
                                t.reconnects += 1;
                                t.longest_offline = t.longest_offline.max(offline);
                                println!("{} connected to {name} at {address} after {:.2} s offline", stamp(), offline.as_secs_f64());
                            }
                            None => println!("{} connected to {name} at {address}", stamp()),
                        }
                    }
                    Event::Disconnected { peer, reason } => {
                        tally(&peer, &mut tallies);
                        let t = tallies.get_mut(&peer).expect("just added");
                        t.offline_since.get_or_insert_with(Instant::now);
                        println!("{} disconnected from {}: {}", stamp(), t.name, disconnect_text(&reason));
                    }
                    Event::Message { peer, message } => println!("{} {}: {message:?}", stamp(), peer_name(node, &peer)),
                    Event::Paired { peer } => println!("{} paired with {}", stamp(), peer.name),
                    Event::Unpaired { peer, by_peer } => println!(
                        "{} pairing with {} removed{}",
                        stamp(),
                        peer.short(),
                        if by_peer { " by the other computer" } else { "" }
                    ),
                    Event::Refused { peer, address, reason } => {
                        let who = peer.map(|p| peer_name(node, &p)).unwrap_or_else(|| address.to_string());
                        println!("{} refused {who}: {}", stamp(), refusal_text(&reason));
                    }
                    Event::Woke { slept } => println!("{} woke up after about {} s, reconnecting", stamp(), slept.as_secs()),
                    Event::PairingFailed { .. } | Event::PairingClosed | Event::Blob { .. } | Event::Stream { .. } => {}
                }
            }
            _ = tick.tick(), if ping => {
                for status in node.status() {
                    let fp = status.peer.fingerprint;
                    let samples = node.take_rtt_samples(&fp);
                    let t = tallies.entry(fp).or_default();
                    t.name.clone_from(&status.peer.name);
                    t.samples.extend(&samples);
                    match &status.link {
                        Some(link) => println!(
                            "{} {:<20} rtt {}  quic {:.2} ms  lost {}/{}  reconnects {}",
                            stamp(),
                            status.peer.name,
                            link.rtt.map_or("–".into(), |r| format!("{:.2} ms", ms(r))),
                            ms(link.quic_rtt),
                            link.probes_lost,
                            link.probes_sent,
                            status.reconnects
                        ),
                        None => println!("{} {:<20} not connected", stamp(), status.peer.name),
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    if ping {
        println!();
        println!("Summary");
        for t in tallies.values_mut() {
            t.samples.sort();
            let pick = |q: f64| {
                t.samples
                    .get(((t.samples.len() as f64 - 1.0) * q).round() as usize)
                    .copied()
            };
            match (pick(0.5), pick(0.95), t.samples.last()) {
                (Some(median), Some(p95), Some(max)) => println!(
                    "  {:<20} {} probes  median {:.2} ms  95 % {:.2} ms  max {:.2} ms",
                    t.name,
                    t.samples.len(),
                    ms(median),
                    ms(p95),
                    ms(*max)
                ),
                _ => println!("  {:<20} no measurements", t.name),
            }
            println!(
                "  {:<20} reconnects {}  longest offline {:.2} s",
                "",
                t.reconnects,
                t.longest_offline.as_secs_f64()
            );
        }
    }
    Ok(())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn peer_name(node: &Node, peer: &Fingerprint) -> String {
    node.paired()
        .into_iter()
        .find(|p| p.fingerprint == *peer)
        .map(|p| p.name)
        .unwrap_or_else(|| peer.short())
}

fn disconnect_text(reason: &Disconnect) -> String {
    match reason {
        Disconnect::Timeout => "no answer for 2 seconds".into(),
        Disconnect::PeerShutdown => "the other computer quit".into(),
        Disconnect::PeerSlept => "the other computer went to sleep".into(),
        Disconnect::Slept => "this computer was asleep".into(),
        Disconnect::Replaced => "replaced by a newer connection".into(),
        Disconnect::Unpaired => "pairing removed".into(),
        Disconnect::Shutdown => "shutting down".into(),
        Disconnect::ProtocolError(e) => format!("protocol error: {e}"),
        Disconnect::Lost(e) => format!("network error: {e}"),
    }
}

fn refusal_text(reason: &Refusal) -> String {
    match reason {
        Refusal::NotPaired => "not paired with this computer".into(),
        Refusal::VersionMismatch { ours, theirs } => {
            format!("uses protocol version {theirs}, this computer {ours}; update the older one")
        }
        Refusal::WrongPeer => "another device answered at the remembered address".into(),
        Refusal::ProtocolError(e) => format!("protocol error: {e}"),
    }
}
