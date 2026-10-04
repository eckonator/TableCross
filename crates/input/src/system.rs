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
    pub fn new() -> Option<Self> {
        #[cfg(target_os = "macos")]
        {
            let kind = mac::cf("PreventUserIdleDisplaySleep");
            let name = mac::cf(REASON);
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
            let mut reason: Vec<u16> = REASON.encode_utf16().chain(Some(0)).collect();
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
                PowerSetRequest(handle, PowerRequestDisplayRequired);
                PowerSetRequest(handle, PowerRequestSystemRequired);
            }
            Some(Self { handle })
        }
        #[cfg(target_os = "linux")]
        {
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
                                    InhibitFlags::Idle | InhibitFlags::Suspend,
                                    InhibitOptions::default().set_reason(REASON),
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// How many assertions with our reason exist (a running TableCross may hold one as well).
    fn listed() -> usize {
        let out = std::process::Command::new("pmset")
            .args(["-g", "assertions"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).matches(REASON).count()
    }

    #[test]
    fn keeps_awake_while_it_exists() {
        let before = listed();
        let awake = KeepAwake::new().expect("assertion");
        assert_eq!(listed(), before + 1);
        drop(awake);
        assert_eq!(listed(), before);
    }
}
