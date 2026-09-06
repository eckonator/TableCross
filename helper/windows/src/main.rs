//! TableCross Windows service.
//!
//! The service runs as SYSTEM and keeps an agent running in the session at the screen. The agent
//! follows the desktop that has the input (the normal desktop, the sign-in screen, the UAC prompt)
//! and injects what the TableCross app hands it over a named pipe. Running as SYSTEM, its input
//! also reaches programs started as administrator.
//!
//! `tablecross-service install` / `uninstall` (as administrator); `service` and `agent` are used by
//! Windows and by the service itself.

#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    win::main();
    #[cfg(not(windows))]
    eprintln!("tablecross-service is only needed on Windows");
}
