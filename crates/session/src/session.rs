//! The running session: network node, engine and input backend, driven by one loop. The app and
//! the command line tool send [`Command`]s and read a [`Status`] snapshot plus [`Notice`]s.

use std::collections::{BTreeSet, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use tc_core::{Action, Captured, Engine, Layout, LayoutFile, Role, Side};
use tc_input::{Backend, Capture, Inject, Problem};
use tc_net::{
    Disconnect, Discovered, Event, Fingerprint, Identity, LinkInfo, Node, NodeConfig, PairError, PairingCode,
    PeerStore, Refusal,
};
use tc_protocol::{Message, Screen};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

use crate::clip::{self, ClipAction, ClipSync, Clipboard, Skip};
use crate::files::{self, Listing};
use crate::layout_file;
use crate::transfer::{self, Cancels, Progress, TransferState, TransferView};

const ESCAPE: u16 = 0x29;
const ARROWS: [(u16, Side); 4] = [
    (0x4F, Side::Right),
    (0x50, Side::Left),
    (0x51, Side::Bottom),
    (0x52, Side::Top),
];
/// Ctrl, Shift and Alt (either side) form the prefix of the keyboard shortcuts.
const PREFIX: [(u16, u16); 3] = [(0xE0, 0xE4), (0xE1, 0xE5), (0xE2, 0xE6)];
const MAX_NOTICES: usize = 200;
const STATUS_EVERY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Settings folder (identity, peers, layout).
    pub dir: PathBuf,
    pub name: String,
    pub port: u16,
    pub discovery: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PairTarget {
    /// A computer found on the network.
    Discovered(Fingerprint),
    Address(SocketAddr),
}

#[derive(Debug, Clone)]
pub enum Command {
    /// Show a pairing code (for `PAIRING_WINDOW`).
    StartPairing,
    StopPairing,
    /// Pair with the computer that shows `code`.
    Pair {
        code: String,
        target: PairTarget,
    },
    Unpair(Fingerprint),
    /// Put a peer next to this computer (`None`: nowhere).
    Place {
        peer: Fingerprint,
        side: Option<Side>,
    },
    SwapCmdCtrl {
        peer: Fingerprint,
        swap: bool,
    },
    AddAddress {
        peer: Fingerprint,
        address: SocketAddr,
    },
    /// Put the pointer on that computer (`None`: this one).
    SwitchTo(Option<Fingerprint>),
    /// Emergency: everything back to local control, all keys released.
    ReleaseAll,
    /// Try to start input capture again (after granting a permission).
    RetryInput,
    CancelTransfer(u64),
    Shutdown,
}

/// Something that happened, for the list of recent messages.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    Connected(String),
    Disconnected(String, Disconnect),
    Refused(String, Refusal),
    Paired(String),
    /// A wrong code was typed for the code shown here.
    PairingFailed(u8),
    PairingClosed,
    PairError(PairError),
    Unpaired(String, bool),
    Woke,
    Controlling(String),
    Local,
    ControlledBy(String),
    EmergencyRelease,
    Problem(Problem),
    InputFailed(String),
    /// Clipboard content from this computer stayed here.
    ClipboardSkipped(Skip),
    /// Clipboard content from this peer was taken over.
    ClipboardReceived(String),
    /// Files from this peer arrived in the folder; they are on the clipboard as well.
    FilesReceived(String, u64, PathBuf),
    TransferFailed(String, String),
}

impl std::fmt::Display for Notice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Notice::Connected(n) => write!(f, "connected to {n}"),
            Notice::Disconnected(n, r) => write!(f, "disconnected from {n}: {r:?}"),
            Notice::Refused(n, r) => write!(f, "refused {n}: {r:?}"),
            Notice::Paired(n) => write!(f, "paired with {n}"),
            Notice::PairingFailed(left) => write!(f, "wrong pairing code entered ({left} attempts left)"),
            Notice::PairingClosed => write!(f, "pairing code void after too many attempts"),
            Notice::PairError(e) => write!(f, "pairing failed: {e}"),
            Notice::Unpaired(n, by_peer) => {
                write!(
                    f,
                    "pairing with {n} removed{}",
                    if *by_peer { " by the other computer" } else { "" }
                )
            }
            Notice::Woke => write!(f, "woke from sleep, reconnecting"),
            Notice::Controlling(n) => write!(f, "controlling {n}"),
            Notice::Local => write!(f, "input is local"),
            Notice::ControlledBy(n) => write!(f, "controlled by {n}"),
            Notice::EmergencyRelease => write!(f, "emergency release: everything released"),
            Notice::Problem(p) => write!(f, "warning: {p}"),
            Notice::InputFailed(e) => write!(f, "input capture not available: {e}"),
            Notice::ClipboardSkipped(Skip::Concealed) => write!(f, "clipboard not shared: marked as private"),
            Notice::ClipboardSkipped(Skip::TooLarge(n)) => write!(f, "clipboard not shared: too large ({n} bytes)"),
            Notice::ClipboardReceived(n) => write!(f, "clipboard taken over from {n}"),
            Notice::FilesReceived(n, count, dir) => {
                write!(f, "{count} file(s) from {n} in {}", dir.display())
            }
            Notice::TransferFailed(n, e) => write!(f, "file transfer with {n} failed: {e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PeerView {
    pub fingerprint: Fingerprint,
    pub name: String,
    pub link: Option<LinkInfo>,
    pub reconnects: u32,
    pub refusal: Option<Refusal>,
    pub side: Option<Side>,
    pub swap_cmd_ctrl: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum InputState {
    #[default]
    Off,
    Running,
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub name: String,
    pub fingerprint: Option<Fingerprint>,
    pub port: u16,
    pub controlling: Option<String>,
    pub controlled_by: Option<String>,
    pub peers: Vec<PeerView>,
    /// Computers on the network that are not paired.
    pub discovered: Vec<Discovered>,
    /// The code shown here and until when it is valid.
    pub pairing: Option<(PairingCode, Instant)>,
    pub pairing_busy: bool,
    pub input: InputState,
    pub problems: Vec<Problem>,
    pub screens: Vec<Screen>,
    pub notices: VecDeque<(SystemTime, Notice)>,
    pub layout: LayoutFile,
    /// File transfers, newest last.
    pub transfers: Vec<TransferView>,
    /// Files copied on another computer that come over when the pointer arrives here.
    pub files_waiting: Option<String>,
}

/// Talks to a running session; cheap to clone.
#[derive(Clone)]
pub struct SessionHandle {
    commands: mpsc::UnboundedSender<Command>,
    status: Arc<Mutex<Status>>,
    notices: broadcast::Sender<Notice>,
}

impl SessionHandle {
    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn status(&self) -> Status {
        self.status.lock().expect("status lock").clone()
    }

    pub fn notices(&self) -> broadcast::Receiver<Notice> {
        self.notices.subscribe()
    }
}

/// Called whenever the status changed (the app repaints).
pub type OnChange = Arc<dyn Fn() + Send + Sync>;

enum Internal {
    Notice(Notice),
    PairDone(Result<String, PairError>),
    ClipRead(Option<tc_clipboard::Read>),
    ClipWritten(Fingerprint, Option<u64>),
    FilesListed(Result<Listing, String>),
}

/// Copied files and running transfers.
#[derive(Default)]
struct FileState {
    /// Our copied files, if they are the newest content anywhere.
    local: Option<(u64, Arc<Listing>)>,
    /// Files waiting on a peer: peer, id, label, count, size.
    remote: Option<(Fingerprint, u64, String, u64, u64)>,
    requested: Option<u64>,
    /// What we put on the clipboard after a transfer (not announced again).
    written: Vec<PathBuf>,
    cancels: Cancels,
    counter: u64,
}

struct Shared {
    node: Node,
    engine: Arc<Mutex<Engine>>,
    inject: Option<Arc<dyn Inject>>,
    status: Arc<Mutex<Status>>,
    notices: broadcast::Sender<Notice>,
    internal: mpsc::UnboundedSender<Internal>,
    on_change: Option<OnChange>,
    dir: PathBuf,
    clipboard: Option<Clipboard>,
    clip: Mutex<ClipSync>,
    files: Mutex<FileState>,
}

impl Shared {
    fn notice(&self, notice: Notice) {
        tracing::info!("{notice}");
        let _ = self.notices.send(notice.clone());
        let mut status = self.status.lock().expect("status lock");
        status.notices.push_back((SystemTime::now(), notice));
        while status.notices.len() > MAX_NOTICES {
            status.notices.pop_front();
        }
        drop(status);
        self.changed();
    }

    fn changed(&self) {
        if let Some(f) = &self.on_change {
            f();
        }
    }

    fn name_of(&self, id: &str) -> String {
        id.parse::<Fingerprint>()
            .ok()
            .and_then(|fp| self.node.paired().into_iter().find(|p| p.fingerprint == fp))
            .map_or_else(|| id.chars().take(8).collect(), |p| p.name)
    }

    fn execute(&self, actions: Vec<Action>) {
        execute(actions, &self.node, self.inject.as_deref());
    }

    /// Turns a name or fingerprint prefix from `layout.json` into this machine or a paired one.
    fn resolver(&self) -> impl Fn(&str) -> Option<Fingerprint> + use<> {
        let machines: Vec<_> = std::iter::once((self.node.name().to_lowercase(), self.node.fingerprint()))
            .chain(
                self.node
                    .paired()
                    .into_iter()
                    .map(|p| (p.name.to_lowercase(), p.fingerprint)),
            )
            .collect();
        move |name: &str| {
            let name = name.to_lowercase();
            machines
                .iter()
                .find(|(n, fp)| *n == name || (name.len() >= 8 && fp.to_string().starts_with(&name)))
                .map(|(_, fp)| *fp)
        }
    }

    fn layout(&self) -> Result<Layout> {
        let file = layout_file::load(&self.dir)?;
        let resolve = self.resolver();
        let layout = Layout::from_file(&file, |name: &str| resolve(name).map(|fp| fp.to_string()))?;
        self.status.lock().expect("status lock").layout = file;
        Ok(layout)
    }

    fn reload_layout(&self) {
        match self.layout() {
            Ok(layout) => self.engine.lock().expect("engine lock").set_layout(layout),
            Err(e) => tracing::warn!("layout: {e}"),
        }
    }

    /// Changes the arrangement here and passes it on to all connected computers.
    fn edit_layout(&self, edit: impl FnOnce(&mut LayoutFile)) {
        let me = self.node.fingerprint();
        let result = layout_file::load(&self.dir).and_then(|mut file| {
            edit(&mut file);
            layout_file::bump(&mut file, &me);
            layout_file::save(&self.dir, &file)
        });
        if let Err(e) = result {
            tracing::warn!("layout: {e}");
        }
        self.reload_layout();
        self.refresh();
        self.share_layout(None);
    }

    /// Our arrangement for other computers.
    fn layout_message(&self) -> Option<Message> {
        let file = layout_file::load(&self.dir).ok()?;
        let me = (self.node.fingerprint(), self.node.name().to_string());
        let paired = self.node.paired();
        Some(layout_file::message(&file, self.resolver(), |fp| {
            if *fp == me.0 {
                return Some(me.1.clone());
            }
            paired.iter().find(|p| p.fingerprint == *fp).map(|p| p.name.clone())
        }))
    }

    /// Sends the arrangement to `to`, or to every connected computer except `except`.
    fn share_layout_with(&self, to: &Fingerprint) {
        if let Some(message) = self.layout_message() {
            self.node.send(to, message);
        }
    }

    fn share_layout(&self, except: Option<Fingerprint>) {
        let Some(message) = self.layout_message() else { return };
        for peer in self.node.status().into_iter().filter(|p| p.link.is_some()) {
            if Some(peer.peer.fingerprint) != except {
                self.node.send(&peer.peer.fingerprint, message.clone());
            }
        }
    }

    /// An arrangement from `from`: taken over if newer, then passed on.
    fn layout_received(&self, from: Fingerprint, message: Message) {
        let Message::Layout {
            revision,
            editor,
            placements,
            names,
        } = message
        else {
            return;
        };
        let result = layout_file::load(&self.dir).and_then(|mut file| {
            let newer = layout_file::adopt(&mut file, revision, editor, placements, names);
            if newer {
                layout_file::save(&self.dir, &file)?;
            }
            Ok(newer)
        });
        match result {
            Ok(true) => {
                tracing::info!("arrangement taken over from {}", self.name_of(&from.to_string()));
                self.reload_layout();
                self.refresh();
                self.share_layout(Some(from));
            }
            Ok(false) => {}
            Err(e) => tracing::warn!("layout: {e}"),
        }
    }

    /// Rebuilds the status snapshot from node, engine and input.
    fn refresh(&self) {
        let file = self.status.lock().expect("status lock").layout.clone();
        let me = self.node.fingerprint();
        let peers: Vec<PeerView> = self
            .node
            .status()
            .into_iter()
            .map(|s| PeerView {
                side: layout_file::side_of(&file, &me, self.node.name(), &s.peer.fingerprint, &s.peer.name),
                swap_cmd_ctrl: layout_file::swaps(&file, &s.peer.fingerprint, &s.peer.name),
                fingerprint: s.peer.fingerprint,
                name: s.peer.name,
                link: s.link,
                reconnects: s.reconnects,
                refusal: s.refusal,
            })
            .collect();
        let paired: BTreeSet<Fingerprint> = peers.iter().map(|p| p.fingerprint).collect();
        let discovered = self
            .node
            .discovered()
            .into_iter()
            .filter(|d| !paired.contains(&d.fingerprint))
            .collect();
        let role = self.engine.lock().expect("engine lock").role();
        let (controlling, controlled_by) = match role {
            Role::Idle => (None, None),
            Role::Controlling(id) => (Some(self.name_of(&id)), None),
            Role::ControlledBy(id) => (None, Some(self.name_of(&id))),
        };
        let mut status = self.status.lock().expect("status lock");
        status.peers = peers;
        status.discovered = discovered;
        status.controlling = controlling;
        status.controlled_by = controlled_by;
        if status
            .pairing
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() >= *until)
        {
            status.pairing = None;
        }
    }
}

/// Carries out the engine's actions.
fn execute(actions: Vec<Action>, node: &Node, inject: Option<&dyn Inject>) {
    for action in actions {
        let result = match (action, inject) {
            (Action::Send { to, message }, _) => {
                if let Ok(peer) = to.parse::<Fingerprint>() {
                    node.send(&peer, message);
                }
                Ok(())
            }
            (Action::Grab { park }, Some(inject)) => inject.grab(park),
            (Action::Ungrab { at }, Some(inject)) => inject.ungrab(at),
            (Action::Inject(message), Some(inject)) => inject.inject(&message),
            (_, None) => Ok(()),
        };
        if let Err(e) = result {
            tracing::warn!("input: {e}");
        }
    }
}

/// The capture handler: keyboard shortcuts first, then the engine decides.
fn handler(shared: Arc<Shared>) -> tc_input::Handler {
    let mut held = BTreeSet::new();
    Box::new(move |event: Captured| {
        if let Captured::Key { usage, down } = event {
            if down {
                held.insert(usage);
            } else {
                held.remove(&usage);
            }
            let prefix = PREFIX.iter().all(|(l, r)| held.contains(l) || held.contains(r));
            if down && prefix && usage == ESCAPE {
                let actions = shared.engine.lock().expect("engine lock").release_everything();
                shared.execute(actions);
                if let Some(inject) = &shared.inject {
                    inject.release_all();
                }
                let _ = shared.internal.send(Internal::Notice(Notice::EmergencyRelease));
                return true;
            }
            if prefix && let Some((_, side)) = ARROWS.iter().find(|(u, _)| *u == usage) {
                if down {
                    let actions = shared.engine.lock().expect("engine lock").switch_side(*side);
                    shared.execute(actions);
                }
                return false;
            }
        }
        let mut engine = shared.engine.lock().expect("engine lock");
        let actions = engine.captured(event);
        let grabbed = matches!(engine.role(), Role::Controlling(_));
        execute(actions, &shared.node, shared.inject.as_deref());
        !grabbed
    })
}

pub struct Session;

impl Session {
    /// Starts node, input (if a backend is given) and the loop on the current Tokio runtime.
    pub async fn start(
        config: SessionConfig,
        backend: Option<Backend>,
        clipboard: Option<Clipboard>,
        on_change: Option<OnChange>,
    ) -> Result<(SessionHandle, JoinHandle<()>)> {
        let identity = Identity::load_or_create(&config.dir)?;
        let store = PeerStore::load(&config.dir)?;
        let screens = backend.as_ref().map(|b| b.inject.screens()).unwrap_or_default();
        let mut node_config = NodeConfig::new(config.name.clone());
        node_config.bind.set_port(config.port);
        node_config.discovery = config.discovery;
        node_config.screens = screens.clone();
        let (node, events) = Node::start(node_config, identity, store).await?;
        Self::with_node(node, events, config.dir, backend, clipboard, on_change).await
    }

    /// Like `start`, with a node that is already running (tests).
    pub async fn with_node(
        node: Node,
        events: mpsc::UnboundedReceiver<Event>,
        dir: PathBuf,
        backend: Option<Backend>,
        clipboard: Option<Clipboard>,
        on_change: Option<OnChange>,
    ) -> Result<(SessionHandle, JoinHandle<()>)> {
        let screens = backend.as_ref().map(|b| b.inject.screens()).unwrap_or_default();
        let (inject, capture) = match backend {
            Some(b) => (Some(b.inject), Some(b.capture)),
            None => (None, None),
        };
        let status = Arc::new(Mutex::new(Status {
            name: node.name().to_string(),
            fingerprint: Some(node.fingerprint()),
            port: node.local_addr().port(),
            screens: screens.clone(),
            ..Status::default()
        }));
        let (commands_tx, commands) = mpsc::unbounded_channel();
        let (internal_tx, internal) = mpsc::unbounded_channel();
        let (notices, _) = broadcast::channel(256);
        let engine = Arc::new(Mutex::new(Engine::new(
            node.fingerprint().to_string(),
            &screens,
            Layout::default(),
        )));
        let shared = Arc::new(Shared {
            node,
            engine,
            inject,
            status: status.clone(),
            notices: notices.clone(),
            internal: internal_tx,
            on_change,
            dir,
            clipboard,
            clip: Mutex::new(ClipSync::default()),
            files: Mutex::new(FileState::default()),
        });
        shared.reload_layout();
        shared.refresh();
        let handle = SessionHandle {
            commands: commands_tx,
            status,
            notices,
        };
        let task = tokio::spawn(run(shared, capture, events, commands, internal));
        Ok((handle, task))
    }
}

fn start_capture(shared: &Arc<Shared>, capture: &mut Option<Box<dyn Capture>>) {
    let Some(capture) = capture.as_mut() else {
        return;
    };
    let state = match capture.start(handler(shared.clone())) {
        Ok(()) => InputState::Running,
        Err(e) => {
            shared.notice(Notice::InputFailed(e.to_string()));
            InputState::Failed(e.to_string())
        }
    };
    shared.status.lock().expect("status lock").input = state;
    shared.changed();
}

async fn run(
    shared: Arc<Shared>,
    mut capture: Option<Box<dyn Capture>>,
    mut events: mpsc::UnboundedReceiver<Event>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut internal: mpsc::UnboundedReceiver<Internal>,
) {
    start_capture(&shared, &mut capture);
    let mut role = Role::Idle;
    let mut problems: Vec<Problem> = Vec::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut status_tick = tokio::time::interval(STATUS_EVERY);
    let mut screen_tick = tokio::time::interval(Duration::from_secs(2));
    let mut clip_tick = tokio::time::interval(Duration::from_millis(300));
    loop {
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { break };
                handle_event(&shared, event);
            }
            command = commands.recv() => {
                match command {
                    None | Some(Command::Shutdown) => break,
                    Some(Command::CancelTransfer(id)) => {
                        shared.files.lock().expect("files lock").cancels.cancel(id);
                    }
                    Some(Command::RetryInput) => {
                        if !matches!(shared.status.lock().expect("status lock").input, InputState::Running) {
                            start_capture(&shared, &mut capture);
                        }
                    }
                    Some(command) => handle_command(&shared, command),
                }
            }
            Some(message) = internal.recv() => match message {
                Internal::Notice(notice) => shared.notice(notice),
                Internal::ClipRead(read) => clip_read(&shared, read),
                Internal::FilesListed(listing) => files_listed(&shared, listing),
                Internal::ClipWritten(from, count) => {
                    shared.clip.lock().expect("clip lock").write_done(count);
                    if count.is_some() {
                        shared.notice(Notice::ClipboardReceived(shared.name_of(&from.to_string())));
                    }
                }
                Internal::PairDone(result) => {
                    shared.status.lock().expect("status lock").pairing_busy = false;
                    match result {
                        Ok(name) => shared.notice(Notice::Paired(name)),
                        Err(e) => shared.notice(Notice::PairError(e)),
                    }
                    shared.reload_layout();
                }
            },
            _ = tick.tick() => {
                let actions = shared.engine.lock().expect("engine lock").tick();
                shared.execute(actions);
                let now = shared.engine.lock().expect("engine lock").role();
                if now != role {
                    let arrived = matches!(now, Role::ControlledBy(_))
                        || (now == Role::Idle && matches!(role, Role::Controlling(_)));
                    if arrived {
                        request_files(&shared);
                    }
                    if !matches!(now, Role::Controlling(_)) {
                        let request = shared.clip.lock().expect("clip lock").became_active();
                        if let Some((peer, message)) = request {
                            shared.node.send(&peer, message);
                        }
                    }
                    let notice = match &now {
                        Role::Idle => Notice::Local,
                        Role::Controlling(id) => Notice::Controlling(shared.name_of(id)),
                        Role::ControlledBy(id) => Notice::ControlledBy(shared.name_of(id)),
                    };
                    role = now;
                    shared.refresh();
                    shared.notice(notice);
                }
            }
            _ = status_tick.tick() => {
                shared.refresh();
                if let Some(capture) = &capture {
                    let health = capture.health();
                    for problem in health.problems.iter().filter(|p| !problems.contains(p)) {
                        shared.notice(Notice::Problem(problem.clone()));
                    }
                    problems = health.problems.clone();
                    shared.status.lock().expect("status lock").problems = health.problems;
                }
                shared.changed();
            }
            _ = clip_tick.tick() => clip_poll(&shared),
            _ = screen_tick.tick() => {
                if let Some(inject) = &shared.inject {
                    let now = inject.screens();
                    let mut status = shared.status.lock().expect("status lock");
                    if !now.is_empty() && now != status.screens {
                        status.screens = now.clone();
                        drop(status);
                        shared.engine.lock().expect("engine lock").set_local_screens(&now);
                    }
                }
            }
        }
    }
    let actions = shared.engine.lock().expect("engine lock").release_everything();
    shared.execute(actions);
    if let Some(inject) = &shared.inject {
        inject.release_all();
    }
    if let Some(capture) = capture.as_mut() {
        capture.stop();
    }
    shared.node.shutdown().await;
}

fn files_offer(id: u64, listing: &Listing) -> Message {
    Message::FilesOffer {
        id,
        names: listing.shown_names(),
        count: listing.count(),
        size: listing.size(),
    }
}

/// Files were copied here: announce them to everyone connected.
fn files_listed(shared: &Arc<Shared>, listing: Result<Listing, String>) {
    let listing = match listing {
        Ok(l) if !l.entries.is_empty() => l,
        Ok(_) => return,
        Err(e) => {
            tracing::warn!("cannot read the copied files: {e}");
            return;
        }
    };
    let offer = {
        let mut state = shared.files.lock().expect("files lock");
        state.counter += 1;
        let id = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
            ^ state.counter.rotate_left(40);
        let offer = files_offer(id, &listing);
        state.local = Some((id, Arc::new(listing)));
        state.remote = None;
        offer
    };
    shared.status.lock().expect("status lock").files_waiting = None;
    for peer in shared.node.status().into_iter().filter(|p| p.link.is_some()) {
        shared.node.send(&peer.peer.fingerprint, offer.clone());
    }
}

/// Asks for files waiting on a peer (the pointer just arrived here).
fn request_files(shared: &Arc<Shared>) {
    let mut state = shared.files.lock().expect("files lock");
    let Some((peer, id, ..)) = state.remote else { return };
    if state.requested == Some(id) {
        return;
    }
    state.requested = Some(id);
    drop(state);
    shared.node.send(&peer, Message::FilesRequest { id });
}

fn update_transfer(shared: &Shared, id: u64, update: impl FnOnce(&mut TransferView)) {
    let mut status = shared.status.lock().expect("status lock");
    if let Some(t) = status.transfers.iter_mut().find(|t| t.id == id) {
        update(t);
    }
    drop(status);
    shared.changed();
}

fn add_transfer(shared: &Shared, view: TransferView) {
    let mut status = shared.status.lock().expect("status lock");
    status.transfers.retain(|t| t.id != view.id);
    status.transfers.push(view);
    if status.transfers.len() > 10 {
        status.transfers.remove(0);
    }
}

fn send_files(shared: &Arc<Shared>, peer: Fingerprint, id: u64) {
    let (listing, cancel) = {
        let mut state = shared.files.lock().expect("files lock");
        match &state.local {
            Some((ours, listing)) if *ours == id => {
                let listing = listing.clone();
                (listing, state.cancels.add(id))
            }
            _ => return,
        }
    };
    add_transfer(
        shared,
        TransferView {
            id,
            peer: shared.name_of(&peer.to_string()),
            incoming: false,
            label: listing.shown_names().join(", "),
            count: listing.count(),
            done: 0,
            total: listing.size(),
            state: TransferState::Running,
        },
    );
    let shared = shared.clone();
    tokio::spawn(async move {
        let progress_shared = shared.clone();
        let mut progress = Progress::new(move |done| update_transfer(&progress_shared, id, |t| t.done = done));
        let result = transfer::send(&shared.node, &peer, id, &listing, &cancel, &mut progress).await;
        let cancelled = cancel.load(std::sync::atomic::Ordering::SeqCst);
        shared.files.lock().expect("files lock").cancels.finished(id);
        update_transfer(&shared, id, |t| {
            t.state = match &result {
                Ok(()) => TransferState::Done(None),
                Err(_) if cancelled => TransferState::Cancelled,
                Err(e) => TransferState::Failed(e.clone()),
            }
        });
    });
}

fn receive_files(shared: &Arc<Shared>, peer: Fingerprint, id: u64, stream: tc_net::InStream) {
    let (label, count, total, cancel) = {
        let mut state = shared.files.lock().expect("files lock");
        match &state.remote {
            Some((from, newest, label, count, size)) if *from == peer && *newest == id => {
                let (label, count, size) = (label.clone(), *count, *size);
                state.remote = None;
                (label, count, size, state.cancels.add(id))
            }
            _ => {
                drop(state);
                stream.cancel();
                return;
            }
        }
    };
    shared.status.lock().expect("status lock").files_waiting = None;
    let peer_name = shared.name_of(&peer.to_string());
    add_transfer(
        shared,
        TransferView {
            id,
            peer: peer_name.clone(),
            incoming: true,
            label,
            count,
            done: 0,
            total,
            state: TransferState::Running,
        },
    );
    let shared = shared.clone();
    tokio::spawn(async move {
        let target = files::download_dir();
        let progress_shared = shared.clone();
        let mut progress = Progress::new(move |done| update_transfer(&progress_shared, id, |t| t.done = done));
        let result = match tokio::fs::create_dir_all(&target).await {
            Ok(()) => transfer::receive(stream, id, &target, &cancel, &mut progress).await,
            Err(e) => Err(e.to_string()),
        };
        let cancelled = cancel.load(std::sync::atomic::Ordering::SeqCst);
        shared.files.lock().expect("files lock").cancels.finished(id);
        match result {
            Ok(placed) => {
                update_transfer(&shared, id, |t| t.state = TransferState::Done(Some(target.clone())));
                shared.files.lock().expect("files lock").written = placed.clone();
                if let Some(clipboard) = &shared.clipboard {
                    let system = clipboard.system.clone();
                    let written = placed.clone();
                    let count = tokio::task::spawn_blocking(move || system.write_files(&written).ok())
                        .await
                        .ok()
                        .flatten();
                    shared.clip.lock().expect("clip lock").write_done(count);
                }
                shared.notice(Notice::FilesReceived(peer_name, count, target));
            }
            Err(_) if cancelled => update_transfer(&shared, id, |t| t.state = TransferState::Cancelled),
            Err(e) => {
                update_transfer(&shared, id, |t| t.state = TransferState::Failed(e.clone()));
                shared.notice(Notice::TransferFailed(peer_name, e));
            }
        }
    });
}

/// Writes content from a peer into the local clipboard (on a blocking thread).
fn clip_write(shared: &Arc<Shared>, peer: Fingerprint, action: Option<ClipAction>) {
    if let (Some(ClipAction::Write { content, .. }), Some(clipboard)) = (action, &shared.clipboard) {
        let system = clipboard.system.clone();
        let internal = shared.internal.clone();
        tokio::task::spawn_blocking(move || {
            let count = system
                .write(&content)
                .map_err(|e| tracing::warn!("clipboard: {e}"))
                .ok();
            let _ = internal.send(Internal::ClipWritten(peer, count));
        });
    }
}

/// Looks whether the local clipboard changed and reads it if so.
fn clip_poll(shared: &Arc<Shared>) {
    let Some(clipboard) = &shared.clipboard else { return };
    let count = clipboard.system.change_count();
    if shared.clip.lock().expect("clip lock").poll(count) == Some(ClipAction::Read) {
        let system = clipboard.system.clone();
        let internal = shared.internal.clone();
        tokio::task::spawn_blocking(move || {
            let read = system.read().map_err(|e| tracing::debug!("clipboard: {e}")).ok();
            let _ = internal.send(Internal::ClipRead(read));
        });
    }
}

fn clip_read(shared: &Arc<Shared>, read: Option<tc_clipboard::Read>) {
    let Some(clipboard) = &shared.clipboard else { return };
    let mut sync = shared.clip.lock().expect("clip lock");
    let Some(read) = read else {
        sync.read_failed();
        return;
    };
    if let tc_clipboard::Read::Files(paths) = &read {
        sync.files_copied();
        drop(sync);
        if shared.files.lock().expect("files lock").written == *paths {
            return; // What a transfer just put there
        }
        let paths = paths.clone();
        let internal = shared.internal.clone();
        tokio::task::spawn_blocking(move || {
            let _ = internal.send(Internal::FilesListed(files::list(&paths).map_err(|e| e.to_string())));
        });
        return;
    }
    let (announcement, skip) = sync.read_done(read, clipboard.max_size);
    drop(sync);
    if let Some(skip) = skip {
        shared.notice(Notice::ClipboardSkipped(skip));
    }
    if let Some((offer, eager)) = announcement {
        for peer in shared.node.status().into_iter().filter(|p| p.link.is_some()) {
            clip::announce(&shared.node, &peer.peer.fingerprint, &offer, eager.as_ref());
        }
    }
}

fn handle_event(shared: &Arc<Shared>, event: Event) {
    match event {
        Event::Connected {
            peer, name, screens, ..
        } => {
            let offer = shared.clip.lock().expect("clip lock").for_new_peer();
            if let Some((offer, eager)) = offer {
                clip::announce(&shared.node, &peer, &offer, eager.as_ref());
            }
            let files_offer = shared
                .files
                .lock()
                .expect("files lock")
                .local
                .as_ref()
                .map(|(id, l)| files_offer(*id, l));
            if let Some(offer) = files_offer {
                shared.node.send(&peer, offer);
            }
            shared
                .engine
                .lock()
                .expect("engine lock")
                .peer_online(&peer.to_string(), &screens);
            shared.reload_layout();
            shared.share_layout_with(&peer);
            shared.notice(Notice::Connected(name));
        }
        Event::Disconnected { peer, reason } => {
            let actions = shared
                .engine
                .lock()
                .expect("engine lock")
                .peer_offline(&peer.to_string());
            shared.execute(actions);
            if !matches!(reason, Disconnect::Replaced) {
                shared.notice(Notice::Disconnected(shared.name_of(&peer.to_string()), reason));
            }
        }
        Event::Message {
            peer,
            message: Message::ClipboardOffer { id, size, .. },
        } => {
            shared.files.lock().expect("files lock").remote = None;
            shared.status.lock().expect("status lock").files_waiting = None;
            let active = !matches!(shared.engine.lock().expect("engine lock").role(), Role::Controlling(_));
            let mut sync = shared.clip.lock().expect("clip lock");
            let request = sync.offered(peer, id, size, active);
            let early = sync
                .take_early(peer, id)
                .and_then(|data| sync.received(peer, id, &data));
            drop(sync);
            if let Some(request) = request {
                shared.node.send(&peer, request);
            }
            clip_write(shared, peer, early);
        }
        Event::Message {
            peer,
            message: Message::FilesOffer { id, names, count, size },
        } => {
            shared.clip.lock().expect("clip lock").clear_remote();
            let label = names.join(", ");
            {
                let mut state = shared.files.lock().expect("files lock");
                state.local = None;
                state.remote = Some((peer, id, label.clone(), count, size));
                state.requested = None;
            }
            shared.status.lock().expect("status lock").files_waiting = Some(label);
            if matches!(shared.engine.lock().expect("engine lock").role(), Role::ControlledBy(_)) {
                request_files(shared);
            }
        }
        Event::Message {
            peer,
            message: Message::FilesRequest { id },
        } => send_files(shared, peer, id),
        Event::Stream { peer, header, stream } => {
            let Some(stream) = stream.take() else { return };
            match files::parse_header(&header) {
                Some(id) => receive_files(shared, peer, id, stream),
                None => stream.cancel(),
            }
        }
        Event::Message {
            peer,
            message: Message::ClipboardRequest { id },
        } => {
            let content = shared.clip.lock().expect("clip lock").requested(id);
            if let Some((header, data)) = content {
                shared.node.send_blob(&peer, header, data.as_ref().clone());
            }
        }
        Event::Blob { peer, header, data } => {
            let action = clip::parse_header(&header)
                .and_then(|id| shared.clip.lock().expect("clip lock").received(peer, id, &data));
            clip_write(shared, peer, action);
        }
        Event::Message {
            peer,
            message: message @ Message::Layout { .. },
        } => shared.layout_received(peer, message),
        Event::Message { peer, message } => {
            let actions = shared
                .engine
                .lock()
                .expect("engine lock")
                .received(&peer.to_string(), message);
            shared.execute(actions);
        }
        Event::Paired { peer } => {
            let mut status = shared.status.lock().expect("status lock");
            status.pairing = None;
            drop(status);
            shared.reload_layout();
            shared.notice(Notice::Paired(peer.name));
        }
        Event::PairingFailed { attempts_left } => shared.notice(Notice::PairingFailed(attempts_left)),
        Event::PairingClosed => {
            shared.status.lock().expect("status lock").pairing = None;
            shared.notice(Notice::PairingClosed);
        }
        Event::Refused { peer, address, reason } => {
            let who = peer.map_or_else(|| address.to_string(), |p| shared.name_of(&p.to_string()));
            shared.notice(Notice::Refused(who, reason));
        }
        Event::Unpaired { peer, by_peer } => {
            let me = shared.node.fingerprint();
            shared.edit_layout(|file| layout_file::place(file, &me, &peer, None));
            shared.notice(Notice::Unpaired(peer.short(), by_peer));
        }
        Event::Woke { .. } => shared.notice(Notice::Woke),
    }
    shared.refresh();
}

fn handle_command(shared: &Arc<Shared>, command: Command) {
    let me = shared.node.fingerprint();
    match command {
        Command::StartPairing => {
            let code = shared.node.start_pairing();
            shared.status.lock().expect("status lock").pairing = Some((code, Instant::now() + tc_net::PAIRING_WINDOW));
        }
        Command::StopPairing => {
            shared.node.stop_pairing();
            shared.status.lock().expect("status lock").pairing = None;
        }
        Command::Pair { code, target } => {
            let Some(code) = PairingCode::parse(&code) else {
                shared.notice(Notice::PairError(PairError::WrongCode));
                return;
            };
            let addresses = match target {
                PairTarget::Address(a) => vec![a],
                PairTarget::Discovered(fp) => shared
                    .node
                    .discovered()
                    .into_iter()
                    .find(|d| d.fingerprint == fp)
                    .map(|d| d.addresses)
                    .unwrap_or_default(),
            };
            shared.status.lock().expect("status lock").pairing_busy = true;
            let node = shared.node.clone();
            let internal = shared.internal.clone();
            tokio::spawn(async move {
                let result = node.pair(&addresses, &code).await.map(|p| p.name);
                let _ = internal.send(Internal::PairDone(result));
            });
        }
        Command::Unpair(peer) => {
            if let Err(e) = shared.node.unpair(&peer) {
                tracing::warn!("cannot remove the pairing: {e}");
            }
        }
        Command::Place { peer, side } => shared.edit_layout(|file| layout_file::place(file, &me, &peer, side)),
        Command::SwapCmdCtrl { peer, swap } => shared.edit_layout(|file| layout_file::set_swap(file, &peer, swap)),
        Command::AddAddress { peer, address } => {
            if let Err(e) = shared.node.add_address(&peer, address) {
                tracing::warn!("cannot save the address: {e}");
            }
        }
        Command::SwitchTo(target) => {
            let target = target.map_or_else(|| me.to_string(), |fp| fp.to_string());
            let actions = shared.engine.lock().expect("engine lock").switch_to(&target);
            shared.execute(actions);
        }
        Command::ReleaseAll => {
            let actions = shared.engine.lock().expect("engine lock").release_everything();
            shared.execute(actions);
            if let Some(inject) = &shared.inject {
                inject.release_all();
            }
            shared.notice(Notice::EmergencyRelease);
        }
        Command::RetryInput | Command::Shutdown | Command::CancelTransfer(_) => unreachable!("handled in the loop"),
    }
    shared.refresh();
    shared.changed();
}
