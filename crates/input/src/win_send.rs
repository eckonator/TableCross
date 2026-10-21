//! Injecting input on Windows with `SendInput`, used by the app and by the agent of the system
//! service; and the app's connection to that agent.

use std::fs::File;
use std::io::Write;
use std::time::{Duration, Instant};

use tc_core::Point;
use tc_protocol::{MediaKey, Message, MouseButton, PressedKeys};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput, VK_NUMLOCK,
    VK_PAUSE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use crate::keymap::windows as keymap;

/// Virtual keys of the media keys (Windows has none for screen brightness).
pub(crate) const MEDIA_KEYS: [(u16, MediaKey); 6] = [
    (0xAF, MediaKey::VolumeUp),
    (0xAE, MediaKey::VolumeDown),
    (0xAD, MediaKey::Mute),
    (0xB3, MediaKey::PlayPause),
    (0xB0, MediaKey::Next),
    (0xB1, MediaKey::Previous),
];

/// Marks input we send ourselves, so the hooks let it through untouched.
pub const OWN_INPUT: usize = 0x7462_6C78; // "tblx"
pub(crate) const WHEEL_DELTA: f64 = 120.0;
/// Wheel units per logical point of precise scrolling (a detent is about 40 points).
const WHEEL_PER_POINT: f64 = 3.0;
/// Pseudo scan code of Pause in the table.
pub(crate) const PAUSE: u16 = 0xE045;
pub(crate) const NUM_LOCK: u16 = 0x45;
const RECONNECT: Duration = Duration::from_secs(2);

/// Keys and buttons sent so far, so everything can be released again.
#[derive(Default)]
pub struct Injector {
    keys: PressedKeys,
    buttons: Vec<MouseButton>,
    wheel_rest: (f64, f64),
}

/// `false` if Windows refused part of it (UIPI, or another desktop is in front).
fn send(inputs: &[INPUT]) -> bool {
    if inputs.is_empty() {
        return true;
    }
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        // Usually UIPI: the window in front runs with higher rights
        tracing::debug!("SendInput delivered {sent} of {} events", inputs.len());
    }
    sent as usize == inputs.len()
}

fn mouse_input(dx: i32, dy: i32, data: i32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data as u32,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: OWN_INPUT,
            },
        },
    }
}

fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: OWN_INPUT,
            },
        },
    }
}

/// Absolute position in the 0..65535 range over the whole virtual desktop.
fn move_input(p: Point) -> INPUT {
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        )
    };
    let nx = ((p.x - f64::from(vx) + 0.5) * 65536.0 / f64::from(vw)) as i32;
    let ny = ((p.y - f64::from(vy) + 0.5) * 65536.0 / f64::from(vh)) as i32;
    mouse_input(
        nx,
        ny,
        0,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    )
}

impl Injector {
    fn key(&mut self, usage: u16, down: bool) {
        let Some(scan) = keymap::from_hid(usage) else {
            return;
        };
        let changed = if down {
            self.keys.press(usage)
        } else {
            self.keys.release(usage)
        };
        // Pressed again while held: auto-repeat, sent as another press like a keyboard does
        if !changed && !down {
            return;
        }
        let up = if down { 0 } else { KEYEVENTF_KEYUP };
        let input = match scan {
            PAUSE => key_input(VK_PAUSE, 0, up),
            NUM_LOCK => key_input(VK_NUMLOCK, 0, up | KEYEVENTF_EXTENDEDKEY),
            _ => {
                let extended = if scan & 0xE000 != 0 { KEYEVENTF_EXTENDEDKEY } else { 0 };
                key_input(0, scan & 0xFF, KEYEVENTF_SCANCODE | extended | up)
            }
        };
        send(&[input]);
    }

    fn button(&mut self, button: MouseButton, down: bool) {
        if down == self.buttons.contains(&button) {
            return;
        }
        if down {
            self.buttons.push(button);
        } else {
            self.buttons.retain(|b| *b != button);
        }
        let (flags, data) = match (button, down) {
            (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
            (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (MouseButton::Back, true) => (MOUSEEVENTF_XDOWN, 1),
            (MouseButton::Back, false) => (MOUSEEVENTF_XUP, 1),
            (MouseButton::Forward, true) => (MOUSEEVENTF_XDOWN, 2),
            (MouseButton::Forward, false) => (MOUSEEVENTF_XUP, 2),
        };
        send(&[mouse_input(0, 0, data, flags)]);
    }

    fn wheel(&mut self, dx: f64, dy: f64, precise: bool) {
        let factor = if precise { WHEEL_PER_POINT } else { WHEEL_DELTA };
        let (rx, ry) = self.wheel_rest;
        let (tx, ty) = (rx + dx * factor, ry + dy * factor);
        let (wx, wy) = (tx.trunc(), ty.trunc());
        self.wheel_rest = (tx - wx, ty - wy);
        let mut inputs = Vec::new();
        if wy != 0.0 {
            inputs.push(mouse_input(0, 0, wy as i32, MOUSEEVENTF_WHEEL));
        }
        if wx != 0.0 {
            inputs.push(mouse_input(0, 0, wx as i32, MOUSEEVENTF_HWHEEL));
        }
        send(&inputs);
    }

    /// `Enter`, `MouseMove`, `Key`, `MouseButton`, `Wheel`, `ReleaseAll`.
    pub fn apply(&mut self, message: &Message) -> bool {
        match *message {
            Message::Enter { x, y } | Message::MouseMove { x, y, .. } => {
                send(&[move_input(Point::new(f64::from(x), f64::from(y)))])
            }
            Message::MouseButton { button, down } => {
                self.button(button, down);
                true
            }
            Message::Key { usage, down } => {
                self.key(usage, down);
                true
            }
            Message::Wheel { dx, dy, precise } => {
                self.wheel(f64::from(dx), f64::from(dy), precise);
                true
            }
            Message::Media { key } => {
                if let Some((vk, _)) = MEDIA_KEYS.iter().find(|(_, k)| *k == key) {
                    send(&[
                        key_input(*vk, 0, KEYEVENTF_EXTENDEDKEY),
                        key_input(*vk, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP),
                    ]);
                }
                true
            }
            Message::ReleaseAll => {
                self.release_all();
                true
            }
            _ => true,
        }
    }

    pub fn release_all(&mut self) {
        for usage in self.keys.clone().release_all() {
            self.key(usage, false);
        }
        for button in self.buttons.clone() {
            self.button(button, false);
        }
    }
}

/// Name of the agent's pipe in a session.
pub fn agent_pipe(session: u32) -> String {
    format!(r"\\.\pipe\tablecross-agent-{session}")
}

/// One message on the agent's pipe: length (u32 little endian) and the encoded message.
pub fn frame(message: &Message) -> Vec<u8> {
    let bytes = tc_protocol::encode(message);
    let mut out = (bytes.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&bytes);
    out
}

/// The app's connection to the service's agent in its own session (if the service is installed).
#[derive(Default)]
pub struct Agent {
    pipe: Option<File>,
    last_try: Option<Instant>,
}

impl Agent {
    /// `true` if the agent took the message.
    pub fn send(&mut self, message: &Message) -> bool {
        if self.pipe.is_none() && self.last_try.is_none_or(|t| t.elapsed() >= RECONNECT) {
            self.last_try = Some(Instant::now());
            let mut session = 0u32;
            if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } != 0 {
                self.pipe = std::fs::OpenOptions::new().write(true).open(agent_pipe(session)).ok();
                if self.pipe.is_some() {
                    tracing::info!("using the TableCross service for input");
                }
            }
        }
        let Some(pipe) = self.pipe.as_mut() else {
            return false;
        };
        if pipe.write_all(&frame(message)).is_err() {
            self.pipe = None;
            return false;
        }
        true
    }
}
