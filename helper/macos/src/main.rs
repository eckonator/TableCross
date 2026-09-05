//! TableCross system helper for macOS (LaunchDaemon, runs as root).
//!
//! Reads keyboards through IOKit, below the window server, so typing on another computer keeps
//! working while a program on this Mac holds Secure Input. It takes the keyboards away from the
//! system only while the TableCross app asks for it (pointer on another computer) and gives them
//! back at once when the app says so, disconnects, crashes or stops sending signs of life.

#[cfg(target_os = "macos")]
mod mac;

fn main() {
    #[cfg(target_os = "macos")]
    mac::run();
    #[cfg(not(target_os = "macos"))]
    eprintln!("tablecross-helper is only needed on macOS");
}
