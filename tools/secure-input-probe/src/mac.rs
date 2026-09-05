use std::ffi::{CString, c_void};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::time::{Duration, Instant};

use core_foundation_sys::array::{CFArrayCreate, kCFTypeArrayCallBacks};
use core_foundation_sys::base::{CFAllocatorRef, CFIndex, CFRelease, kCFAllocatorDefault};
use core_foundation_sys::dictionary::{
    CFDictionaryCreate, CFDictionaryGetValue, CFDictionaryRef, kCFTypeDictionaryKeyCallBacks,
    kCFTypeDictionaryValueCallBacks,
};
use core_foundation_sys::mach_port::{CFMachPortCreateRunLoopSource, CFMachPortRef};
use core_foundation_sys::number::{CFNumberCreate, CFNumberGetValue, CFNumberRef, kCFNumberSInt32Type};
use core_foundation_sys::runloop::{
    CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRef, CFRunLoopRun, CFRunLoopRunInMode, kCFRunLoopDefaultMode,
};
use core_foundation_sys::string::{CFStringCreateWithCString, CFStringRef, kCFStringEncodingUTF8};

type Ref = *mut c_void;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDManagerCreate(alloc: CFAllocatorRef, options: u32) -> Ref;
    fn IOHIDManagerSetDeviceMatchingMultiple(manager: Ref, multiple: *const c_void);
    fn IOHIDManagerRegisterInputValueCallback(
        manager: Ref,
        callback: extern "C" fn(*mut c_void, i32, *mut c_void, Ref),
        context: *mut c_void,
    );
    fn IOHIDManagerScheduleWithRunLoop(manager: Ref, run_loop: CFRunLoopRef, mode: CFStringRef);
    fn IOHIDManagerOpen(manager: Ref, options: u32) -> i32;
    fn IOHIDManagerClose(manager: Ref, options: u32) -> i32;
    fn IOHIDManagerUnscheduleFromRunLoop(manager: Ref, run_loop: CFRunLoopRef, mode: CFStringRef);
    fn IOHIDValueGetElement(value: Ref) -> Ref;
    fn IOHIDValueGetIntegerValue(value: Ref) -> CFIndex;
    fn IOHIDElementGetUsagePage(element: Ref) -> u32;
    fn IOHIDElementGetUsage(element: Ref) -> u32;
    fn IOHIDCheckAccess(request: u32) -> u32;
    fn IOHIDRequestAccess(request: u32) -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: extern "C" fn(*mut c_void, u32, Ref, *mut c_void) -> Ref,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(event: Ref, field: u32) -> i64;
    fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

unsafe extern "C" {
    fn getuid() -> u32;
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn EnableSecureEventInput() -> i32;
    fn DisableSecureEventInput() -> i32;
}

const HID_LISTEN_EVENT: u32 = 1;
const HID_ACCESS_GRANTED: u32 = 0;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const SYSTEM_DEFINED: u32 = 14;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
const FIELD_KEYCODE: u32 = 9;
const PAGE_KEYBOARD: u32 = 0x07;
const PAGE_CONSUMER: u32 = 0x0C;
const SEIZE_DEVICE: u32 = 0x01;

static START: OnceLock<Instant> = OnceLock::new();
static TAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn log(source: &str, text: &str) {
    let t = START.get().map_or(0.0, |s| s.elapsed().as_secs_f64());
    println!("{t:7.2} s  {source:<6} {text}");
}

fn cf_string(text: &str) -> CFStringRef {
    let c = CString::new(text).unwrap();
    unsafe { CFStringCreateWithCString(kCFAllocatorDefault, c.as_ptr(), kCFStringEncodingUTF8) }
}

fn cf_number(value: i32) -> CFNumberRef {
    unsafe { CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, (&value as *const i32).cast()) }
}

pub fn run() {
    START.get_or_init(Instant::now);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number_after = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .and_then(|v| v.parse::<u64>().ok())
    };
    if let Some(secs) = number_after("--hold") {
        hold_secure_input(secs);
        return;
    }
    if let Some(i) = args.iter().position(|a| a == "--service") {
        match args.get(i + 1) {
            Some(go) => run_service(go),
            None => eprintln!("--service needs the path of the trigger file"),
        }
        return;
    }
    let secs = number_after("--seconds").unwrap_or(60);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(secs));
        log("END", &format!("after {secs} s"));
        std::process::exit(0);
    });

    report_permissions();
    std::thread::spawn(watch_secure_input);
    unsafe {
        let run_loop = CFRunLoopGetCurrent();
        start_event_tap(run_loop);
        start_hid(run_loop);
        log("START", "type a few letters and press a media key (volume) now");
        CFRunLoopRun();
    }
}

fn report_permissions() {
    unsafe {
        let hid = IOHIDCheckAccess(HID_LISTEN_EVENT);
        if hid != HID_ACCESS_GRANTED {
            IOHIDRequestAccess(HID_LISTEN_EVENT);
        }
        if !CGPreflightListenEventAccess() {
            CGRequestListenEventAccess();
        }
        let text = match hid {
            0 => "granted",
            1 => "denied",
            _ => "not asked yet",
        };
        log(
            "ACCESS",
            &format!(
                "input monitoring (IOHID): {text}, event tap: {}",
                if CGPreflightListenEventAccess() {
                    "granted"
                } else {
                    "not granted"
                }
            ),
        );
    }
}

/// Reports which process holds Secure Input (session key, as shown by `ioreg`).
fn watch_secure_input() {
    let key = cf_string("kCGSSessionSecureInputPID");
    let mut last: Option<i32> = Some(-1);
    loop {
        let pid = unsafe {
            let session = CGSessionCopyCurrentDictionary();
            let mut pid: Option<i32> = None;
            if !session.is_null() {
                let value = CFDictionaryGetValue(session, key.cast());
                if !value.is_null() {
                    let mut v: i32 = 0;
                    if CFNumberGetValue(value as CFNumberRef, kCFNumberSInt32Type, (&mut v as *mut i32).cast())
                        && v != 0
                    {
                        pid = Some(v);
                    }
                }
                CFRelease(session.cast());
            }
            pid
        };
        if pid != last {
            match pid {
                Some(p) => log(
                    "STATUS",
                    &format!("Secure Input ON, held by PID {p} ({})", process_name(p)),
                ),
                None => log("STATUS", "Secure Input off"),
            }
            last = pid;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn process_name(pid: i32) -> String {
    std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

unsafe fn start_event_tap(run_loop: CFRunLoopRef) {
    let mask = (1u64 << KEY_DOWN) | (1 << KEY_UP) | (1 << FLAGS_CHANGED) | (1 << SYSTEM_DEFINED);
    // kCGHIDEventTap = 0, kCGHeadInsertEventTap = 0, kCGEventTapOptionListenOnly = 1
    let tap = unsafe { CGEventTapCreate(0, 0, 1, mask, tap_callback, std::ptr::null_mut()) };
    if tap.is_null() {
        log("TAP", "could not be created (missing permission?)");
        return;
    }
    TAP.store(tap.cast(), Ordering::Relaxed);
    unsafe {
        let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0);
        CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode);
        CGEventTapEnable(tap, true);
    }
    log("TAP", "listening");
}

extern "C" fn tap_callback(_proxy: *mut c_void, kind: u32, event: Ref, _info: *mut c_void) -> Ref {
    match kind {
        KEY_DOWN | KEY_UP => {
            let code = unsafe { CGEventGetIntegerValueField(event, FIELD_KEYCODE) };
            log(
                "TAP",
                &format!(
                    "key {} (Mac code {code})",
                    if kind == KEY_DOWN { "down" } else { "up  " }
                ),
            );
        }
        FLAGS_CHANGED => {
            let code = unsafe { CGEventGetIntegerValueField(event, FIELD_KEYCODE) };
            log("TAP", &format!("modifier (Mac code {code})"));
        }
        SYSTEM_DEFINED => log("TAP", "system key (media, volume …)"),
        TAP_DISABLED_BY_TIMEOUT | TAP_DISABLED_BY_USER_INPUT => {
            log("TAP", "disabled by the system, re-enabling");
            let tap = TAP.load(Ordering::Relaxed);
            if !tap.is_null() {
                unsafe { CGEventTapEnable(tap.cast(), true) };
            }
        }
        other => log("TAP", &format!("event {other}")),
    }
    event
}

unsafe fn start_hid(run_loop: CFRunLoopRef) {
    let (_manager, result) = unsafe { open_hid(run_loop, 0) };
    if result == 0 {
        log("HID", "listening");
    } else {
        log("HID", &format!("could not be opened (error 0x{result:08X})"));
    }
}

/// Opens keyboards and media keys; with `SEIZE_DEVICE` exclusively (the system then receives nothing).
unsafe fn open_hid(run_loop: CFRunLoopRef, options: u32) -> (Ref, i32) {
    unsafe {
        let manager = IOHIDManagerCreate(kCFAllocatorDefault, 0);
        // Keyboards (Generic Desktop 1 / 6) and media keys (Consumer 0x0C / 1)
        let matching = [device_match(1, 6), device_match(PAGE_CONSUMER as i32, 1)];
        let array = CFArrayCreate(
            kCFAllocatorDefault,
            matching.as_ptr().cast(),
            matching.len() as CFIndex,
            &kCFTypeArrayCallBacks,
        );
        IOHIDManagerSetDeviceMatchingMultiple(manager, array.cast());
        IOHIDManagerRegisterInputValueCallback(manager, hid_callback, std::ptr::null_mut());
        IOHIDManagerScheduleWithRunLoop(manager, run_loop, kCFRunLoopDefaultMode);
        let result = IOHIDManagerOpen(manager, options);
        (manager, result)
    }
}

unsafe fn close_hid(manager: Ref, run_loop: CFRunLoopRef) {
    unsafe {
        IOHIDManagerUnscheduleFromRunLoop(manager, run_loop, kCFRunLoopDefaultMode);
        IOHIDManagerClose(manager, 0);
        CFRelease(manager.cast());
    }
}

/// Test service (LaunchDaemon running as root): waits up to 30 min for the trigger file, then runs
/// 60 s: 0–20 s listen, 20–40 s keyboard seized, 40–60 s listen.
fn run_service(go: &str) {
    START.get_or_init(Instant::now);
    let deadline = Instant::now() + Duration::from_secs(30 * 60);
    unsafe {
        let access = IOHIDCheckAccess(HID_LISTEN_EVENT);
        if access != HID_ACCESS_GRANTED {
            IOHIDRequestAccess(HID_LISTEN_EVENT);
        }
        log(
            "SERVICE",
            &format!(
                "uid {}, input monitoring {}",
                getuid(),
                if access == HID_ACCESS_GRANTED {
                    "granted"
                } else {
                    "missing"
                }
            ),
        );
        let run_loop = CFRunLoopGetCurrent();
        while Instant::now() < deadline {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.2, 0);
            if std::fs::remove_file(go).is_ok() {
                service_cycle(run_loop);
            }
        }
    }
    log("SERVICE", "exiting after 30 min");
}

unsafe fn service_cycle(run_loop: CFRunLoopRef) {
    let begin = Instant::now();
    let run_until = |secs: f64| {
        while begin.elapsed().as_secs_f64() < secs {
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 0) };
        }
    };
    unsafe {
        let (m, r) = open_hid(run_loop, 0);
        log("CYCLE", &format!("part 1: listening (0x{r:08X})"));
        run_until(20.0);
        close_hid(m, run_loop);
        let (m, r) = open_hid(run_loop, SEIZE_DEVICE);
        log("CYCLE", &format!("part 2: keyboard seized (0x{r:08X})"));
        run_until(40.0);
        close_hid(m, run_loop);
        let (m, r) = open_hid(run_loop, 0);
        log("CYCLE", &format!("part 3: listening (0x{r:08X})"));
        run_until(60.0);
        close_hid(m, run_loop);
    }
    log("CYCLE", "done");
}

fn device_match(page: i32, usage: i32) -> CFDictionaryRef {
    let keys = [cf_string("DeviceUsagePage"), cf_string("DeviceUsage")];
    let values = [cf_number(page), cf_number(usage)];
    unsafe {
        CFDictionaryCreate(
            kCFAllocatorDefault,
            keys.as_ptr().cast(),
            values.as_ptr().cast(),
            2,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
    }
}

extern "C" fn hid_callback(_ctx: *mut c_void, _result: i32, _sender: *mut c_void, value: Ref) {
    unsafe {
        let element = IOHIDValueGetElement(value);
        let page = IOHIDElementGetUsagePage(element);
        let usage = IOHIDElementGetUsage(element);
        let pressed = IOHIDValueGetIntegerValue(value);
        let state = if pressed != 0 { "down" } else { "up  " };
        match page {
            // 4..=0xE7 are real keys; values below and above are rollover/error codes
            PAGE_KEYBOARD if (4..=0xE7).contains(&usage) => {
                log("HID", &format!("key {state} (HID 0x{usage:02X})"));
            }
            PAGE_CONSUMER if usage != 0 => log("HID", &format!("media key {state} (0x{usage:03X})")),
            _ => {}
        }
    }
}

fn hold_secure_input(secs: u64) {
    START.get_or_init(Instant::now);
    unsafe { EnableSecureEventInput() };
    log(
        "HOLD",
        &format!("Secure Input enabled for {secs} s (PID {})", std::process::id()),
    );
    std::thread::sleep(Duration::from_secs(secs));
    unsafe { DisableSecureEventInput() };
    log("HOLD", "Secure Input released");
}
