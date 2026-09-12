# TableCross

Share one mouse, keyboard and clipboard between computers on your desk: macOS, Windows and Linux
(Wayland on GNOME and KDE Plasma, and X11). Move the pointer over the edge of one screen and keep working
on the next computer.

> **Status: early development.** Test builds are on the
> [releases page](https://github.com/eckonator/TableCross/releases); expect rough edges.

## Installing

**macOS 11 or newer** (Apple silicon and Intel): paste into Terminal

```sh
curl -fsSL https://raw.githubusercontent.com/eckonator/TableCross/main/scripts/install-macos.sh | bash
```

or open the `.dmg` and drag TableCross to Applications. TableCross is not notarized by Apple, so a copy
from the `.dmg` or `.zip` is blocked by Gatekeeper at first; allow it once in Terminal:

```sh
xattr -dr com.apple.quarantine /Applications/TableCross.app
```

On first start macOS asks for the
**Accessibility** permission (needed to use mouse and keyboard) and possibly for access to the local
network. The setup offers the optional system helper for password fields (see below).

**Windows 10 and 11** (64-bit): run `TableCross-…-Windows-Setup.exe`. It installs the app, the optional
service for the sign-in screen and elevated programs, and opens UDP port 24800 for private networks.
There is also a portable `.zip`.

**Linux** (X11, or Wayland on GNOME 45+ / KDE Plasma 6.1+), x86_64 or arm64: unpack the `.tar.gz` and
run `./install.sh` (installs for the current user; `./install.sh --uninstall` removes it).

**Raspberry Pi 4/5** (64-bit Raspberry Pi OS): use the `arm64` archive. The default desktop (labwc or
Wayfire on Wayland) offers no way for programs to read or send input, so switch to X11 in
`sudo raspi-config` → Advanced Options → Wayland → X11.

### Antivirus warnings

TableCross reads the keyboard and mouse to send them to the other computer. Programs that do this
look like keyloggers to some antivirus software, so a warning is possible, especially on Windows.
TableCross only sends input to computers you paired yourself, encrypted, on your local network; the
source code is open for anyone to check.

## The app

`tablecross` opens a window with three setup steps (permissions, pairing with a code, arranging the
computers by dragging) and a status page per computer (connection, round trip, problems with a way
out, recent messages, “copy diagnostics”). Closing the window keeps TableCross running in the menu
bar (macOS) or notification area (Windows). The interface is in English and German.

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

On Linux, X11 sessions work directly. Wayland sessions use the desktop portals (GNOME 45 or newer,
KDE Plasma 6.1 or newer): the system asks once for permission to control this computer, and again
when TableCross starts capturing. While a Wayland computer is controlled from elsewhere, its own
mouse cannot take control back (the system does not tell programs about input); use the keyboard
shortcut on the controlling computer instead.

## Building

Requires Rust 1.99 or newer.

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Command line tool

`tablecross-cli` connects computers without the app, for development and testing:

```sh
tablecross-cli pair            # on the first computer: shows a 6-digit code
tablecross-cli pair 123456     # on the second computer: finds the first one and pairs
tablecross-cli ping            # on both: keeps the connection, prints round trip, losses and reconnects
tablecross-cli peers           # paired computers and the ones found on the network
```

Sharing mouse and keyboard (both computers paired):

```sh
tablecross-cli place office-pc right   # on the Mac: the PC stands right of it
tablecross-cli place macbook left      # on the PC: the Mac stands left of it
tablecross-cli run                     # on both
```

Move the pointer over the edge to control the other computer; using a computer's own mouse or
keyboard takes control back. Ctrl+Shift+Alt+Esc releases everything at once. `--swap-cmd-ctrl` on
`place` swaps ⌘ and Ctrl while controlling that computer. On macOS the terminal (later the app) needs
the Accessibility permission.

On macOS, typing on the other computer stops working while a program holds Secure Input (password
fields, some terminals and password managers). The system helper reads the keyboard below that
barrier and takes it away from the Mac only while the pointer is on the other computer:

```sh
tablecross-cli helper install   # asks for the administrator password
tablecross-cli helper status    # then allow it under Privacy & Security → Input Monitoring
```

If TableCross stops or crashes, the helper gives the keyboard back within a second.

On Windows, the optional service lets TableCross operate the sign-in screen, UAC prompts and programs
started as administrator (run once in an administrator prompt):

```sh
tablecross-service install
```

Without network discovery (`--no-discovery`) pass the address of the other computer with `--addr IP:PORT`.
Traffic uses UDP port 24800; allow it in the firewall for private networks.

## License

GPL-3.0-or-later, see [LICENSE](LICENSE).
