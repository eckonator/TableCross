//! Things of the whole system: keeping the computer awake while it is used across computers, locking
//! the screen and shutting down.
//!
//! - macOS: a power assertion (`IOPMAssertionCreateWithName`), locking through the login framework,
//!   shutting down through the login window.
//! - Windows: a power request (`PowerCreateRequest`), `LockWorkStation`, `ExitWindowsEx`.
//! - Linux: the Inhibit portal of the desktop, `loginctl lock-session`, the session manager of the
//!   desktop or `systemctl poweroff`.

/// Keeps display and system awake while it exists. Without it, a Mac whose keyboard the system
/// helper holds counts as unused while someone types on another computer, and goes to sleep.
pub struct KeepAwake {
    #[cfg(target_os = "macos")]
    id: u32,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(target_os = "linux")]
    release: Option<tokio::sync::oneshot::Sender<()>>,
}

// The Windows handle is only used to clear the request again
unsafe impl Send for KeepAwake {}
unsafe impl Sync for KeepAwake {}

const REASON: &str = "Mouse and keyboard are shared with another computer";
const SYSTEM_REASON: &str = "Stays reachable for the other computers of TableCross";

#[cfg(target_os = "macos")]
mod mac {
    use core_foundation_sys::base::CFRelease;
    use core_foundation_sys::string::{CFStringCreateWithCString, CFStringRef, kCFStringEncodingUTF8};
    use std::ffi::{CString, c_char, c_void};

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        pub fn IOPMAssertionCreateWithName(kind: CFStringRef, level: u32, name: CFStringRef, id: *mut u32) -> i32;
        pub fn IOPMAssertionRelease(id: u32) -> i32;
    }

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    pub fn cf(text: &str) -> CFStringRef {
        let text = CString::new(text).expect("no NUL");
        unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), kCFStringEncodingUTF8) }
    }

    pub fn release(s: CFStringRef) {
        unsafe { CFRelease(s.cast()) };
    }

    /// The lock of the menu bar and of Control+Command+Q.
    pub fn lock_now() -> bool {
        unsafe {
            let path = c"/System/Library/PrivateFrameworks/login.framework/Versions/Current/login";
            let handle = dlopen(path.as_ptr(), 1);
            if handle.is_null() {
                return false;
            }
            let symbol = dlsym(handle, c"SACLockScreenImmediate".as_ptr());
            if symbol.is_null() {
                return false;
            }
            let lock: extern "C" fn() -> i32 = std::mem::transmute(symbol);
            lock() == 0
        }
    }
}

impl KeepAwake {
    /// Display and system stay awake.
    pub fn new() -> Option<Self> {
        Self::with(true, REASON)
    }

    /// Only the system stays awake (reachable on the network); the display may still turn off.
    pub fn system_only() -> Option<Self> {
        Self::with(false, SYSTEM_REASON)
    }

    fn with(display: bool, reason: &str) -> Option<Self> {
        #[cfg(target_os = "macos")]
        {
            let kind = mac::cf(if display {
                "PreventUserIdleDisplaySleep"
            } else {
                "PreventUserIdleSystemSleep"
            });
            let name = mac::cf(reason);
            let mut id = 0;
            let result = unsafe { mac::IOPMAssertionCreateWithName(kind, 255, name, &mut id) };
            mac::release(kind);
            mac::release(name);
            (result == 0).then_some(Self { id })
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Power::{
                PowerCreateRequest, PowerRequestDisplayRequired, PowerRequestSystemRequired, PowerSetRequest,
            };
            use windows_sys::Win32::System::SystemServices::POWER_REQUEST_CONTEXT_VERSION;
            use windows_sys::Win32::System::Threading::{
                POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
            };
            let mut reason: Vec<u16> = reason.encode_utf16().chain(Some(0)).collect();
            let context = REASON_CONTEXT {
                Version: POWER_REQUEST_CONTEXT_VERSION,
                Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
                Reason: REASON_CONTEXT_0 {
                    SimpleReasonString: reason.as_mut_ptr(),
                },
            };
            let handle = unsafe { PowerCreateRequest(&context) };
            if handle.is_null() || handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
                return None;
            }
            unsafe {
                if display {
                    PowerSetRequest(handle, PowerRequestDisplayRequired);
                }
                PowerSetRequest(handle, PowerRequestSystemRequired);
            }
            Some(Self { handle })
        }
        #[cfg(target_os = "linux")]
        {
            let reason = reason.to_string();
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            std::thread::Builder::new()
                .name("tablecross-awake".into())
                .spawn(move || {
                    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
                        return;
                    };
                    runtime.block_on(async move {
                        use ashpd::desktop::inhibit::{InhibitFlags, InhibitOptions, InhibitProxy};
                        let request = async {
                            let proxy = InhibitProxy::new().await?;
                            proxy
                                .inhibit(
                                    None,
                                    if display {
                                        InhibitFlags::Idle | InhibitFlags::Suspend
                                    } else {
                                        InhibitFlags::Suspend.into()
                                    },
                                    InhibitOptions::default().set_reason(reason.as_str()),
                                )
                                .await
                        }
                        .await;
                        match request {
                            Ok(request) => {
                                let _ = rx.await;
                                let _ = request.close().await;
                            }
                            Err(e) => tracing::info!("cannot keep the computer awake: {e}"),
                        }
                    });
                })
                .ok()?;
            Some(Self { release: Some(tx) })
        }
        #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
        {
            let _ = (display, reason);
            None
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        unsafe {
            mac::IOPMAssertionRelease(self.id);
        }
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Power::{
                PowerClearRequest, PowerRequestDisplayRequired, PowerRequestSystemRequired,
            };
            PowerClearRequest(self.handle, PowerRequestDisplayRequired);
            PowerClearRequest(self.handle, PowerRequestSystemRequired);
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
        #[cfg(target_os = "linux")]
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

/// Whether the left mouse button is physically held right now (`None`: cannot tell here). Events
/// can claim otherwise: macOS sends a release of its own when a drag is cancelled.
pub fn left_button_held() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
        }
        /// `kCGEventSourceStateHIDSystemState`: what the hardware reports.
        const HID_STATE: i32 = 1;
        Some(unsafe { CGEventSourceButtonState(HID_STATE, 0) })
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// Opens a folder in the file manager.
pub fn show_folder(path: &std::path::Path) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    if let Err(e) = std::process::Command::new(program).arg(path).spawn() {
        tracing::warn!("cannot open {}: {e}", path.display());
    }
}

/// Locks the screen of this computer.
pub fn lock_screen() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if mac::lock_now() {
            return Ok(());
        }
        // Fallback: screen off; locks if the Mac asks for the password right after that
        run("pmset", &["displaysleepnow"])
    }
    #[cfg(windows)]
    {
        if unsafe { windows_sys::Win32::System::Shutdown::LockWorkStation() } != 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().to_string())
        }
    }
    #[cfg(target_os = "linux")]
    {
        run("loginctl", &["lock-session"]).or_else(|_| run("xdg-screensaver", &["lock"]))
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        Err("not supported".into())
    }
}

/// Shuts this computer down like its own menu does: applications are asked to quit and may stop it
/// for unsaved documents.
pub fn power_off() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // The "shut" event of the login window: no dialog, but applications may still object
        run(
            "osascript",
            &["-e", "tell application \"loginwindow\" to «event aevtshut»"],
        )
    }
    #[cfg(windows)]
    {
        windows_power_off()
    }
    #[cfg(target_os = "linux")]
    {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_uppercase();
        let session = if desktop.contains("GNOME") {
            run("gnome-session-quit", &["--power-off", "--no-prompt"])
        } else if desktop.contains("KDE") {
            run(
                "dbus-send",
                &[
                    "--session",
                    "--type=method_call",
                    "--dest=org.kde.Shutdown",
                    "/Shutdown",
                    "org.kde.Shutdown.logoutAndShutdown",
                ],
            )
        } else {
            Err("no session manager".into())
        };
        session.or_else(|_| run("systemctl", &["poweroff"]))
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        Err("not supported".into())
    }
}

/// Every user may shut down, but the right has to be switched on in the process token first.
#[cfg(windows)]
fn windows_power_off() -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LUID};
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, SE_SHUTDOWN_NAME,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Shutdown::{
        EWX_POWEROFF, EWX_SHUTDOWN, ExitWindowsEx, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let error = || std::io::Error::last_os_error().to_string();
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
            return Err(error());
        }
        let mut luid = LUID {
            LowPart: 0,
            HighPart: 0,
        };
        let found = LookupPrivilegeValueW(std::ptr::null(), SE_SHUTDOWN_NAME, &mut luid) != 0;
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let adjusted =
            found && AdjustTokenPrivileges(token, 0, &privileges, 0, std::ptr::null_mut(), std::ptr::null_mut()) != 0;
        CloseHandle(token);
        if !adjusted {
            return Err(error());
        }
        if ExitWindowsEx(
            EWX_SHUTDOWN | EWX_POWEROFF,
            SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_FLAG_PLANNED,
        ) == 0
        {
            return Err(error());
        }
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} failed"))
    }
}

/// With this argument the program only tells whether it has the Accessibility permission (exit
/// code 0, else 1) and ends; see [`accessibility_fresh`].
pub const ACCESS_CHECK_FLAG: &str = "--check-accessibility";

/// First thing in `main`: answers [`ACCESS_CHECK_FLAG`] and ends the process.
pub fn answer_access_check() {
    if std::env::args().nth(1).as_deref() != Some(ACCESS_CHECK_FLAG) {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        #[link(name = "ApplicationServices", kind = "framework")]
        unsafe extern "C" {
            fn AXIsProcessTrusted() -> bool;
        }
        std::process::exit(if unsafe { AXIsProcessTrusted() } { 0 } else { 1 });
    }
    #[cfg(not(target_os = "macos"))]
    std::process::exit(0);
}

/// macOS: what this process may do as macOS sees it now: a window server session to post into,
/// Accessibility, posting events, listening to events. `None` elsewhere.
pub fn event_access() -> Option<(bool, bool, bool, bool)> {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "ApplicationServices", kind = "framework")]
        unsafe extern "C" {
            fn AXIsProcessTrusted() -> bool;
            fn CGPreflightPostEventAccess() -> bool;
            fn CGPreflightListenEventAccess() -> bool;
            fn CGSessionCopyCurrentDictionary() -> *const std::ffi::c_void;
        }
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFRelease(cf: *const std::ffi::c_void);
        }
        unsafe {
            let session = CGSessionCopyCurrentDictionary();
            if !session.is_null() {
                CFRelease(session);
            }
            Some((
                !session.is_null(),
                AXIsProcessTrusted(),
                CGPreflightPostEventAccess(),
                CGPreflightListenEventAccess(),
            ))
        }
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// macOS: moves the pointer in each way there is and tells which ways macOS carried out (the
/// pointer is put back afterwards). Where injected events vanish silently this shows the way that
/// still works. `None` elsewhere.
pub fn pointer_probe() -> Option<String> {
    #[cfg(target_os = "macos")]
    return crate::mac::pointer_probe();
    #[cfg(not(target_os = "macos"))]
    None
}

/// macOS: from now on input is injected through the HID system, as if from a device. Needed on the
/// login screen, which ignores the usual way. Does nothing elsewhere.
pub fn inject_through_hid_system() {
    #[cfg(target_os = "macos")]
    crate::mac::inject_through_hid_system();
}

/// macOS: whether Accessibility is granted, asked of a fresh process of this program. A running
/// process may keep the answer it got first, so a permission granted later shows only here.
/// `None` where it cannot be told.
pub fn accessibility_fresh() -> Option<bool> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let status = std::process::Command::new(exe)
        .arg(ACCESS_CHECK_FLAG)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

/// macOS: whether `program` (one of ours that answers [`ACCESS_CHECK_FLAG`]) has the Accessibility
/// permission itself. A child normally counts as part of this app, so it is started as responsible
/// for itself. `None` where it cannot be told.
pub fn accessibility_of(program: &std::path::Path) -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::{CString, c_char, c_int, c_void};
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn posix_spawnattr_init(attr: *mut *mut c_void) -> c_int;
            fn posix_spawnattr_destroy(attr: *mut *mut c_void) -> c_int;
            // Private but stable since macOS 10.14; without it the answer would be this app's
            fn responsibility_spawnattrs_setdisclaim(attr: *mut *mut c_void, disclaim: c_int) -> c_int;
            fn posix_spawn(
                pid: *mut i32,
                path: *const c_char,
                actions: *const c_void,
                attr: *const *mut c_void,
                argv: *const *const c_char,
                envp: *const *const c_char,
            ) -> c_int;
            fn waitpid(pid: i32, status: *mut c_int, options: c_int) -> i32;
        }
        let path = CString::new(program.as_os_str().as_bytes()).ok()?;
        let flag = CString::new(ACCESS_CHECK_FLAG).ok()?;
        let argv = [path.as_ptr(), flag.as_ptr(), std::ptr::null()];
        let envp = [std::ptr::null()];
        unsafe {
            let mut attr = std::ptr::null_mut();
            if posix_spawnattr_init(&mut attr) != 0 {
                return None;
            }
            responsibility_spawnattrs_setdisclaim(&mut attr, 1);
            let mut pid = 0;
            let spawned = posix_spawn(
                &mut pid,
                path.as_ptr(),
                std::ptr::null(),
                &attr,
                argv.as_ptr(),
                envp.as_ptr(),
            );
            posix_spawnattr_destroy(&mut attr);
            if spawned != 0 {
                return None;
            }
            let mut status = 0;
            while waitpid(pid, &mut status, 0) < 0 {
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                    return None;
                }
            }
            // Ended normally: the exit code is in the second byte
            if status & 0x7f != 0 {
                return None;
            }
            match (status >> 8) & 0xff {
                0 => Some(true),
                1 => Some(false),
                _ => None,
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = program;
        None
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// How many assertions with this reason exist (a running TableCross may hold one as well).
    fn listed(reason: &str) -> usize {
        let out = std::process::Command::new("pmset")
            .args(["-g", "assertions"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).matches(reason).count()
    }

    #[test]
    fn keeps_awake_while_it_exists() {
        let before = listed(REASON);
        let awake = KeepAwake::new().expect("assertion");
        assert_eq!(listed(REASON), before + 1);
        drop(awake);
        assert_eq!(listed(REASON), before);
    }

    #[test]
    fn keeps_the_system_awake_while_it_exists() {
        let before = listed(SYSTEM_REASON);
        let awake = KeepAwake::system_only().expect("assertion");
        assert_eq!(listed(SYSTEM_REASON), before + 1);
        drop(awake);
        assert_eq!(listed(SYSTEM_REASON), before);
    }
}
