//! macOS: capture with a Quartz event tap in the login session, inject with `CGEventPost`.
//!
//! Needs the Accessibility permission (active tap and posting). While another program holds Secure
//! Input, the tap gets no letters; the system helper (separate) covers that case.

use std::ffi::{CString, c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use core_foundation_sys::base::{CFRelease, CFTypeRef, kCFAllocatorDefault};
use core_foundation_sys::dictionary::{
    CFDictionaryCreate, CFDictionaryRef, kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks,
};
use core_foundation_sys::mach_port::{CFMachPortCreateRunLoopSource, CFMachPortRef};
use core_foundation_sys::number::kCFBooleanTrue;
use core_foundation_sys::runloop::{
    CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRef, CFRunLoopRun, CFRunLoopStop, kCFRunLoopCommonModes,
};
use core_foundation_sys::string::{CFStringCreateWithCString, CFStringRef, kCFStringEncodingUTF8};
use tc_core::{Captured, Point};
use tc_protocol::{Message, MouseButton, PressedKeys, Screen};

use crate::keymap::mac as keymap;
use crate::mac_helper::HelperClient;
use crate::{Backend, Capture, Handler, Health, Inject, InputError, Problem};

type Ref = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: extern "C" fn(Ref, u32, Ref, *mut c_void) -> Ref,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    fn CGEventGetLocation(event: Ref) -> CGPoint;
    fn CGEventGetIntegerValueField(event: Ref, field: u32) -> i64;
    fn CGEventGetDoubleValueField(event: Ref, field: u32) -> f64;
    fn CGEventSetIntegerValueField(event: Ref, field: u32, value: i64);
    fn CGEventSetDoubleValueField(event: Ref, field: u32, value: f64);
    fn CGEventGetFlags(event: Ref) -> u64;
    fn CGEventSetFlags(event: Ref, flags: u64);
    fn CGEventSetType(event: Ref, kind: u32);
    fn CGEventCreate(source: Ref) -> Ref;
    fn CGEventCreateMouseEvent(source: Ref, kind: u32, position: CGPoint, button: u32) -> Ref;
    fn CGEventCreateKeyboardEvent(source: Ref, key: u16, down: bool) -> Ref;
    fn CGEventCreateScrollWheelEvent2(
        source: Ref,
        units: u32,
        count: u32,
        wheel1: i32,
        wheel2: i32,
        wheel3: i32,
    ) -> Ref;
    fn CGEventPost(tap: u32, event: Ref);
    fn CGEventSourceCreate(state: i32) -> Ref;
    fn CGEventSourceSetUserData(source: Ref, data: i64);
    fn CGEventSourceSetLocalEventsSuppressionInterval(source: Ref, seconds: f64);
    fn CGWarpMouseCursorPosition(position: CGPoint) -> i32;
    fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFStringRef;
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn LMGetKbdType() -> u8;
    fn KBGetLayoutType(keyboard: i16) -> u32;
    fn IsSecureEventInputEnabled() -> bool;
}

unsafe extern "C" {
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

// Event types
const LEFT_DOWN: u32 = 1;
const LEFT_UP: u32 = 2;
const RIGHT_DOWN: u32 = 3;
const RIGHT_UP: u32 = 4;
const MOUSE_MOVED: u32 = 5;
const LEFT_DRAGGED: u32 = 6;
const RIGHT_DRAGGED: u32 = 7;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
/// `NSEventTypeSystemDefined`; media keys arrive as these.
const SYSTEM_DEFINED: u32 = 14;
const SCROLL_WHEEL: u32 = 22;
const OTHER_DOWN: u32 = 25;
const OTHER_UP: u32 = 26;
const OTHER_DRAGGED: u32 = 27;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

// Event fields
const CLICK_STATE: u32 = 1;
const BUTTON_NUMBER: u32 = 3;
const DELTA_X: u32 = 4;
const DELTA_Y: u32 = 5;
const KEYCODE: u32 = 9;
const SOURCE_USER_DATA: u32 = 42;
const SCROLL_IS_CONTINUOUS: u32 = 88;
const SCROLL_FIXED_AXIS1: u32 = 93;
const SCROLL_FIXED_AXIS2: u32 = 94;
const SCROLL_POINT_AXIS1: u32 = 96;
const SCROLL_POINT_AXIS2: u32 = 97;

// Flags
const FLAG_ALPHA_SHIFT: u64 = 0x1_0000;
const FLAG_SHIFT: u64 = 0x2_0000;
const FLAG_CONTROL: u64 = 0x4_0000;
const FLAG_ALTERNATE: u64 = 0x8_0000;
const FLAG_COMMAND: u64 = 0x10_0000;
const FLAG_NON_COALESCED: u64 = 0x100;

const SESSION_TAP: u32 = 1;
const HID_TAP: u32 = 0;
const HEAD_INSERT: u32 = 0;
const TAP_DEFAULT: u32 = 0;
const SOURCE_COMBINED_SESSION: i32 = 0;
const SCROLL_PIXEL: u32 = 0;
const SCROLL_LINE: u32 = 1;
const KEYBOARD_ISO: u32 = u32::from_be_bytes(*b"ISO ");

/// Marks events we post ourselves, so the tap lets them through untouched.
const OWN_EVENTS: i64 = 0x7462_6C78; // "tblx"
const DOUBLE_CLICK: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_DISTANCE: f64 = 5.0;
const CAPS_LOCK: u16 = 0x39;

/// HID modifier usage → (generic flag, device-dependent flag)
const MODIFIER_FLAGS: [(u16, u64, u64); 8] = [
    (0xE0, FLAG_CONTROL, 0x0001),
    (0xE1, FLAG_SHIFT, 0x0002),
    (0xE2, FLAG_ALTERNATE, 0x0020),
    (0xE3, FLAG_COMMAND, 0x0008),
    (0xE4, FLAG_CONTROL, 0x2000),
    (0xE5, FLAG_SHIFT, 0x0004),
    (0xE6, FLAG_ALTERNATE, 0x0040),
    (0xE7, FLAG_COMMAND, 0x0010),
];

fn is_iso() -> bool {
    unsafe { KBGetLayoutType(i16::from(LMGetKbdType())) == KEYBOARD_ISO }
}

fn cf_string(text: &str) -> CFStringRef {
    let c = CString::new(text).expect("no NUL");
    unsafe { CFStringCreateWithCString(kCFAllocatorDefault, c.as_ptr(), kCFStringEncodingUTF8) }
}

/// Without this a program in the background cannot hide the pointer. Private, but stable for many
/// years; if it is missing the pointer just stays visible at the edge.
fn allow_hiding_cursor_in_background() {
    unsafe {
        let default_connection = dlsym(RTLD_DEFAULT, c"_CGSDefaultConnection".as_ptr());
        let set_property = dlsym(RTLD_DEFAULT, c"CGSSetConnectionProperty".as_ptr());
        if default_connection.is_null() || set_property.is_null() {
            tracing::info!("cannot hide the pointer while in the background");
            return;
        }
        let default_connection: extern "C" fn() -> i32 = std::mem::transmute(default_connection);
        let set_property: extern "C" fn(i32, i32, CFStringRef, CFTypeRef) -> i32 = std::mem::transmute(set_property);
        let key = cf_string("SetsCursorInBackground");
        let cid = default_connection();
        set_property(cid, cid, key, kCFBooleanTrue.cast());
        CFRelease(key.cast());
    }
}

/// Asks for the Accessibility permission (shows the system prompt once).
fn request_accessibility() -> bool {
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt.cast::<c_void>()];
        let values = [kCFBooleanTrue.cast::<c_void>()];
        let options = CFDictionaryCreate(
            kCFAllocatorDefault,
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        );
        let trusted = AXIsProcessTrustedWithOptions(options);
        CFRelease(options.cast());
        trusted
    }
}

/// State shared by capture and injection.
struct Shared {
    grabbed: AtomicBool,
    /// The system helper, once capture runs.
    helper: Mutex<Option<Arc<HelperClient>>>,
    tap: AtomicPtr<c_void>,
    run_loop: AtomicPtr<c_void>,
    inject: Mutex<InjectState>,
}

struct InjectState {
    /// `CGEventSourceRef`, used only under this lock.
    source: usize,
    keys: PressedKeys,
    buttons: Vec<MouseButton>,
    position: Point,
    last_click: Option<(MouseButton, Instant, Point, i64)>,
    scroll_rest: (f64, f64),
    cursor_hidden: bool,
}

// The event source is only touched while holding the lock
unsafe impl Send for InjectState {}

pub fn backend() -> Result<Backend, InputError> {
    allow_hiding_cursor_in_background();
    let source = unsafe { CGEventSourceCreate(SOURCE_COMBINED_SESSION) };
    if source.is_null() {
        return Err(InputError::System("cannot create an event source".into()));
    }
    unsafe {
        CGEventSourceSetUserData(source, OWN_EVENTS);
        // Otherwise local input is ignored for a moment after every injected event
        CGEventSourceSetLocalEventsSuppressionInterval(source, 0.0);
    }
    let shared = Arc::new(Shared {
        grabbed: AtomicBool::new(false),
        helper: Mutex::new(None),
        tap: AtomicPtr::new(std::ptr::null_mut()),
        run_loop: AtomicPtr::new(std::ptr::null_mut()),
        inject: Mutex::new(InjectState {
            source: source as usize,
            keys: PressedKeys::new(),
            buttons: Vec::new(),
            position: current_position(),
            last_click: None,
            scroll_rest: (0.0, 0.0),
            cursor_hidden: false,
        }),
    });
    Ok(Backend {
        capture: Box::new(MacCapture { shared: shared.clone() }),
        inject: Arc::new(MacInject { shared }),
    })
}

fn current_position() -> Point {
    unsafe {
        let event = CGEventCreate(std::ptr::null_mut());
        if event.is_null() {
            return Point::default();
        }
        let p = CGEventGetLocation(event);
        CFRelease(event.cast());
        Point::new(p.x, p.y)
    }
}

pub struct MacCapture {
    shared: Arc<Shared>,
}

struct TapContext {
    /// Shared with the helper client, which delivers keys while the keyboards are taken.
    handler: Arc<Mutex<Handler>>,
    shared: Arc<Shared>,
    iso: bool,
}

fn mask(types: &[u32]) -> u64 {
    types.iter().fold(0, |m, t| m | (1u64 << t))
}

impl Capture for MacCapture {
    fn start(&mut self, handler: Handler) -> Result<(), InputError> {
        if !request_accessibility() {
            return Err(InputError::Permission(
                "Accessibility (System Settings → Privacy & Security → Accessibility)".into(),
            ));
        }
        let shared = self.shared.clone();
        let handler = Arc::new(Mutex::new(handler));
        {
            let handler = handler.clone();
            let helper = HelperClient::start(move |usage, down| {
                if let Ok(mut handler) = handler.lock() {
                    handler(Captured::Key { usage, down });
                }
            });
            *self.shared.helper.lock().expect("helper lock") = Some(helper);
        }
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("tablecross-capture".into())
            .spawn(move || {
                let context = Box::into_raw(Box::new(TapContext {
                    handler,
                    shared: shared.clone(),
                    iso: is_iso(),
                }));
                let events = mask(&[
                    LEFT_DOWN,
                    LEFT_UP,
                    RIGHT_DOWN,
                    RIGHT_UP,
                    MOUSE_MOVED,
                    LEFT_DRAGGED,
                    RIGHT_DRAGGED,
                    KEY_DOWN,
                    KEY_UP,
                    FLAGS_CHANGED,
                    SYSTEM_DEFINED,
                    SCROLL_WHEEL,
                    OTHER_DOWN,
                    OTHER_UP,
                    OTHER_DRAGGED,
                ]);
                let tap = unsafe {
                    CGEventTapCreate(
                        SESSION_TAP,
                        HEAD_INSERT,
                        TAP_DEFAULT,
                        events,
                        tap_callback,
                        context.cast(),
                    )
                };
                if tap.is_null() {
                    let _ = ready_tx.send(false);
                    drop(unsafe { Box::from_raw(context) });
                    return;
                }
                shared.tap.store(tap.cast(), Ordering::SeqCst);
                unsafe {
                    let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0);
                    let run_loop = CFRunLoopGetCurrent();
                    shared.run_loop.store(run_loop.cast(), Ordering::SeqCst);
                    CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
                    CGEventTapEnable(tap, true);
                    let _ = ready_tx.send(true);
                    CFRunLoopRun();
                    CGEventTapEnable(tap, false);
                    CFRelease(source.cast());
                    CFRelease(tap.cast());
                    shared.tap.store(std::ptr::null_mut(), Ordering::SeqCst);
                    drop(Box::from_raw(context));
                }
            })
            .map_err(|e| InputError::System(e.to_string()))?;
        match ready_rx.recv() {
            Ok(true) => Ok(()),
            _ => Err(InputError::Permission(
                "Accessibility (System Settings → Privacy & Security → Accessibility); \
                 after granting it, start TableCross again"
                    .into(),
            )),
        }
    }

    fn stop(&mut self) {
        let run_loop = self.shared.run_loop.swap(std::ptr::null_mut(), Ordering::SeqCst);
        if !run_loop.is_null() {
            unsafe { CFRunLoopStop(run_loop as CFRunLoopRef) };
        }
    }

    fn health(&self) -> Health {
        let tap = self.shared.tap.load(Ordering::SeqCst);
        let capturing = !tap.is_null() && unsafe { CGEventTapIsEnabled(tap.cast()) };
        let mut problems = Vec::new();
        if !unsafe { AXIsProcessTrusted() } {
            problems.push(Problem::AccessibilityMissing);
        }
        if !capturing {
            problems.push(Problem::CaptureStopped);
        }
        let helper = self
            .shared
            .helper
            .lock()
            .expect("helper lock")
            .as_ref()
            .map(|h| h.status());
        let covered = match helper {
            Some(status) if status.connected => {
                if !status.access {
                    problems.push(Problem::HelperNoAccess);
                }
                if status.failed > 0 {
                    problems.push(Problem::HelperKeyboardsBusy(status.failed));
                }
                status.access
            }
            Some(status) if status.version_mismatch.is_some() => {
                problems.push(Problem::HelperOutdated);
                false
            }
            _ => {
                problems.push(Problem::HelperMissing);
                false
            }
        };
        if !covered && unsafe { IsSecureEventInputEnabled() } {
            problems.push(Problem::SecureInput);
        }
        Health { capturing, problems }
    }
}

extern "C" fn tap_callback(_proxy: Ref, kind: u32, event: Ref, user_info: *mut c_void) -> Ref {
    let context = unsafe { &mut *(user_info as *mut TapContext) };
    if kind == TAP_DISABLED_BY_TIMEOUT || kind == TAP_DISABLED_BY_USER_INPUT {
        // The system switches slow taps off; switch it on again at once
        tracing::warn!("event tap was disabled by the system, enabling it again");
        let tap = context.shared.tap.load(Ordering::SeqCst);
        if !tap.is_null() {
            unsafe { CGEventTapEnable(tap.cast(), true) };
        }
        return event;
    }
    if unsafe { CGEventGetIntegerValueField(event, SOURCE_USER_DATA) } == OWN_EVENTS {
        return event;
    }
    let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut pass = true;
        let Ok(mut handler) = context.handler.lock() else {
            return true;
        };
        for captured in translate(kind, event, context.iso) {
            pass &= handler(captured);
        }
        pass
    }))
    .unwrap_or(true);
    if pass { event } else { std::ptr::null_mut() }
}

fn button_of(kind: u32, event: Ref) -> MouseButton {
    match kind {
        LEFT_DOWN | LEFT_UP => MouseButton::Left,
        RIGHT_DOWN | RIGHT_UP => MouseButton::Right,
        _ => match unsafe { CGEventGetIntegerValueField(event, BUTTON_NUMBER) } {
            3 => MouseButton::Back,
            4 => MouseButton::Forward,
            _ => MouseButton::Middle,
        },
    }
}

fn translate(kind: u32, event: Ref, iso: bool) -> Vec<Captured> {
    unsafe {
        match kind {
            MOUSE_MOVED | LEFT_DRAGGED | RIGHT_DRAGGED | OTHER_DRAGGED => {
                let p = CGEventGetLocation(event);
                vec![Captured::Motion {
                    x: p.x,
                    y: p.y,
                    dx: CGEventGetDoubleValueField(event, DELTA_X),
                    dy: CGEventGetDoubleValueField(event, DELTA_Y),
                }]
            }
            LEFT_DOWN | RIGHT_DOWN | OTHER_DOWN | LEFT_UP | RIGHT_UP | OTHER_UP => vec![Captured::Button {
                button: button_of(kind, event),
                down: matches!(kind, LEFT_DOWN | RIGHT_DOWN | OTHER_DOWN),
            }],
            SYSTEM_DEFINED => media::read(event)
                .map(|(key, down)| vec![Captured::Media { key, down }])
                .unwrap_or_default(),
            KEY_DOWN | KEY_UP => {
                let code = CGEventGetIntegerValueField(event, KEYCODE) as u16;
                match keymap::to_hid(code, iso) {
                    Some(usage) => vec![Captured::Key {
                        usage,
                        down: kind == KEY_DOWN,
                    }],
                    None => Vec::new(),
                }
            }
            FLAGS_CHANGED => {
                let code = CGEventGetIntegerValueField(event, KEYCODE) as u16;
                let flags = CGEventGetFlags(event);
                let Some(usage) = keymap::to_hid(code, iso) else {
                    return Vec::new();
                };
                if usage == CAPS_LOCK {
                    // Caps Lock reports only the new lock state: forward one press
                    return vec![
                        Captured::Key { usage, down: true },
                        Captured::Key { usage, down: false },
                    ];
                }
                match MODIFIER_FLAGS.iter().find(|(u, ..)| *u == usage) {
                    Some((_, _, device)) => vec![Captured::Key {
                        usage,
                        down: flags & device != 0,
                    }],
                    None => Vec::new(),
                }
            }
            SCROLL_WHEEL => {
                let precise = CGEventGetIntegerValueField(event, SCROLL_IS_CONTINUOUS) != 0;
                let (dy, dx) = if precise {
                    (
                        CGEventGetDoubleValueField(event, SCROLL_POINT_AXIS1),
                        CGEventGetDoubleValueField(event, SCROLL_POINT_AXIS2),
                    )
                } else {
                    (
                        CGEventGetDoubleValueField(event, SCROLL_FIXED_AXIS1),
                        CGEventGetDoubleValueField(event, SCROLL_FIXED_AXIS2),
                    )
                };
                // Axis 2 is positive when the content moves right, i.e. more of the left shows
                vec![Captured::Wheel { dx: -dx, dy, precise }]
            }
            _ => Vec::new(),
        }
    }
}

pub struct MacInject {
    shared: Arc<Shared>,
}

impl MacInject {
    /// While grabbed, the helper takes the keyboards, so keys arrive even under Secure Input.
    fn set_seize(&self, seize: bool) {
        if let Some(helper) = self.shared.helper.lock().expect("helper lock").as_ref() {
            helper.set_seize(seize);
        }
    }
}

impl InjectState {
    fn source(&self) -> Ref {
        self.source as Ref
    }

    fn flags(&self) -> u64 {
        MODIFIER_FLAGS
            .iter()
            .filter(|(usage, ..)| self.keys.is_down(*usage))
            .fold(FLAG_NON_COALESCED, |f, (_, generic, device)| f | generic | device)
    }

    fn post(&self, event: Ref) {
        if event.is_null() {
            return;
        }
        unsafe {
            CGEventSetIntegerValueField(event, SOURCE_USER_DATA, OWN_EVENTS);
            CGEventSetFlags(event, self.flags() | (CGEventGetFlags(event) & FLAG_ALPHA_SHIFT));
            CGEventPost(HID_TAP, event);
            CFRelease(event.cast());
        }
    }

    fn move_to(&mut self, p: Point, dx: f64, dy: f64) {
        self.position = p;
        let kind = if self.buttons.contains(&MouseButton::Left) {
            LEFT_DRAGGED
        } else if self.buttons.contains(&MouseButton::Right) {
            RIGHT_DRAGGED
        } else if !self.buttons.is_empty() {
            OTHER_DRAGGED
        } else {
            MOUSE_MOVED
        };
        unsafe {
            let event = CGEventCreateMouseEvent(self.source(), kind, CGPoint { x: p.x, y: p.y }, 0);
            if !event.is_null() {
                CGEventSetDoubleValueField(event, DELTA_X, dx);
                CGEventSetDoubleValueField(event, DELTA_Y, dy);
            }
            self.post(event);
        }
    }

    fn button(&mut self, button: MouseButton, down: bool) {
        let (kind, number) = match (button, down) {
            (MouseButton::Left, true) => (LEFT_DOWN, 0),
            (MouseButton::Left, false) => (LEFT_UP, 0),
            (MouseButton::Right, true) => (RIGHT_DOWN, 1),
            (MouseButton::Right, false) => (RIGHT_UP, 1),
            (MouseButton::Middle, d) => (if d { OTHER_DOWN } else { OTHER_UP }, 2),
            (MouseButton::Back, d) => (if d { OTHER_DOWN } else { OTHER_UP }, 3),
            (MouseButton::Forward, d) => (if d { OTHER_DOWN } else { OTHER_UP }, 4),
        };
        if down {
            if self.buttons.contains(&button) {
                return;
            }
            self.buttons.push(button);
        } else {
            if !self.buttons.contains(&button) {
                return;
            }
            self.buttons.retain(|b| *b != button);
        }
        // Double and triple clicks: the system counts them only if we say so
        let now = Instant::now();
        let p = self.position;
        let clicks = match self.last_click {
            Some((b, at, q, n)) if down && b == button => {
                let close = (p.x - q.x).hypot(p.y - q.y) <= DOUBLE_CLICK_DISTANCE;
                if now - at < DOUBLE_CLICK && close {
                    (n + 1).min(3)
                } else {
                    1
                }
            }
            Some((b, .., n)) if !down && b == button => n,
            _ => 1,
        };
        if down {
            self.last_click = Some((button, now, p, clicks));
        }
        unsafe {
            let event = CGEventCreateMouseEvent(self.source(), kind, CGPoint { x: p.x, y: p.y }, number);
            if !event.is_null() {
                CGEventSetIntegerValueField(event, CLICK_STATE, clicks);
                CGEventSetIntegerValueField(event, BUTTON_NUMBER, i64::from(number));
            }
            self.post(event);
        }
    }

    fn key(&mut self, usage: u16, down: bool) {
        let Some(code) = keymap::from_hid(usage, is_iso()) else {
            return;
        };
        let changed = if down {
            self.keys.press(usage)
        } else {
            self.keys.release(usage)
        };
        if !changed {
            return;
        }
        unsafe {
            let event = CGEventCreateKeyboardEvent(self.source(), code, down);
            if event.is_null() {
                return;
            }
            if MODIFIER_FLAGS.iter().any(|(u, ..)| *u == usage) {
                // Modifiers are "flags changed" events with the new flags
                CGEventSetType(event, FLAGS_CHANGED);
            }
            self.post(event);
        }
    }

    fn wheel(&mut self, dx: f64, dy: f64, precise: bool) {
        // Fractions add up until a whole step is reached
        let (rx, ry) = self.scroll_rest;
        let (tx, ty) = (rx + dx, ry + dy);
        let (wx, wy) = (tx.trunc(), ty.trunc());
        self.scroll_rest = (tx - wx, ty - wy);
        if wx == 0.0 && wy == 0.0 {
            return;
        }
        let units = if precise { SCROLL_PIXEL } else { SCROLL_LINE };
        unsafe {
            let event = CGEventCreateScrollWheelEvent2(self.source(), units, 2, wy as i32, -(wx as i32), 0);
            self.post(event);
        }
    }

    fn release_all(&mut self) {
        for usage in self.keys.clone().release_all() {
            self.key(usage, false);
        }
        for button in self.buttons.clone() {
            self.button(button, false);
        }
    }
}

impl Inject for MacInject {
    fn grab(&self, park: Point) -> Result<(), InputError> {
        self.shared.grabbed.store(true, Ordering::SeqCst);
        self.set_seize(true);
        let mut state = self.shared.inject.lock().expect("inject lock");
        unsafe {
            CGWarpMouseCursorPosition(CGPoint { x: park.x, y: park.y });
            CGAssociateMouseAndMouseCursorPosition(false);
            if !state.cursor_hidden {
                CGDisplayHideCursor(CGMainDisplayID());
                state.cursor_hidden = true;
            }
        }
        Ok(())
    }

    fn ungrab(&self, at: Point) -> Result<(), InputError> {
        self.shared.grabbed.store(false, Ordering::SeqCst);
        self.set_seize(false);
        let mut state = self.shared.inject.lock().expect("inject lock");
        state.position = at;
        unsafe {
            CGWarpMouseCursorPosition(CGPoint { x: at.x, y: at.y });
            CGAssociateMouseAndMouseCursorPosition(true);
            if state.cursor_hidden {
                CGDisplayShowCursor(CGMainDisplayID());
                state.cursor_hidden = false;
            }
        }
        Ok(())
    }

    fn inject(&self, message: &Message) -> Result<(), InputError> {
        let mut state = self.shared.inject.lock().expect("inject lock");
        match *message {
            Message::Enter { x, y } => state.move_to(Point::new(f64::from(x), f64::from(y)), 0.0, 0.0),
            Message::MouseMove { x, y, dx, dy, .. } => {
                state.move_to(Point::new(f64::from(x), f64::from(y)), f64::from(dx), f64::from(dy))
            }
            Message::MouseButton { button, down } => state.button(button, down),
            Message::Key { usage, down } => state.key(usage, down),
            Message::Wheel { dx, dy, precise } => state.wheel(f64::from(dx), f64::from(dy), precise),
            Message::Media { key } => {
                for down in [true, false] {
                    if let Some(event) = media::create(key, down) {
                        state.post(event);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn release_all(&self) {
        self.set_seize(false);
        self.shared.inject.lock().expect("inject lock").release_all();
    }

    fn screens(&self) -> Vec<Screen> {
        let mut ids = [0u32; 16];
        let mut count = 0u32;
        if unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) } != 0 {
            return Vec::new();
        }
        ids[..count as usize]
            .iter()
            .map(|id| {
                let b = unsafe { CGDisplayBounds(*id) };
                Screen {
                    x: b.origin.x.round() as i32,
                    y: b.origin.y.round() as i32,
                    width: b.size.width.round() as u32,
                    height: b.size.height.round() as u32,
                    scale: 1.0,
                }
            })
            .collect()
    }
}

/// Media keys: system events of subtype 8 ("aux control buttons"); `data1` holds the key type in
/// the upper 16 bits and 0xA (down) or 0xB (up) in bits 8 to 15.
mod media {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_core_graphics::CGEvent;
    use objc2_foundation::NSPoint;
    use tc_protocol::MediaKey;

    use super::Ref;

    const AUX_CONTROL_BUTTONS: i16 = 8;
    const KEYS: [(i64, MediaKey); 8] = [
        (0, MediaKey::VolumeUp),
        (1, MediaKey::VolumeDown),
        (2, MediaKey::BrightnessUp),
        (3, MediaKey::BrightnessDown),
        (7, MediaKey::Mute),
        (16, MediaKey::PlayPause),
        (17, MediaKey::Next),
        (18, MediaKey::Previous),
    ];

    pub fn read(event: Ref) -> Option<(MediaKey, bool)> {
        objc2::rc::autoreleasepool(|_| {
            // SAFETY: the tap hands over a valid CGEvent for the duration of the callback
            let ns = NSEvent::eventWithCGEvent(unsafe { &*(event as *const CGEvent) })?;
            if ns.r#type() != NSEventType::SystemDefined || ns.subtype().0 != AUX_CONTROL_BUTTONS {
                return None;
            }
            let data = ns.data1() as i64;
            let code = (data >> 16) & 0xFFFF;
            let down = (data >> 8) & 0xFF == 0xA;
            KEYS.iter().find(|(c, _)| *c == code).map(|(_, key)| (*key, down))
        })
    }

    /// A new event (owned by the caller, like the other `CGEventCreate…` results).
    pub fn create(key: MediaKey, down: bool) -> Option<Ref> {
        let code = KEYS.iter().find(|(_, k)| *k == key)?.0;
        let state: i64 = if down { 0xA } else { 0xB };
        objc2::rc::autoreleasepool(|_| {
            let ns =
                NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
                    NSEventType::SystemDefined,
                    NSPoint::new(0.0, 0.0),
                    NSEventModifierFlags((state as usize) << 8),
                    0.0,
                    0,
                    None,
                    AUX_CONTROL_BUTTONS,
                    ((code << 16) | (state << 8)) as isize,
                    -1,
                )?;
            let cg = ns.CGEvent()?;
            // Keep it beyond the pool, as the caller releases it
            Some(objc2::rc::Retained::into_raw(cg).cast())
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn created_events_read_back() {
            for (_, key) in KEYS {
                for down in [true, false] {
                    let event = create(key, down).expect("event");
                    assert_eq!(read(event), Some((key, down)));
                    unsafe { core_foundation_sys::base::CFRelease(event.cast()) };
                }
            }
        }
    }
}
