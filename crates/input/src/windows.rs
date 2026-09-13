//! Windows: capture with low-level hooks, inject with `SendInput` (scan codes, absolute positions).
//!
//! While grabbed, the pointer is parked in the middle of its screen and hidden; every movement is
//! blocked and reported as the distance from the parking spot (with the user's pointer speed and
//! acceleration already applied). Windows removes hooks that answer too slowly without telling
//! anyone, so a timer compares the system's last input with the hooks' and installs them again.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use tc_core::{Captured, Point};
use tc_protocol::{Message, MouseButton, Screen};
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetLastInputInfo, LASTINPUTINFO, VK_NUMLOCK, VK_PAUSE, VK_SNAPSHOT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CopyIcon, CreateCursor, DispatchMessageW, GetCursorPos, GetMessageW, HC_ACTION, HCURSOR, HHOOK,
    KBDLLHOOKSTRUCT, KillTimer, LLKHF_EXTENDED, MSG, MSLLHOOKSTRUCT, PostThreadMessageW, SPI_SETCURSORS, SetCursorPos,
    SetSystemCursor, SetTimer, SetWindowsHookExW, SystemParametersInfoW, TranslateMessage, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_TIMER,
    WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use crate::keymap::windows as keymap;
use crate::win_send::{Agent, Injector, NUM_LOCK, OWN_INPUT, PAUSE, WHEEL_DELTA};
use crate::{Backend, Capture, Handler, Health, Inject, InputError, Problem};

/// Ctrl that Windows adds in front of AltGr on layouts that have it; not a real key.
const FAKE_ALTGR_CTRL: u32 = 0x21D;
const HOOK_CHECK_MS: u32 = 1000;
/// Standard cursor ids that get the empty cursor while grabbed.
const CURSOR_IDS: [u32; 13] = [
    32512, 32513, 32514, 32515, 32516, 32642, 32643, 32644, 32645, 32646, 32648, 32649, 32650,
];

struct Shared {
    grabbed: AtomicBool,
    park_x: AtomicI32,
    park_y: AtomicI32,
    /// The pointer was put back at `park_x`/`park_y` on ungrab; motion is measured from there.
    returned: AtomicBool,
    hook_thread: AtomicU32,
    /// `GetTickCount` of the last event the hooks saw.
    last_hook_event: AtomicU32,
    hooks_reinstalled: AtomicU32,
    inject: Mutex<InjectState>,
}

#[derive(Default)]
struct InjectState {
    injector: Injector,
    cursor_hidden: bool,
    agent: Agent,
}

pub fn backend() -> Result<Backend, InputError> {
    unsafe {
        // Pixels, not scaled coordinates, everywhere
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // A crash while grabbed could have left the empty cursor behind
        SystemParametersInfoW(SPI_SETCURSORS, 0, std::ptr::null_mut(), 0);
    }
    let shared = Arc::new(Shared {
        grabbed: AtomicBool::new(false),
        park_x: AtomicI32::new(0),
        park_y: AtomicI32::new(0),
        returned: AtomicBool::new(false),
        hook_thread: AtomicU32::new(0),
        last_hook_event: AtomicU32::new(0),
        hooks_reinstalled: AtomicU32::new(0),
        inject: Mutex::new(InjectState::default()),
    });
    Ok(Backend {
        capture: Box::new(WinCapture { shared: shared.clone() }),
        inject: Arc::new(WinInject { shared }),
    })
}

struct HookContext {
    handler: Handler,
    shared: Arc<Shared>,
    last: POINT,
    keyboard: HHOOK,
    mouse: HHOOK,
}

thread_local! {
    static CONTEXT: RefCell<Option<HookContext>> = const { RefCell::new(None) };
}

fn tick_count() -> u32 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}

pub struct WinCapture {
    shared: Arc<Shared>,
}

fn install_hooks() -> Option<(HHOOK, HHOOK)> {
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0);
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0);
        if keyboard.is_null() || mouse.is_null() {
            if !keyboard.is_null() {
                UnhookWindowsHookEx(keyboard);
            }
            if !mouse.is_null() {
                UnhookWindowsHookEx(mouse);
            }
            return None;
        }
        Some((keyboard, mouse))
    }
}

impl Capture for WinCapture {
    fn start(&mut self, handler: Handler) -> Result<(), InputError> {
        let shared = self.shared.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("tablecross-capture".into())
            .spawn(move || unsafe {
                let Some((keyboard, mouse)) = install_hooks() else {
                    let _ = ready_tx.send(false);
                    return;
                };
                let mut last = POINT { x: 0, y: 0 };
                GetCursorPos(&mut last);
                CONTEXT.with(|c| {
                    *c.borrow_mut() = Some(HookContext {
                        handler,
                        shared: shared.clone(),
                        last,
                        keyboard,
                        mouse,
                    })
                });
                shared.hook_thread.store(GetCurrentThreadId(), Ordering::SeqCst);
                shared.last_hook_event.store(tick_count(), Ordering::SeqCst);
                let timer = SetTimer(std::ptr::null_mut(), 0, HOOK_CHECK_MS, None);
                let _ = ready_tx.send(true);
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    if msg.message == WM_TIMER {
                        check_hooks();
                        continue;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                KillTimer(std::ptr::null_mut(), timer);
                CONTEXT.with(|c| {
                    if let Some(context) = c.borrow_mut().take() {
                        UnhookWindowsHookEx(context.keyboard);
                        UnhookWindowsHookEx(context.mouse);
                    }
                });
                shared.hook_thread.store(0, Ordering::SeqCst);
            })
            .map_err(|e| InputError::System(e.to_string()))?;
        match ready_rx.recv() {
            Ok(true) => Ok(()),
            _ => Err(InputError::System("cannot install the input hooks".into())),
        }
    }

    fn stop(&mut self) {
        let thread = self.shared.hook_thread.load(Ordering::SeqCst);
        if thread != 0 {
            unsafe { PostThreadMessageW(thread, WM_QUIT, 0, 0) };
        }
    }

    fn health(&self) -> Health {
        let capturing = self.shared.hook_thread.load(Ordering::SeqCst) != 0;
        let mut problems = Vec::new();
        if !capturing {
            problems.push(Problem::CaptureStopped);
        }
        let reinstalled = self.shared.hooks_reinstalled.load(Ordering::SeqCst);
        if reinstalled > 0 {
            problems.push(Problem::HooksRemoved(reinstalled));
        }
        Health { capturing, problems }
    }
}

/// The system saw input the hooks did not: Windows removed them. Install them again.
fn check_hooks() {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return;
    }
    CONTEXT.with(|c| {
        let mut context = c.borrow_mut();
        let Some(context) = context.as_mut() else {
            return;
        };
        let last_hook = context.shared.last_hook_event.load(Ordering::SeqCst);
        if info.dwTime.wrapping_sub(last_hook) as i32 <= 500 {
            return;
        }
        tracing::warn!("input hooks stopped receiving events, installing them again");
        unsafe {
            UnhookWindowsHookEx(context.keyboard);
            UnhookWindowsHookEx(context.mouse);
        }
        if let Some((keyboard, mouse)) = install_hooks() {
            context.keyboard = keyboard;
            context.mouse = mouse;
        }
        context.shared.last_hook_event.store(tick_count(), Ordering::SeqCst);
        context.shared.hooks_reinstalled.fetch_add(1, Ordering::SeqCst);
    });
}

fn mark_alive() {
    CONTEXT.with(|c| {
        if let Ok(context) = c.try_borrow()
            && let Some(context) = context.as_ref()
        {
            context.shared.last_hook_event.store(tick_count(), Ordering::SeqCst);
        }
    });
}

/// Runs the handler; `true` means the event goes on to the system. A hook call while the handler
/// is still running (re-entered through a nested message loop) is let through.
fn handle(events: &[Captured]) -> bool {
    CONTEXT.with(|c| {
        let Ok(mut context) = c.try_borrow_mut() else {
            return true;
        };
        let Some(context) = context.as_mut() else {
            return true;
        };
        let mut pass = true;
        for event in events {
            pass &= (context.handler)(event.clone());
        }
        pass
    })
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code != HC_ACTION as i32 {
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }
        let info = &*(lparam as *const KBDLLHOOKSTRUCT);
        // Every event counts as sign of life, our own injected ones too (they update the system's
        // last-input time as well)
        mark_alive();
        if info.dwExtraInfo == OWN_INPUT || info.scanCode == FAKE_ALTGR_CTRL {
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }
        let scan = match info.vkCode as u16 {
            VK_PAUSE => PAUSE,
            VK_NUMLOCK => NUM_LOCK,
            VK_SNAPSHOT => 0xE037,
            _ => {
                let extended = if info.flags & LLKHF_EXTENDED != 0 { 0xE000 } else { 0 };
                (info.scanCode as u16 & 0xFF) | extended
            }
        };
        let Some(usage) = keymap::to_hid(scan) else {
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        };
        let down = matches!(wparam as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let pass = std::panic::catch_unwind(|| handle(&[Captured::Key { usage, down }])).unwrap_or(true);
        if pass {
            CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
        } else {
            1
        }
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if code != HC_ACTION as i32 {
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }
        let info = &*(lparam as *const MSLLHOOKSTRUCT);
        // Every event counts as sign of life, our own injected ones too (they update the system's
        // last-input time as well)
        mark_alive();
        if info.dwExtraInfo == OWN_INPUT {
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }
        let wheel = f64::from((info.mouseData >> 16) as i16) / WHEEL_DELTA;
        let x_button = || {
            if (info.mouseData >> 16) == 1 {
                MouseButton::Back
            } else {
                MouseButton::Forward
            }
        };
        let event = match wparam as u32 {
            WM_MOUSEMOVE => Some(motion(info.pt)),
            WM_LBUTTONDOWN => Some(Captured::Button {
                button: MouseButton::Left,
                down: true,
            }),
            WM_LBUTTONUP => Some(Captured::Button {
                button: MouseButton::Left,
                down: false,
            }),
            WM_RBUTTONDOWN => Some(Captured::Button {
                button: MouseButton::Right,
                down: true,
            }),
            WM_RBUTTONUP => Some(Captured::Button {
                button: MouseButton::Right,
                down: false,
            }),
            WM_MBUTTONDOWN => Some(Captured::Button {
                button: MouseButton::Middle,
                down: true,
            }),
            WM_MBUTTONUP => Some(Captured::Button {
                button: MouseButton::Middle,
                down: false,
            }),
            WM_XBUTTONDOWN => Some(Captured::Button {
                button: x_button(),
                down: true,
            }),
            WM_XBUTTONUP => Some(Captured::Button {
                button: x_button(),
                down: false,
            }),
            WM_MOUSEWHEEL => Some(Captured::Wheel {
                dx: 0.0,
                dy: wheel,
                precise: false,
            }),
            WM_MOUSEHWHEEL => Some(Captured::Wheel {
                dx: wheel,
                dy: 0.0,
                precise: false,
            }),
            _ => None,
        };
        let Some(event) = event else {
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        };
        let pass = std::panic::catch_unwind(|| handle(&[event])).unwrap_or(true);
        if pass {
            CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
        } else {
            1
        }
    }
}

/// Motion from the hook's position: relative to the parking spot while grabbed, else to the last
/// position.
fn motion(pt: POINT) -> Captured {
    CONTEXT.with(|c| {
        let mut context = c.try_borrow_mut().ok();
        let Some(context) = context.as_deref_mut().and_then(Option::as_mut) else {
            return Captured::Motion {
                x: f64::from(pt.x),
                y: f64::from(pt.y),
                dx: 0.0,
                dy: 0.0,
            };
        };
        let shared = &context.shared;
        if shared.grabbed.load(Ordering::SeqCst) {
            let (px, py) = (
                shared.park_x.load(Ordering::SeqCst),
                shared.park_y.load(Ordering::SeqCst),
            );
            context.last = POINT { x: px, y: py };
            Captured::Motion {
                x: f64::from(px),
                y: f64::from(py),
                dx: f64::from(pt.x - px),
                dy: f64::from(pt.y - py),
            }
        } else {
            if shared.returned.swap(false, Ordering::SeqCst) {
                // Measured from the parking spot, the first motion would look like a jump across
                // the screen and switch straight back to the other computer
                context.last = POINT {
                    x: shared.park_x.load(Ordering::SeqCst),
                    y: shared.park_y.load(Ordering::SeqCst),
                };
            }
            let last = context.last;
            context.last = pt;
            Captured::Motion {
                x: f64::from(pt.x),
                y: f64::from(pt.y),
                dx: f64::from(pt.x - last.x),
                dy: f64::from(pt.y - last.y),
            }
        }
    })
}

pub struct WinInject {
    shared: Arc<Shared>,
}

fn hide_cursor() {
    // Empty 32×32 cursor: AND mask all ones, XOR mask all zeros
    let and = [0xFFu8; 128];
    let xor = [0u8; 128];
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        let blank: HCURSOR = CreateCursor(module, 0, 0, 32, 32, and.as_ptr().cast(), xor.as_ptr().cast());
        if blank.is_null() {
            return;
        }
        for id in CURSOR_IDS {
            // SetSystemCursor takes ownership, so each id gets its own copy
            let copy = CopyIcon(blank);
            if !copy.is_null() {
                SetSystemCursor(copy, id);
            }
        }
        windows_sys::Win32::UI::WindowsAndMessaging::DestroyCursor(blank);
    }
}

fn show_cursor() {
    unsafe { SystemParametersInfoW(SPI_SETCURSORS, 0, std::ptr::null_mut(), 0) };
}

impl Inject for WinInject {
    fn grab(&self, park: Point) -> Result<(), InputError> {
        // Park in the middle of the screen the pointer left from, so movement in every direction
        // can be measured
        let screens = self.screens();
        let rect = screens
            .iter()
            .find(|s| {
                park.x >= f64::from(s.x)
                    && park.x < f64::from(s.x) + f64::from(s.width)
                    && park.y >= f64::from(s.y)
                    && park.y < f64::from(s.y) + f64::from(s.height)
            })
            .or(screens.first());
        let (cx, cy) = rect.map_or((park.x as i32, park.y as i32), |s| {
            (s.x + s.width as i32 / 2, s.y + s.height as i32 / 2)
        });
        self.shared.park_x.store(cx, Ordering::SeqCst);
        self.shared.park_y.store(cy, Ordering::SeqCst);
        self.shared.grabbed.store(true, Ordering::SeqCst);
        let mut state = self.shared.inject.lock().expect("inject lock");
        unsafe { SetCursorPos(cx, cy) };
        if !state.cursor_hidden {
            hide_cursor();
            state.cursor_hidden = true;
        }
        Ok(())
    }

    fn ungrab(&self, at: Point) -> Result<(), InputError> {
        let mut state = self.shared.inject.lock().expect("inject lock");
        self.shared.grabbed.store(false, Ordering::SeqCst);
        if state.cursor_hidden {
            show_cursor();
            state.cursor_hidden = false;
        }
        let (x, y) = (at.x.round() as i32, at.y.round() as i32);
        unsafe { SetCursorPos(x, y) };
        self.shared.park_x.store(x, Ordering::SeqCst);
        self.shared.park_y.store(y, Ordering::SeqCst);
        self.shared.returned.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn inject(&self, message: &Message) -> Result<(), InputError> {
        let mut state = self.shared.inject.lock().expect("inject lock");
        // The service's agent injects with the highest rights and on the sign-in and UAC screens
        if !state.agent.send(message) {
            state.injector.apply(message);
        }
        Ok(())
    }

    fn release_all(&self) {
        let mut state = self.shared.inject.lock().expect("inject lock");
        state.agent.send(&Message::ReleaseAll);
        state.injector.release_all();
        if state.cursor_hidden {
            show_cursor();
            state.cursor_hidden = false;
        }
    }

    fn screens(&self) -> Vec<Screen> {
        unsafe extern "system" fn collect(monitor: HMONITOR, _dc: HDC, _rect: *mut RECT, data: LPARAM) -> i32 {
            unsafe {
                let screens = &mut *(data as *mut Vec<Screen>);
                let mut info: MONITORINFO = std::mem::zeroed();
                info.cbSize = size_of::<MONITORINFO>() as u32;
                if GetMonitorInfoW(monitor, &mut info) == 0 {
                    return 1;
                }
                let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
                GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
                let r = info.rcMonitor;
                screens.push(Screen {
                    x: r.left,
                    y: r.top,
                    width: (r.right - r.left).max(0) as u32,
                    height: (r.bottom - r.top).max(0) as u32,
                    scale: dpi_x as f32 / 96.0,
                });
                1
            }
        }
        let mut screens: Vec<Screen> = Vec::new();
        unsafe {
            EnumDisplayMonitors(
                std::ptr::null_mut(),
                std::ptr::null(),
                Some(collect),
                &mut screens as *mut Vec<Screen> as LPARAM,
            );
        }
        screens
    }
}
