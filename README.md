# TableCross

Share one mouse, keyboard and clipboard between computers on your desk: macOS, Windows and Linux
(Wayland on GNOME and KDE Plasma, and X11). Move the pointer over the edge of one screen and keep working
on the next computer.

> **Status: early development.** Nothing to install yet.

## Goals

- **Reliable first.** Automatic reconnect after sleep, network changes and restarts; no stuck keys
  (all keys are released when the pointer leaves or a connection drops); a status page that says
  plainly what is wrong (missing permission, no connection, latency).
- **Works while password fields are active on macOS.** A small system helper reads the keyboard below
  the window server, so typing on the other computer keeps working when an app holds Secure Input.
- **Layout-independent keys.** Keys travel as USB HID codes, not characters, so German, US and other layouts
  behave like a real keyboard plugged into the other computer.
- **Simple setup.** Computers find each other on the local network and pair with a short code.
  Traffic is always encrypted.
- **Clipboard and files.** Text, rich text and images first, then copying files.
- **Windows sign-in screen.** An optional Windows service keeps control on the login and UAC screens.

Local network only, no cloud, no account, no telemetry.

## Building

Requires Rust 1.99 or newer.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## License

GPL-3.0-or-later, see [LICENSE](LICENSE).
