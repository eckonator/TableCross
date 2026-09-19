use std::ffi::{CString, c_void};
use std::io::{BufReader, BufWriter, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime};

use core_foundation_sys::base::{CFAllocatorRef, CFIndex, CFRelease, kCFAllocatorDefault};
use core_foundation_sys::dictionary::{
    CFDictionaryCreate, CFDictionaryRef, kCFTypeDictionaryKeyCallBacks, kCFTypeDictionaryValueCallBacks,
};
use core_foundation_sys::number::{CFNumberCreate, CFNumberRef, kCFNumberSInt32Type};
use core_foundation_sys::runloop::{CFRunLoopGetCurrent, CFRunLoopRef, CFRunLoopRunInMode, kCFRunLoopDefaultMode};
use core_foundation_sys::set::{CFSetGetCount, CFSetGetValues, CFSetRef};
use core_foundation_sys::string::{CFStringCreateWithCString, CFStringRef, kCFStringEncodingUTF8};
use tc_input::helper_protocol::{
    CONSUMER_KEYS, DeadManSwitch, FromHelper, HELPER_VERSION, ToHelper, read_frame, socket_path, write_frame,
};

type Ref = *mut c_void;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDManagerCreate(alloc: CFAllocatorRef, options: u32) -> Ref;
    fn IOHIDManagerSetDeviceMatching(manager: Ref, matching: CFDictionaryRef);
    fn IOHIDManagerScheduleWithRunLoop(manager: Ref, run_loop: CFRunLoopRef, mode: CFStringRef);
    fn IOHIDManagerCopyDevices(manager: Ref) -> CFSetRef;
    fn IOHIDManagerRegisterDeviceMatchingCallback(
        manager: Ref,
        callback: extern "C" fn(*mut c_void, i32, *mut c_void, Ref),
        context: *mut c_void,
    );
    fn IOHIDDeviceOpen(device: Ref, options: u32) -> i32;
    fn IOHIDDeviceClose(device: Ref, options: u32) -> i32;
    fn IOHIDDeviceScheduleWithRunLoop(device: Ref, run_loop: CFRunLoopRef, mode: CFStringRef);
    fn IOHIDDeviceUnscheduleFromRunLoop(device: Ref, run_loop: CFRunLoopRef, mode: CFStringRef);
    fn IOHIDDeviceRegisterInputValueCallback(
        device: Ref,
        callback: Option<extern "C" fn(*mut c_void, i32, *mut c_void, Ref)>,
        context: *mut c_void,
    );
    fn IOHIDValueGetElement(value: Ref) -> Ref;
    fn IOHIDValueGetIntegerValue(value: Ref) -> CFIndex;
    fn IOHIDElementGetUsagePage(element: Ref) -> u32;
    fn IOHIDElementGetUsage(element: Ref) -> u32;
    fn IOHIDCheckAccess(request: u32) -> u32;
    fn IOHIDRequestAccess(request: u32) -> bool;
    fn CFRetain(cf: *const c_void) -> *const c_void;
}

unsafe extern "C" {
    fn getpeereid(fd: i32, uid: *mut u32, gid: *mut u32) -> i32;
    fn getuid() -> u32;
}

/// `TC_HELPER_DRY_RUN=1`: never touch keyboards or permissions (tests).
fn dry_run() -> bool {
    std::env::var_os("TC_HELPER_DRY_RUN").is_some()
}

const LISTEN_EVENT: u32 = 1;
const ACCESS_GRANTED: u32 = 0;
const SEIZE_DEVICE: u32 = 1;
const PAGE_KEYBOARD: u32 = 0x07;
const PAGE_CONSUMER: u32 = 0x0C;
/// Without the permission the helper restarts now and then: the system grants it only to a fresh
/// process (measured), and launchd starts the helper again by itself.
const RESTART_WITHOUT_ACCESS: Duration = Duration::from_secs(30);

fn log(text: &str) {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    eprintln!("{secs} {text}");
}

/// The connected app; the key callback writes to it.
static CLIENT: Mutex<Option<BufWriter<UnixStream>>> = Mutex::new(None);

fn send(message: &FromHelper) {
    let mut client = CLIENT.lock().expect("client lock");
    if let Some(stream) = client.as_mut() {
        let ok = write_frame(stream, message).and_then(|()| stream.flush()).is_ok();
        if !ok {
            *client = None;
        }
    }
}

fn cf_string(text: &str) -> CFStringRef {
    let c = CString::new(text).expect("no NUL");
    unsafe { CFStringCreateWithCString(kCFAllocatorDefault, c.as_ptr(), kCFStringEncodingUTF8) }
}

fn cf_number(value: i32) -> CFNumberRef {
    unsafe { CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, (&value as *const i32).cast()) }
}

/// Keyboards: Generic Desktop page, usage Keyboard.
fn keyboard_matching() -> CFDictionaryRef {
    let keys = [cf_string("DeviceUsagePage"), cf_string("DeviceUsage")];
    let values = [cf_number(1), cf_number(6)];
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

extern "C" fn key_callback(_context: *mut c_void, _result: i32, _sender: *mut c_void, value: Ref) {
    unsafe {
        let element = IOHIDValueGetElement(value);
        let page = IOHIDElementGetUsagePage(element);
        let usage = IOHIDElementGetUsage(element);
        let down = IOHIDValueGetIntegerValue(value) != 0;
        if page == PAGE_CONSUMER {
            if let Some((_, key)) = CONSUMER_KEYS.iter().find(|(u, _)| *u == usage) {
                send(&FromHelper::Media { key: *key, down });
            }
            return;
        }
        if page != PAGE_KEYBOARD || !(0x04..=0xE7).contains(&usage) {
            return;
        }
        send(&FromHelper::Key {
            usage: usage as u16,
            down,
        });
    }
}

/// Keyboards taken from the system, each on its own: one that someone else holds exclusively must
/// not stop the others (seizing all at once failed with 0xE00002C5).
struct Keyboards {
    manager: Ref,
    run_loop: CFRunLoopRef,
    seized: Vec<Ref>,
    failed: u16,
    active: bool,
}

impl Keyboards {
    fn new(run_loop: CFRunLoopRef) -> Self {
        unsafe {
            let manager = IOHIDManagerCreate(kCFAllocatorDefault, 0);
            let matching = keyboard_matching();
            IOHIDManagerSetDeviceMatching(manager, matching);
            CFRelease(matching.cast());
            IOHIDManagerRegisterDeviceMatchingCallback(manager, device_added, std::ptr::null_mut());
            IOHIDManagerScheduleWithRunLoop(manager, run_loop, kCFRunLoopDefaultMode);
            Self {
                manager,
                run_loop,
                seized: Vec::new(),
                failed: 0,
                active: false,
            }
        }
    }

    fn devices(&self) -> Vec<Ref> {
        unsafe {
            let set = IOHIDManagerCopyDevices(self.manager);
            if set.is_null() {
                return Vec::new();
            }
            let count = CFSetGetCount(set) as usize;
            let mut values: Vec<*const c_void> = vec![std::ptr::null(); count];
            CFSetGetValues(set, values.as_mut_ptr());
            let devices = values.into_iter().map(|d| CFRetain(d) as Ref).collect();
            CFRelease(set.cast());
            devices
        }
    }

    fn seize_device(&mut self, device: Ref) {
        if self.seized.contains(&device) {
            unsafe { CFRelease(device.cast()) };
            return;
        }
        unsafe {
            let result = IOHIDDeviceOpen(device, SEIZE_DEVICE);
            if result != 0 {
                log(&format!("cannot take a keyboard (0x{result:08X})"));
                self.failed += 1;
                CFRelease(device.cast());
                return;
            }
            IOHIDDeviceRegisterInputValueCallback(device, Some(key_callback), std::ptr::null_mut());
            IOHIDDeviceScheduleWithRunLoop(device, self.run_loop, kCFRunLoopDefaultMode);
        }
        self.seized.push(device);
    }

    fn seize(&mut self) {
        self.active = true;
        self.failed = 0;
        if dry_run() {
            self.report();
            return;
        }
        for device in self.devices() {
            self.seize_device(device);
        }
        log(&format!(
            "keyboards taken: {}, not possible: {}",
            self.seized.len(),
            self.failed
        ));
        self.report();
    }

    fn release(&mut self) {
        self.active = false;
        for device in self.seized.drain(..) {
            unsafe {
                IOHIDDeviceRegisterInputValueCallback(device, None, std::ptr::null_mut());
                IOHIDDeviceUnscheduleFromRunLoop(device, self.run_loop, kCFRunLoopDefaultMode);
                IOHIDDeviceClose(device, 0);
                CFRelease(device.cast());
            }
        }
        log("keyboards given back");
        self.report();
    }

    fn report(&self) {
        send(&FromHelper::Status {
            seized: self.active,
            devices: self.seized.len() as u16,
            failed: self.failed,
        });
    }
}

/// Keyboards that appeared (also the ones present at start); the main loop takes them if
/// keyboards are taken right now.
static ADDED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

extern "C" fn device_added(_context: *mut c_void, _result: i32, _sender: *mut c_void, device: Ref) {
    unsafe { CFRetain(device.cast()) };
    ADDED.lock().expect("added lock").push(device as usize);
}

/// The user allowed to talk to the helper: `--user UID` from the launchd job, else the user at
/// the screen.
fn allowed_user() -> Option<u32> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(uid) = args
        .iter()
        .position(|a| a == "--user")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
    {
        return Some(uid);
    }
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/dev/console").ok().map(|m| m.uid())
}

enum Incoming {
    Connected,
    Message(ToHelper),
    Gone,
}

/// Accepts the app's connections; only the allowed user (or root) may connect. A new connection
/// replaces the previous one.
fn serve(events: Sender<Incoming>) {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log(&format!("cannot open {path}: {e}"));
            std::process::exit(1);
        }
    };
    // Everyone may connect, the peer check below decides
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let (mut uid, mut gid) = (u32::MAX, u32::MAX);
        if unsafe { getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
            continue;
        }
        let own = uid == unsafe { getuid() };
        if uid != 0 && !own && Some(uid) != allowed_user() {
            log(&format!("refused a connection from user {uid}"));
            continue;
        }
        let Ok(reader) = stream.try_clone() else { continue };
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        *CLIENT.lock().expect("client lock") = Some(BufWriter::new(stream));
        let _ = events.send(Incoming::Connected);
        let events = events.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            while let Ok(message) = read_frame::<ToHelper>(&mut reader) {
                if events.send(Incoming::Message(message)).is_err() {
                    return;
                }
            }
            let _ = events.send(Incoming::Gone);
        });
    }
}

pub fn run() {
    let access = dry_run() || unsafe { IOHIDCheckAccess(LISTEN_EVENT) } == ACCESS_GRANTED;
    if !access {
        // Puts the helper into System Settings → Privacy & Security → Input Monitoring
        unsafe { IOHIDRequestAccess(LISTEN_EVENT) };
    }
    log(&format!(
        "started, version {HELPER_VERSION}, input monitoring {}",
        if access { "granted" } else { "missing" }
    ));
    let started = Instant::now();
    let run_loop = unsafe { CFRunLoopGetCurrent() };
    let mut keyboards = Keyboards::new(run_loop);

    let (tx, rx): (Sender<Incoming>, Receiver<Incoming>) = mpsc::channel();
    std::thread::spawn(move || serve(tx));
    let mut switch = DeadManSwitch::new(Instant::now());
    loop {
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 0) };
        let now = Instant::now();
        while let Ok(event) = rx.try_recv() {
            match event {
                Incoming::Connected => {
                    switch.disconnected();
                    send(&FromHelper::Hello {
                        version: HELPER_VERSION,
                        access,
                    });
                    keyboards.report();
                }
                Incoming::Message(message) => switch.message(&message, now),
                Incoming::Gone => switch.disconnected(),
            }
        }
        let added: Vec<usize> = std::mem::take(&mut *ADDED.lock().expect("added lock"));
        for device in added {
            if keyboards.active {
                keyboards.seize_device(device as Ref);
                keyboards.report();
            } else {
                unsafe { CFRelease(device as *const c_void) };
            }
        }
        let want = switch.should_seize(now);
        if want && !keyboards.active {
            keyboards.seize();
        } else if !want && keyboards.active {
            keyboards.release();
        }
        if !access && !keyboards.active && started.elapsed() > RESTART_WITHOUT_ACCESS {
            log("restarting to pick up the Input Monitoring permission");
            std::process::exit(0);
        }
    }
}
