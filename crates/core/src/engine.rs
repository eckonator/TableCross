//! The leader/target state machine.
//!
//! Exactly one machine is in control at a time. The machine whose own mouse and keyboard are used
//! is the *leader*: as long as its pointer is on its own screens, nothing special happens. When the
//! pointer crosses an edge towards a neighbor, the leader grabs its input (suppresses it locally) and
//! tracks the pointer on the neighbor (the *target*), sending it absolute positions, keys, buttons and
//! wheel. The target only injects what it gets. If someone uses the target's own mouse or keyboard,
//! the target takes control back.
//!
//! Stuck keys are prevented by routing: every pressed key and button remembers where its "down" went
//! (the local system and/or a remote machine), and its "up" goes exactly there, wherever the pointer
//! is by then. Modifiers held while crossing are pressed on the new side as well, so Shift-click and
//! similar work across the edge; a mouse button held while crossing is released (a drag ends at the
//! edge). Whenever a link ends, everything the remote side got is released.
//!
//! The engine does no I/O: it takes events and returns [`Action`]s, so all of this is tested without
//! an operating system.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use tc_protocol::{Message, MouseButton, PressedKeys, Screen};

use crate::geometry::{Point, Screens, Side};
use crate::layout::Layout;

pub type MachineId = String;

/// Logical points the local mouse must move while controlled by a remote machine before this
/// machine takes control back. Keys, buttons and wheel take over at once.
const TAKEOVER_MOTION: f64 = 4.0;

/// Input from this machine's own devices.
#[derive(Debug, Clone, PartialEq)]
pub enum Captured {
    /// Pointer at `x`/`y` (meaningless while grabbed), moved by `dx`/`dy`; native units.
    Motion {
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
    },
    Button {
        button: MouseButton,
        down: bool,
    },
    /// USB HID usage (page 7).
    Key {
        usage: u16,
        down: bool,
    },
    /// Same meaning as `Message::Wheel`.
    Wheel {
        dx: f64,
        dy: f64,
        precise: bool,
    },
}

/// What the host has to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Suppress all local input from now on, hide the pointer and keep it at `park`; report motion as
    /// deltas.
    Grab {
        park: Point,
    },
    /// Stop suppressing; show the pointer at `at`.
    Ungrab {
        at: Point,
    },
    Send {
        to: MachineId,
        message: Message,
    },
    /// Inject into the local system: `Enter` (show the pointer there), `MouseMove`, `Key`,
    /// `MouseButton`, `Wheel`.
    Inject(Message),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Dest {
    /// The local system got the "down" (natively, or injected).
    Local,
    Remote(MachineId),
}

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    /// Local input drives the local pointer.
    Local,
    /// Local input drives the pointer on `target`, currently at `pos` (target's native units).
    Remote {
        target: MachineId,
        pos: Point,
        /// Where the local pointer left; it comes back there if the link ends.
        exit: Point,
        /// Native units per logical point of the local screen the pointer left from.
        scale: f64,
    },
    /// A remote machine drives this one.
    Controlled { by: MachineId, last_seq: Option<u32> },
}

/// What the engine is doing, for status displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Idle,
    Controlling(MachineId),
    ControlledBy(MachineId),
}

#[derive(Debug)]
pub struct Engine {
    me: MachineId,
    layout: Layout,
    /// Screens of this machine and of every connected peer.
    machines: HashMap<MachineId, Screens>,
    mode: Mode,
    held_keys: BTreeMap<u16, BTreeSet<Dest>>,
    held_buttons: BTreeMap<MouseButton, BTreeSet<Dest>>,
    injected_keys: PressedKeys,
    injected_buttons: BTreeSet<MouseButton>,
    seq: u32,
    resend: bool,
    takeover_motion: f64,
    /// Last known position of the local pointer (from motion while not grabbed).
    local_pointer: Point,
}

pub fn is_modifier(usage: u16) -> bool {
    (0xE0..=0xE7).contains(&usage)
}

/// Swaps Ctrl and ⌘/Win on both sides.
fn swap_cmd_ctrl(usage: u16) -> u16 {
    match usage {
        0xE0 => 0xE3,
        0xE3 => 0xE0,
        0xE4 => 0xE7,
        0xE7 => 0xE4,
        other => other,
    }
}

/// `seq` is newer than `last`, with wrap-around.
fn newer(seq: u32, last: u32) -> bool {
    (seq.wrapping_sub(last) as i32) > 0
}

impl Engine {
    pub fn new(me: impl Into<MachineId>, screens: &[Screen], layout: Layout) -> Self {
        let me = me.into();
        let mut machines = HashMap::new();
        if let Some(s) = Screens::new(screens) {
            machines.insert(me.clone(), s);
        }
        Self {
            me,
            layout,
            machines,
            mode: Mode::Local,
            held_keys: BTreeMap::new(),
            held_buttons: BTreeMap::new(),
            injected_keys: PressedKeys::new(),
            injected_buttons: BTreeSet::new(),
            seq: 0,
            resend: false,
            takeover_motion: 0.0,
            local_pointer: Point::default(),
        }
    }

    pub fn role(&self) -> Role {
        match &self.mode {
            Mode::Local => Role::Idle,
            Mode::Remote { target, .. } => Role::Controlling(target.clone()),
            Mode::Controlled { by, .. } => Role::ControlledBy(by.clone()),
        }
    }

    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
    }

    pub fn set_local_screens(&mut self, screens: &[Screen]) {
        match Screens::new(screens) {
            Some(s) => self.machines.insert(self.me.clone(), s),
            None => self.machines.remove(&self.me),
        };
    }

    /// A peer connected (or sent new screens).
    pub fn peer_online(&mut self, id: &str, screens: &[Screen]) {
        if id == self.me {
            return;
        }
        match Screens::new(screens) {
            Some(s) => self.machines.insert(id.to_string(), s),
            None => self.machines.remove(id),
        };
    }

    /// A peer's link ended: everything involving it is released.
    pub fn peer_offline(&mut self, id: &str) -> Vec<Action> {
        let mut actions = Vec::new();
        self.machines.remove(id);
        let remote = Dest::Remote(id.to_string());
        for dests in self.held_keys.values_mut().chain(self.held_buttons.values_mut()) {
            dests.remove(&remote);
        }
        match &self.mode {
            Mode::Remote { target, exit, .. } if target == id => {
                let exit = *exit;
                self.return_home(exit, &mut actions);
            }
            Mode::Controlled { by, .. } if by == id => {
                self.release_injected(&mut actions);
                self.mode = Mode::Local;
            }
            _ => {}
        }
        actions
    }

    /// Input from this machine's own devices.
    pub fn captured(&mut self, event: Captured) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Mode::Controlled { by, .. } = &self.mode {
            let by = by.clone();
            if let Captured::Motion { x, y, dx, dy } = event {
                let scale = self
                    .machines
                    .get(&self.me)
                    .map_or(1.0, |s| s.screen_at(Point::new(x, y)).scale);
                self.takeover_motion += dx.hypot(dy) / scale;
                if self.takeover_motion < TAKEOVER_MOTION {
                    return actions;
                }
            }
            // Someone uses this machine: it takes control back
            self.release_injected(&mut actions);
            actions.push(Action::Send {
                to: by,
                message: Message::TakeOver,
            });
            self.mode = Mode::Local;
        }
        match event {
            Captured::Motion { x, y, dx, dy } => self.motion(Point::new(x, y), dx, dy, &mut actions),
            Captured::Key { usage, down } => self.key(usage, down, &mut actions),
            Captured::Button { button, down } => self.button(button, down, &mut actions),
            Captured::Wheel { dx, dy, precise } => {
                if let Mode::Remote { target, .. } = &self.mode {
                    actions.push(Action::Send {
                        to: target.clone(),
                        message: Message::Wheel {
                            dx: dx as f32,
                            dy: dy as f32,
                            precise,
                        },
                    });
                }
            }
        }
        actions
    }

    /// A message from a peer (only input-related messages matter here).
    pub fn received(&mut self, from: &str, message: Message) -> Vec<Action> {
        let mut actions = Vec::new();
        let controlled_by_sender = matches!(&self.mode, Mode::Controlled { by, .. } if by == from);
        match message {
            Message::Enter { x, y } => {
                match &self.mode {
                    Mode::Remote { target, exit, .. } => {
                        // Someone else took the lead while we were leading: step back
                        let (target, exit) = (target.clone(), *exit);
                        self.leave_remote(&target, &mut actions);
                        actions.push(Action::Ungrab { at: exit });
                    }
                    Mode::Controlled { .. } | Mode::Local => {}
                }
                self.release_injected(&mut actions);
                self.mode = Mode::Controlled {
                    by: from.to_string(),
                    last_seq: None,
                };
                self.takeover_motion = 0.0;
                actions.push(Action::Inject(Message::Enter { x, y }));
            }
            Message::MouseMove { seq, .. } if controlled_by_sender => {
                if let Mode::Controlled { last_seq, .. } = &mut self.mode
                    && last_seq.is_none_or(|last| newer(seq, last))
                {
                    *last_seq = Some(seq);
                    actions.push(Action::Inject(message));
                }
            }
            Message::Key { usage, down } if controlled_by_sender => {
                let forward = if down {
                    self.injected_keys.press(usage)
                } else {
                    self.injected_keys.release(usage)
                };
                if forward {
                    actions.push(Action::Inject(message));
                }
            }
            Message::MouseButton { button, down } if controlled_by_sender => {
                let forward = if down {
                    self.injected_buttons.insert(button)
                } else {
                    self.injected_buttons.remove(&button)
                };
                if forward {
                    actions.push(Action::Inject(message));
                }
            }
            Message::Wheel { .. } if controlled_by_sender => actions.push(Action::Inject(message)),
            Message::ReleaseAll if controlled_by_sender => self.release_injected(&mut actions),
            Message::Leave if controlled_by_sender => {
                self.release_injected(&mut actions);
                self.mode = Mode::Local;
            }
            Message::TakeOver => {
                if let Mode::Remote { target, exit, .. } = &self.mode
                    && target == from
                {
                    // The target released everything itself
                    let exit = *exit;
                    let remote = Dest::Remote(from.to_string());
                    for dests in self.held_keys.values_mut().chain(self.held_buttons.values_mut()) {
                        dests.remove(&remote);
                    }
                    self.return_home(exit, &mut actions);
                }
            }
            _ => {}
        }
        actions
    }

    /// Call about every 250 ms: repeats the last position once after the pointer stopped, in case
    /// that datagram was lost.
    pub fn tick(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.resend
            && let Mode::Remote { target, pos, .. } = &self.mode
        {
            let (target, pos) = (target.clone(), *pos);
            self.send_position(&target, pos, 0.0, 0.0, &mut actions);
            self.resend = false;
        }
        actions
    }

    /// Moves control to the neighbor beside the machine the pointer is on now (keyboard shortcut).
    pub fn switch_side(&mut self, side: Side) -> Vec<Action> {
        let current = match &self.mode {
            Mode::Remote { target, .. } => target.clone(),
            _ => self.me.clone(),
        };
        match self.layout.neighbor(&current, side) {
            Some(next) => {
                let next = next.to_string();
                self.switch_to(&next)
            }
            None => Vec::new(),
        }
    }

    /// Puts the pointer in the middle of `machine` (this one or a connected peer).
    pub fn switch_to(&mut self, machine: &str) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Mode::Controlled { by, .. } = &self.mode {
            // Asked for on this machine's own keyboard: take control first
            let by = by.clone();
            self.release_injected(&mut actions);
            actions.push(Action::Send {
                to: by,
                message: Message::TakeOver,
            });
            self.mode = Mode::Local;
        }
        let Some(center) = self.machines.get(machine).map(|s| {
            let r = s.rects()[0];
            Point::new(((r.left + r.right) / 2.0).floor(), ((r.top + r.bottom) / 2.0).floor())
        }) else {
            return actions;
        };
        match self.mode.clone() {
            Mode::Remote { target, .. } if target == machine => {}
            Mode::Remote {
                target, exit, scale, ..
            } => {
                self.leave_remote(&target, &mut actions);
                if machine == self.me {
                    self.mode = Mode::Local;
                    self.sync_modifiers_home(&mut actions);
                    actions.push(Action::Ungrab { at: center });
                } else {
                    self.enter_remote(machine, center, &mut actions);
                    self.mode = Mode::Remote {
                        target: machine.to_string(),
                        pos: center,
                        exit,
                        scale,
                    };
                }
            }
            _ if machine == self.me => {}
            _ => {
                let park = self.local_pointer;
                let scale = self.machines.get(&self.me).map_or(1.0, |s| s.screen_at(park).scale);
                for (button, dests) in &mut self.held_buttons {
                    if dests.remove(&Dest::Local) {
                        actions.push(Action::Inject(Message::MouseButton {
                            button: *button,
                            down: false,
                        }));
                    }
                }
                actions.push(Action::Grab { park });
                self.enter_remote(machine, center, &mut actions);
                self.mode = Mode::Remote {
                    target: machine.to_string(),
                    pos: center,
                    exit: park,
                    scale,
                };
            }
        }
        actions
    }

    /// Emergency: give everything back to the local system and release all keys everywhere.
    pub fn release_everything(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        match &self.mode {
            Mode::Remote { target, exit, .. } => {
                let (target, exit) = (target.clone(), *exit);
                self.leave_remote(&target, &mut actions);
                actions.push(Action::Ungrab { at: exit });
            }
            Mode::Controlled { by, .. } => actions.push(Action::Send {
                to: by.clone(),
                message: Message::TakeOver,
            }),
            Mode::Local => {}
        }
        self.release_injected(&mut actions);
        self.mode = Mode::Local;
        actions
    }

    fn motion(&mut self, p: Point, dx: f64, dy: f64, actions: &mut Vec<Action>) {
        match self.mode.clone() {
            Mode::Local => {
                self.local_pointer = p;
                let Some(mine) = self.machines.get(&self.me) else {
                    return;
                };
                let Some(side) = mine.exit_side(p, Point::new(p.x + dx, p.y + dy)) else {
                    return;
                };
                if mine.corner_distance(side, p) < self.layout.corner {
                    return;
                }
                let Some((next, entry)) = self.neighbor_entry(&self.me, side, p) else {
                    return;
                };
                let scale = mine.screen_at(p).scale;
                // A drag ends at the edge
                for (button, dests) in &mut self.held_buttons {
                    if dests.remove(&Dest::Local) {
                        actions.push(Action::Inject(Message::MouseButton {
                            button: *button,
                            down: false,
                        }));
                    }
                }
                actions.push(Action::Grab { park: p });
                self.enter_remote(&next, entry, actions);
                self.mode = Mode::Remote {
                    target: next,
                    pos: entry,
                    exit: p,
                    scale,
                };
            }
            Mode::Remote {
                target,
                pos,
                exit,
                scale,
            } => {
                let Some(screens) = self.machines.get(&target) else {
                    self.return_home(exit, actions);
                    return;
                };
                let (lx, ly) = (dx / scale, dy / scale);
                let here = screens.screen_at(pos);
                let to = Point::new(pos.x + lx * here.scale, pos.y + ly * here.scale);
                let Some(side) = screens.exit_side(pos, to) else {
                    self.set_position(&target, to, lx, ly, actions);
                    return;
                };
                let clamped = here.clamp(to);
                if screens.corner_distance(side, pos) < self.layout.corner {
                    self.set_position(&target, clamped, lx, ly, actions);
                    return;
                }
                match self.neighbor_entry(&target, side, pos) {
                    Some((next, entry)) if next == self.me => {
                        self.leave_remote(&target, actions);
                        self.mode = Mode::Local;
                        self.sync_modifiers_home(actions);
                        actions.push(Action::Ungrab { at: entry });
                    }
                    Some((next, entry)) => {
                        self.leave_remote(&target, actions);
                        self.enter_remote(&next, entry, actions);
                        self.mode = Mode::Remote {
                            target: next,
                            pos: entry,
                            exit,
                            scale,
                        };
                    }
                    None => self.set_position(&target, clamped, lx, ly, actions),
                }
            }
            Mode::Controlled { .. } => {}
        }
    }

    /// The machine beyond `side` of `machine` and where the pointer enters it.
    fn neighbor_entry(&self, machine: &str, side: Side, p: Point) -> Option<(MachineId, Point)> {
        let next = self.layout.neighbor(machine, side)?;
        let from = self.machines.get(machine)?;
        let to = self.machines.get(next)?;
        let entry = to.entry(side.opposite(), from.along(side, p));
        Some((next.to_string(), entry))
    }

    fn set_position(&mut self, target: &str, pos: Point, lx: f64, ly: f64, actions: &mut Vec<Action>) {
        if let Mode::Remote { pos: current, .. } = &mut self.mode {
            if *current == pos {
                return;
            }
            *current = pos;
        }
        self.send_position(target, pos, lx, ly, actions);
        self.resend = true;
    }

    fn send_position(&mut self, target: &str, pos: Point, lx: f64, ly: f64, actions: &mut Vec<Action>) {
        self.seq = self.seq.wrapping_add(1);
        actions.push(Action::Send {
            to: target.to_string(),
            message: Message::MouseMove {
                seq: self.seq,
                x: pos.x as f32,
                y: pos.y as f32,
                dx: lx as f32,
                dy: ly as f32,
            },
        });
    }

    fn map_key(&self, target: &str, usage: u16) -> u16 {
        if self.layout.swap_cmd_ctrl.contains(target) {
            swap_cmd_ctrl(usage)
        } else {
            usage
        }
    }

    fn key(&mut self, usage: u16, down: bool, actions: &mut Vec<Action>) {
        if !tc_protocol::is_key_usage(usage) {
            return;
        }
        if down {
            if self.held_keys.contains_key(&usage) {
                return; // Auto-repeat: the receiving system repeats on its own
            }
            let dest = match &self.mode {
                Mode::Remote { target, .. } => {
                    let target = target.clone();
                    actions.push(Action::Send {
                        to: target.clone(),
                        message: Message::Key {
                            usage: self.map_key(&target, usage),
                            down: true,
                        },
                    });
                    Dest::Remote(target)
                }
                _ => Dest::Local,
            };
            self.held_keys.insert(usage, BTreeSet::from([dest]));
        } else if let Some(dests) = self.held_keys.remove(&usage) {
            let grabbed = matches!(self.mode, Mode::Remote { .. });
            for dest in dests {
                match dest {
                    // Without grab the local system got the release itself
                    Dest::Local if grabbed => actions.push(Action::Inject(Message::Key { usage, down: false })),
                    Dest::Local => {}
                    Dest::Remote(m) => actions.push(Action::Send {
                        message: Message::Key {
                            usage: self.map_key(&m, usage),
                            down: false,
                        },
                        to: m,
                    }),
                }
            }
        }
    }

    fn button(&mut self, button: MouseButton, down: bool, actions: &mut Vec<Action>) {
        if down {
            if self.held_buttons.contains_key(&button) {
                return;
            }
            let dest = match &self.mode {
                Mode::Remote { target, .. } => {
                    actions.push(Action::Send {
                        to: target.clone(),
                        message: Message::MouseButton { button, down: true },
                    });
                    Dest::Remote(target.clone())
                }
                _ => Dest::Local,
            };
            self.held_buttons.insert(button, BTreeSet::from([dest]));
        } else if let Some(dests) = self.held_buttons.remove(&button) {
            let grabbed = matches!(self.mode, Mode::Remote { .. });
            for dest in dests {
                match dest {
                    Dest::Local if grabbed => {
                        actions.push(Action::Inject(Message::MouseButton { button, down: false }))
                    }
                    Dest::Local => {}
                    Dest::Remote(to) => actions.push(Action::Send {
                        to,
                        message: Message::MouseButton { button, down: false },
                    }),
                }
            }
        }
    }

    /// Tells `target` the pointer arrives and presses the held modifiers there as well.
    fn enter_remote(&mut self, target: &str, entry: Point, actions: &mut Vec<Action>) {
        actions.push(Action::Send {
            to: target.to_string(),
            message: Message::Enter {
                x: entry.x as f32,
                y: entry.y as f32,
            },
        });
        let modifiers: Vec<u16> = self.held_keys.keys().copied().filter(|u| is_modifier(*u)).collect();
        for usage in modifiers {
            actions.push(Action::Send {
                to: target.to_string(),
                message: Message::Key {
                    usage: self.map_key(target, usage),
                    down: true,
                },
            });
            if let Some(dests) = self.held_keys.get_mut(&usage) {
                dests.insert(Dest::Remote(target.to_string()));
            }
        }
        self.resend = false;
    }

    /// Releases on `target` whatever it got from here and says goodbye.
    fn leave_remote(&mut self, target: &str, actions: &mut Vec<Action>) {
        let remote = Dest::Remote(target.to_string());
        let swap = self.layout.swap_cmd_ctrl.contains(target);
        let mut releases = Vec::new();
        for (usage, dests) in &mut self.held_keys {
            if dests.remove(&remote) {
                releases.push(Message::Key {
                    usage: if swap { swap_cmd_ctrl(*usage) } else { *usage },
                    down: false,
                });
            }
        }
        for (button, dests) in &mut self.held_buttons {
            if dests.remove(&remote) {
                releases.push(Message::MouseButton {
                    button: *button,
                    down: false,
                });
            }
        }
        for message in releases {
            actions.push(Action::Send {
                to: target.to_string(),
                message,
            });
        }
        actions.push(Action::Send {
            to: target.to_string(),
            message: Message::Leave,
        });
        self.resend = false;
    }

    /// Back to local control after the target went away or took over.
    fn return_home(&mut self, at: Point, actions: &mut Vec<Action>) {
        self.mode = Mode::Local;
        self.sync_modifiers_home(actions);
        actions.push(Action::Ungrab { at });
        self.resend = false;
    }

    /// Modifiers still held but unknown to the local system (pressed while grabbed) are pressed
    /// locally, so a held Shift keeps working on this side.
    fn sync_modifiers_home(&mut self, actions: &mut Vec<Action>) {
        for (usage, dests) in &mut self.held_keys {
            if is_modifier(*usage) && !dests.contains(&Dest::Local) {
                dests.insert(Dest::Local);
                actions.push(Action::Inject(Message::Key {
                    usage: *usage,
                    down: true,
                }));
            }
        }
    }

    fn release_injected(&mut self, actions: &mut Vec<Action>) {
        for usage in self.injected_keys.release_all() {
            actions.push(Action::Inject(Message::Key { usage, down: false }));
        }
        for button in std::mem::take(&mut self.injected_buttons) {
            actions.push(Action::Inject(Message::MouseButton { button, down: false }));
        }
    }
}
