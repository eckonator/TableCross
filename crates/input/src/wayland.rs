//! Linux with Wayland (GNOME 45+, KDE Plasma 6.1+): portals and libei.
//!
//! - Injecting: the RemoteDesktop portal hands out an EI connection; the permission is asked once and
//!   remembered with a restore token.
//! - Capturing: the InputCapture portal puts barriers on the outer screen edges. When the pointer
//!   hits one, the compositor stops local input and sends everything over EI until we release it.
//!   Outside of that, a Wayland session tells no program about input, so using this computer's own
//!   mouse while it is controlled cannot take control back.
//!
//! Portals are asynchronous: everything runs on a small Tokio runtime of its own; the traits talk
//! to it through a channel. Screen coordinates are the compositor's logical pixels.

use std::collections::HashSet;
use std::num::NonZeroU32;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ashpd::desktop::input_capture::{
    Barrier, Capabilities, ConnectToEISOptions as CaptureEisOptions, CreateSessionOptions as CaptureSessionOptions,
    EnableOptions, GetZonesOptions, InputCapture, ReleaseOptions, SetPointerBarriersOptions,
};
use ashpd::desktop::remote_desktop::{
    ConnectToEISOptions, DeviceType, RemoteDesktop, SelectDevicesOptions, StartOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, Session};
use futures_util::StreamExt;
use reis::ei;
use reis::event::{Device, DeviceCapability, EiEvent};
use tc_core::{Captured, Point};
use tc_protocol::{Message, MouseButton, PressedKeys, Screen};
use tokio::sync::mpsc;

use crate::keymap::linux as keymap;
use crate::{Backend, Capture, Handler, Health, Inject, InputError, Problem};

/// Linux button codes (input-event-codes.h).
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;
/// EI discrete scrolling counts 120 per wheel detent.
const DETENT: f32 = 120.0;

enum Cmd {
    StartCapture(Handler),
    Inject(Message),
    Grab,
    Ungrab(Point),
    ReleaseAll,
}

#[derive(Default)]
struct Shared {
    screens: Mutex<Vec<Screen>>,
    grabbed: AtomicBool,
    capturing: AtomicBool,
    problems: Mutex<Vec<Problem>>,
}

pub fn backend() -> Result<Backend, InputError> {
    let (tx, rx) = mpsc::unbounded_channel();
    let shared = Arc::new(Shared::default());
    let task_shared = shared.clone();
    std::thread::Builder::new()
        .name("tablecross-wayland".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(r) => r,
                Err(e) => {
                    tracing::error!("no runtime for the portals: {e}");
                    return;
                }
            };
            runtime.block_on(run(rx, task_shared));
        })
        .map_err(|e| InputError::System(e.to_string()))?;
    Ok(Backend {
        capture: Box::new(WaylandCapture {
            tx: tx.clone(),
            shared: shared.clone(),
        }),
        inject: Arc::new(WaylandInject { tx, shared }),
    })
}

fn now_us() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64)
}

/// Where the RemoteDesktop permission is remembered.
fn token_file() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("tablecross/remote-desktop-token"))
}

/// Injecting through the RemoteDesktop portal.
struct Sender {
    connection: reis::event::Connection,
    devices: Vec<Device>,
    resumed: HashSet<Device>,
    sequence: u32,
    keys: PressedKeys,
    buttons: Vec<MouseButton>,
    position: Option<(f32, f32)>,
}

async fn open_sender() -> Result<(Sender, reis::tokio::EiConvertEventStream, Session<RemoteDesktop>), ashpd::Error> {
    let portal = RemoteDesktop::new().await?;
    let session = portal.create_session(CreateSessionOptions::default()).await?;
    let token = token_file().and_then(|f| std::fs::read_to_string(f).ok());
    portal
        .select_devices(
            &session,
            SelectDevicesOptions::default()
                .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
                .set_persist_mode(PersistMode::ExplicitlyRevoked)
                .set_restore_token(token.as_deref()),
        )
        .await?;
    let started = portal.start(&session, None, StartOptions::default()).await?.response()?;
    if let (Some(token), Some(file)) = (started.restore_token(), token_file()) {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, token);
    }
    let fd = portal.connect_to_eis(&session, ConnectToEISOptions::default()).await?;
    let context = ei::Context::new(UnixStream::from(fd)).map_err(|e| ashpd::Error::IO(e.into()))?;
    let (connection, events) = context
        .handshake_tokio("tablecross", ei::handshake::ContextType::Sender)
        .await
        .map_err(|e| ashpd::Error::IO(std::io::Error::other(e.to_string())))?;
    Ok((
        Sender {
            connection,
            devices: Vec::new(),
            resumed: HashSet::new(),
            sequence: 0,
            keys: PressedKeys::new(),
            buttons: Vec::new(),
            position: None,
        },
        events,
        session,
    ))
}

impl Sender {
    fn event(&mut self, event: EiEvent, shared: &Shared) {
        match event {
            EiEvent::SeatAdded(e) => {
                e.seat.bind_capabilities(
                    DeviceCapability::PointerAbsolute
                        | DeviceCapability::Pointer
                        | DeviceCapability::Keyboard
                        | DeviceCapability::Scroll
                        | DeviceCapability::Button,
                );
            }
            EiEvent::DeviceAdded(e) => {
                if e.device.has_capability(DeviceCapability::PointerAbsolute) {
                    let screens = e
                        .device
                        .regions()
                        .iter()
                        .map(|r| Screen {
                            x: r.x as i32,
                            y: r.y as i32,
                            width: r.width,
                            height: r.height,
                            scale: 1.0,
                        })
                        .collect();
                    *shared.screens.lock().expect("screens lock") = screens;
                }
                self.devices.push(e.device);
            }
            EiEvent::DeviceRemoved(e) => {
                self.devices.retain(|d| *d != e.device);
                self.resumed.remove(&e.device);
            }
            EiEvent::DeviceResumed(e) => {
                e.device.device().start_emulating(e.serial, self.sequence);
                self.sequence = self.sequence.wrapping_add(1);
                self.resumed.insert(e.device);
            }
            EiEvent::DevicePaused(e) => {
                self.resumed.remove(&e.device);
            }
            _ => {}
        }
        let _ = self.connection.flush();
    }

    fn device(&self, capability: DeviceCapability) -> Option<&Device> {
        self.devices
            .iter()
            .find(|d| self.resumed.contains(*d) && d.has_capability(capability))
    }

    fn frame(&self, device: &Device) {
        device.device().frame(self.connection.serial(), now_us());
        let _ = self.connection.flush();
    }

    fn inject(&mut self, message: &Message) {
        match *message {
            Message::Enter { x, y } | Message::MouseMove { x, y, .. } => {
                if let Some(device) = self.device(DeviceCapability::PointerAbsolute) {
                    if let Some(pointer) = device.interface::<ei::PointerAbsolute>() {
                        pointer.motion_absolute(x, y);
                        self.frame(device);
                    }
                } else if let Some(device) = self.device(DeviceCapability::Pointer) {
                    // Only relative motion offered: move by the difference
                    let (px, py) = self.position.unwrap_or((x, y));
                    if let Some(pointer) = device.interface::<ei::Pointer>() {
                        pointer.motion_relative(x - px, y - py);
                        self.frame(device);
                    }
                }
                self.position = Some((x, y));
            }
            Message::Key { usage, down } => {
                let Some(code) = keymap::from_hid(usage) else { return };
                let changed = if down { self.keys.press(usage) } else { self.keys.release(usage) };
                if !changed {
                    return;
                }
                if let Some(device) = self.device(DeviceCapability::Keyboard)
                    && let Some(keyboard) = device.interface::<ei::Keyboard>()
                {
                    let state = if down { ei::keyboard::KeyState::Press } else { ei::keyboard::KeyState::Released };
                    keyboard.key(u32::from(code), state);
                    self.frame(device);
                }
            }
            Message::Media { key } => {
                if let Some(device) = self.device(DeviceCapability::Keyboard)
                    && let Some(keyboard) = device.interface::<ei::Keyboard>()
                {
                    let code = u32::from(keymap::media_code(key));
                    keyboard.key(code, ei::keyboard::KeyState::Press);
                    self.frame(device);
                    keyboard.key(code, ei::keyboard::KeyState::Released);
                    self.frame(device);
                }
            }
            Message::MouseButton { button, down } => {
                if down == self.buttons.contains(&button) {
                    return;
                }
                if down {
                    self.buttons.push(button);
                } else {
                    self.buttons.retain(|b| *b != button);
                }
                let code = match button {
                    MouseButton::Left => BTN_LEFT,
                    MouseButton::Right => BTN_RIGHT,
                    MouseButton::Middle => BTN_MIDDLE,
                    MouseButton::Back => BTN_SIDE,
                    MouseButton::Forward => BTN_EXTRA,
                };
                if let Some(device) = self.device(DeviceCapability::Button)
                    && let Some(buttons) = device.interface::<ei::Button>()
                {
                    let state =
                        if down { ei::button::ButtonState::Press } else { ei::button::ButtonState::Released };
                    buttons.button(code, state);
                    self.frame(device);
                }
            }
            Message::Wheel { dx, dy, precise } => {
                if let Some(device) = self.device(DeviceCapability::Scroll)
                    && let Some(scroll) = device.interface::<ei::Scroll>()
                {
                    // EI counts down and right as positive
                    if precise {
                        scroll.scroll(dx, -dy);
                    } else {
                        scroll.scroll_discrete((dx * DETENT) as i32, (-dy * DETENT) as i32);
                    }
                    self.frame(device);
                }
            }
            _ => {}
        }
    }

    fn release_all(&mut self) {
        for usage in self.keys.clone().release_all() {
            self.inject(&Message::Key { usage, down: false });
        }
        for button in self.buttons.clone() {
            self.inject(&Message::MouseButton { button, down: false });
        }
    }
}

/// Capturing through the InputCapture portal.
struct Receiver {
    portal: InputCapture,
    session: Session<InputCapture>,
    zones: Vec<(i32, i32, i32, i32)>,
    activation: Option<u32>,
    park: (f64, f64),
    handler: Handler,
}

/// Barriers on every zone edge that has no other zone behind it.
fn barriers(zones: &[(i32, i32, i32, i32)]) -> Vec<Barrier> {
    let touches = |a: &(i32, i32, i32, i32), b: &(i32, i32, i32, i32), side: u8| {
        let (ax, ay, aw, ah) = *a;
        let (bx, by, bw, bh) = *b;
        let overlap_y = ay < by + bh && by < ay + ah;
        let overlap_x = ax < bx + bw && bx < ax + aw;
        match side {
            0 => overlap_y && bx + bw == ax,
            1 => overlap_y && bx == ax + aw,
            2 => overlap_x && by + bh == ay,
            _ => overlap_x && by == ay + ah,
        }
    };
    let mut out = Vec::new();
    for (i, zone) in zones.iter().enumerate() {
        let (x, y, w, h) = *zone;
        for side in 0..4u8 {
            if zones.iter().enumerate().any(|(j, other)| j != i && touches(zone, other, side)) {
                continue;
            }
            let position = match side {
                0 => (x, y, x, y + h - 1),
                1 => (x + w, y, x + w, y + h - 1),
                2 => (x, y, x + w - 1, y),
                _ => (x, y + h, x + w - 1, y + h),
            };
            if let Some(id) = NonZeroU32::new(out.len() as u32 + 1) {
                out.push(Barrier::new(id, position));
            }
        }
    }
    out
}

async fn open_receiver(
    handler: Handler,
) -> Result<(Receiver, reis::tokio::EiConvertEventStream), ashpd::Error> {
    let portal = InputCapture::new().await?;
    let (session, _) = portal
        .create_session(
            None,
            CaptureSessionOptions::default().set_capabilities(Capabilities::Keyboard | Capabilities::Pointer),
        )
        .await?;
    let zones_reply = portal.zones(&session, GetZonesOptions::default()).await?.response()?;
    let zones: Vec<_> = zones_reply
        .regions()
        .iter()
        .map(|r| (r.x_offset(), r.y_offset(), r.width() as i32, r.height() as i32))
        .collect();
    portal
        .set_pointer_barriers(
            &session,
            &barriers(&zones),
            zones_reply.zone_set(),
            SetPointerBarriersOptions::default(),
        )
        .await?
        .response()?;
    let fd = portal.connect_to_eis(&session, CaptureEisOptions::default()).await?;
    let context = ei::Context::new(UnixStream::from(fd)).map_err(|e| ashpd::Error::IO(e.into()))?;
    let (_connection, events) = context
        .handshake_tokio("tablecross", ei::handshake::ContextType::Receiver)
        .await
        .map_err(|e| ashpd::Error::IO(std::io::Error::other(e.to_string())))?;
    portal.enable(&session, EnableOptions::default()).await?;
    Ok((
        Receiver {
            portal,
            session,
            zones,
            activation: None,
            park: (0.0, 0.0),
            handler,
        },
        events,
    ))
}

impl Receiver {
    /// The pointer hit a barrier: tell the engine as a push over that edge.
    async fn activated(&mut self, activation: Option<u32>, at: (f32, f32), shared: &Shared) {
        self.activation = activation;
        let (x, y) = (f64::from(at.0), f64::from(at.1));
        self.park = (x, y);
        let zone = self
            .zones
            .iter()
            .find(|(zx, zy, zw, zh)| {
                x >= f64::from(*zx) - 1.0
                    && x <= f64::from(zx + zw)
                    && y >= f64::from(*zy) - 1.0
                    && y <= f64::from(zy + zh)
            })
            .copied();
        let (dx, dy) = match zone {
            Some((zx, zy, zw, zh)) => {
                let (left, right) = (x - f64::from(zx), f64::from(zx + zw) - x);
                let (top, bottom) = (y - f64::from(zy), f64::from(zy + zh) - y);
                let nearest = left.min(right).min(top).min(bottom);
                if nearest == left {
                    (-2.0, 0.0)
                } else if nearest == right {
                    (2.0, 0.0)
                } else if nearest == top {
                    (0.0, -2.0)
                } else {
                    (0.0, 2.0)
                }
            }
            None => (0.0, 0.0),
        };
        let _ = (self.handler)(Captured::Motion { x, y, dx, dy });
        if !shared.grabbed.load(Ordering::SeqCst) {
            // No computer behind this edge: give the pointer back at once
            self.release(Some((x, y))).await;
        }
    }

    async fn release(&mut self, at: Option<(f64, f64)>) {
        let options = ReleaseOptions::default()
            .set_activation_id(self.activation)
            .set_cursor_position(at);
        if let Err(e) = self.portal.release(&self.session, options).await {
            tracing::warn!("cannot release the pointer: {e}");
        }
        self.activation = None;
    }

    fn event(&mut self, event: EiEvent) {
        let captured = match event {
            EiEvent::SeatAdded(e) => {
                e.seat.bind_capabilities(
                    DeviceCapability::Pointer
                        | DeviceCapability::Keyboard
                        | DeviceCapability::Scroll
                        | DeviceCapability::Button,
                );
                None
            }
            EiEvent::PointerMotion(e) => Some(Captured::Motion {
                x: self.park.0,
                y: self.park.1,
                dx: f64::from(e.dx),
                dy: f64::from(e.dy),
            }),
            EiEvent::Button(e) => {
                let button = match e.button {
                    BTN_LEFT => Some(MouseButton::Left),
                    BTN_RIGHT => Some(MouseButton::Right),
                    BTN_MIDDLE => Some(MouseButton::Middle),
                    BTN_SIDE => Some(MouseButton::Back),
                    BTN_EXTRA => Some(MouseButton::Forward),
                    _ => None,
                };
                button.map(|button| Captured::Button {
                    button,
                    down: e.state == ei::button::ButtonState::Press,
                })
            }
            EiEvent::ScrollDiscrete(e) => Some(Captured::Wheel {
                dx: f64::from(e.discrete_dx) / f64::from(DETENT),
                dy: -f64::from(e.discrete_dy) / f64::from(DETENT),
                precise: false,
            }),
            EiEvent::ScrollDelta(e) => Some(Captured::Wheel {
                dx: f64::from(e.dx),
                dy: -f64::from(e.dy),
                precise: true,
            }),
            EiEvent::KeyboardKey(e) => {
                let down = e.state == ei::keyboard::KeyState::Press;
                u16::try_from(e.key).ok().and_then(|code| match keymap::media(code) {
                    Some(key) => Some(Captured::Media { key, down }),
                    None => keymap::to_hid(code).map(|usage| Captured::Key { usage, down }),
                })
            }
            _ => None,
        };
        if let Some(captured) = captured {
            let _ = (self.handler)(captured);
        }
    }
}

async fn run(mut rx: mpsc::UnboundedReceiver<Cmd>, shared: Arc<Shared>) {
    let (mut sender, mut sender_events, _sender_session) = match open_sender().await {
        Ok((s, e, session)) => (Some(s), Some(e), Some(session)),
        Err(e) => {
            tracing::warn!("no remote control portal: {e}");
            shared.problems.lock().expect("problems lock").push(Problem::CaptureStopped);
            (None, None, None)
        }
    };
    let mut receiver: Option<Receiver> = None;
    let mut receiver_events: Option<reis::tokio::EiConvertEventStream> = None;
    let mut activated = None;
    loop {
        tokio::select! {
            command = rx.recv() => {
                let Some(command) = command else { break };
                match command {
                    Cmd::StartCapture(handler) => match open_receiver(handler).await {
                        Ok((r, events)) => {
                            activated = r.portal.receive_activated().await.ok().map(Box::pin);
                            receiver = Some(r);
                            receiver_events = Some(events);
                            shared.capturing.store(true, Ordering::SeqCst);
                        }
                        Err(e) => {
                            tracing::warn!("no input capture portal: {e}");
                            shared.problems.lock().expect("problems lock").push(Problem::CaptureStopped);
                        }
                    },
                    Cmd::Inject(message) => {
                        if let Some(sender) = sender.as_mut() {
                            sender.inject(&message);
                        }
                    }
                    Cmd::Grab => shared.grabbed.store(true, Ordering::SeqCst),
                    Cmd::Ungrab(at) => {
                        shared.grabbed.store(false, Ordering::SeqCst);
                        if let Some(r) = receiver.as_mut()
                            && r.activation.is_some()
                        {
                            r.release(Some((at.x, at.y))).await;
                        }
                    }
                    Cmd::ReleaseAll => {
                        if let Some(sender) = sender.as_mut() {
                            sender.release_all();
                        }
                    }
                }
            }
            Some(Ok(event)) = async { match sender_events.as_mut() { Some(s) => s.next().await, None => None } } => {
                if let Some(sender) = sender.as_mut() {
                    sender.event(event, &shared);
                }
            }
            Some(Ok(event)) = async { match receiver_events.as_mut() { Some(s) => s.next().await, None => None } } => {
                if let Some(r) = receiver.as_mut() {
                    r.event(event);
                }
            }
            Some(signal) = async { match activated.as_mut() { Some(s) => s.next().await, None => None } } => {
                if let Some(r) = receiver.as_mut() {
                    let at = signal.cursor_position().unwrap_or((0.0, 0.0));
                    r.activated(signal.activation_id(), at, &shared).await;
                }
            }
        }
    }
}

pub struct WaylandCapture {
    tx: mpsc::UnboundedSender<Cmd>,
    shared: Arc<Shared>,
}

impl Capture for WaylandCapture {
    fn start(&mut self, handler: Handler) -> Result<(), InputError> {
        self.tx
            .send(Cmd::StartCapture(handler))
            .map_err(|_| InputError::System("the portal thread stopped".into()))
    }

    fn stop(&mut self) {}

    fn health(&self) -> Health {
        Health {
            capturing: self.shared.capturing.load(Ordering::SeqCst),
            problems: self.shared.problems.lock().expect("problems lock").clone(),
        }
    }
}

pub struct WaylandInject {
    tx: mpsc::UnboundedSender<Cmd>,
    shared: Arc<Shared>,
}

impl Inject for WaylandInject {
    fn grab(&self, _park: Point) -> Result<(), InputError> {
        // The compositor already holds the input since the barrier was hit
        self.shared.grabbed.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Cmd::Grab);
        Ok(())
    }

    fn ungrab(&self, at: Point) -> Result<(), InputError> {
        self.shared.grabbed.store(false, Ordering::SeqCst);
        let _ = self.tx.send(Cmd::Ungrab(at));
        Ok(())
    }

    fn inject(&self, message: &Message) -> Result<(), InputError> {
        let _ = self.tx.send(Cmd::Inject(message.clone()));
        Ok(())
    }

    fn release_all(&self) {
        let _ = self.tx.send(Cmd::ReleaseAll);
    }

    fn screens(&self) -> Vec<Screen> {
        self.shared.screens.lock().expect("screens lock").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barriers_only_on_open_edges() {
        // Two screens side by side: the shared edge gets no barrier
        let zones = [(0, 0, 1920, 1080), (1920, 0, 1280, 1024)];
        let positions: Vec<_> = barriers(&zones).iter().map(|b| b.position().as_tuple()).collect();
        assert_eq!(positions.len(), 6);
        assert!(positions.contains(&(0, 0, 0, 1079)));
        assert!(positions.contains(&(3200, 0, 3200, 1023)));
        assert!(!positions.contains(&(1920, 0, 1920, 1079)));
        assert!(positions.contains(&(0, 1080, 1919, 1080)));
    }
}
