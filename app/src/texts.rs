//! Words for things the session reports, in the user's language.

use tc_core::Side;
use tc_input::Problem;
use tc_net::{Disconnect, PairError, Refusal};
use tc_session::{Notice, Skip};

use crate::i18n::{tr, trf};

pub fn notice(n: &Notice) -> String {
    match n {
        Notice::Connected(name) => trf("Connected to {}", &[name]),
        Notice::Disconnected(name, reason) => trf("Connection to {} ended: {}", &[name, &disconnect(reason)]),
        Notice::Refused(who, reason) => trf("{} was refused: {}", &[who, &refusal(reason)]),
        Notice::Paired(name) => trf("Paired with {}", &[name]),
        Notice::PairingFailed(left) => trf("A wrong code was entered ({} attempts left)", &[left]),
        Notice::PairingClosed => tr("The pairing code is no longer valid after too many attempts").into(),
        Notice::PairError(e) => trf("Pairing failed: {}", &[&pair_error(e)]),
        Notice::Unpaired(name, true) => trf("{} removed the pairing", &[name]),
        Notice::Unpaired(name, false) => trf("Pairing with {} removed", &[name]),
        Notice::Woke => tr("Woke from sleep, connecting again").into(),
        Notice::Controlling(name) => trf("Mouse and keyboard now control {}", &[name]),
        Notice::Local => tr("Mouse and keyboard work on this computer").into(),
        Notice::ControlledBy(name) => trf("{} controls this computer", &[name]),
        Notice::EmergencyRelease => tr("Everything was released").into(),
        Notice::Problem(p) => problem(p),
        Notice::InputFailed(e) => trf("Mouse and keyboard cannot be captured: {}", &[e]),
        Notice::ClipboardSkipped(Skip::Concealed) => {
            tr("Copied password stays on this computer (marked as private)").into()
        }
        Notice::ClipboardSkipped(Skip::TooLarge(n)) => trf(
            "Copied content stays on this computer: {} MB is more than allowed",
            &[&format!("{:.1}", *n as f64 / 1_048_576.0)],
        ),
        Notice::ClipboardReceived(name) => trf("Clipboard taken over from {}", &[name]),
        Notice::FilesReceived(name, count, dir) => trf(
            "{} files from {} are in {} and on the clipboard",
            &[count, name, &dir.display()],
        ),
        Notice::TransferFailed(name, e) => trf("File transfer with {} failed: {}", &[name, e]),
        Notice::LockedAll => tr("All screens were locked").into(),
        Notice::PoweringOff(names) if names.is_empty() => tr("This computer is shutting down").into(),
        Notice::PoweringOff(names) => trf("Shutting down: {}", &[&names.join(", ")]),
    }
}

pub fn disconnect(d: &Disconnect) -> String {
    match d {
        Disconnect::Timeout => tr("no answer for 2 seconds").into(),
        Disconnect::PeerShutdown => tr("the other computer quit TableCross").into(),
        Disconnect::PeerSlept => tr("the other computer went to sleep").into(),
        Disconnect::Slept => tr("this computer was asleep").into(),
        Disconnect::Replaced => tr("replaced by a newer connection").into(),
        Disconnect::Unpaired => tr("pairing removed").into(),
        Disconnect::Shutdown => tr("TableCross is shutting down").into(),
        Disconnect::ProtocolError(e) => trf("protocol error ({})", &[e]),
        Disconnect::Lost(e) => trf("network error ({})", &[e]),
    }
}

pub fn refusal(r: &Refusal) -> String {
    match r {
        Refusal::NotPaired => tr("not paired with this computer").into(),
        Refusal::VersionMismatch { ours, theirs } => trf(
            "the other computer uses version {} of the protocol, this one {}: please update the older one",
            &[theirs, ours],
        ),
        Refusal::WrongPeer => tr("another device answered at the known address").into(),
        Refusal::ProtocolError(e) => trf("protocol error ({})", &[e]),
    }
}

pub fn pair_error(e: &PairError) -> String {
    match e {
        PairError::NoAddress => tr("the other computer was not found on the network").into(),
        PairError::Unreachable => tr("the other computer cannot be reached").into(),
        PairError::OtherNetwork(ips) => {
            let ips = ips.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
            trf(
                "the other computer cannot be reached: its address {} is not in this computer's network \
                 (a virtual machine needs a bridged network adapter instead of NAT)",
                &[&ips],
            )
        }
        PairError::NotShowingCode => {
            tr("the other computer is not showing a code (expired or too many attempts)").into()
        }
        PairError::WrongCode => tr("wrong code").into(),
        PairError::VersionMismatch { ours, theirs } => trf(
            "the other computer uses version {} of the protocol, this one {}: please update the older one",
            &[theirs, ours],
        ),
        PairError::Protocol(e) => trf("protocol error ({})", &[e]),
        PairError::Store(e) => trf("cannot save the pairing ({})", &[e]),
    }
}

/// macOS ties a permission to the exact program; after an update the old entry can stay switched on
/// without counting. Switching it off and on is then not enough.
const STALE_PERMISSION: &str = "Already switched on and still shown here (often after an update)? Select {} in \
                                the list, remove it with “–” and add it again with “+”.";

pub fn problem(p: &Problem) -> String {
    match p {
        Problem::AccessibilityMissing => format!(
            "{} {}",
            tr("TableCross needs the Accessibility permission to use mouse and keyboard."),
            trf(STALE_PERMISSION, &[&"TableCross"])
        ),
        Problem::SecureInput => tr(
            "A program holds Secure Input (password field): typing does not reach other computers right now. \
             The system helper solves this.",
        )
        .into(),
        Problem::CaptureStopped => tr("Mouse and keyboard are not captured right now.").into(),
        Problem::HelperMissing => tr(
            "The system helper is not installed. Without it, typing on other computers stops while a password \
             field is active on this Mac.",
        )
        .into(),
        Problem::HelperNoAccess => format!(
            "{} {} {}",
            tr("The system helper still needs the Input Monitoring permission."),
            tr(
                "The button opens the settings and a Finder window with the helper file: switch it on there, or \
                drag the file into the list."
            ),
            trf(STALE_PERMISSION, &[&"de.eckonator.tablecross.helper"])
        ),
        Problem::HelperOutdated => tr("The system helper is older than the app; please update it.").into(),
        Problem::HelperKeyboardsBusy(n) => trf(
            "The system helper could not take {} keyboard(s); their keys also reach this Mac.",
            &[n],
        ),
        Problem::HooksRemoved(n) => trf(
            "Windows switched off the input capture {} times; it was restarted.",
            &[n],
        ),
        Problem::UniversalControl => tr(
            "Universal Control of macOS is switched on. It also moves the pointer to other Macs at the screen \
             edges, gets in the way of TableCross and takes the pointer over even when TableCross is quit.",
        )
        .into(),
    }
}

/// "right of this computer"
pub fn side(s: Option<Side>) -> &'static str {
    match s {
        Some(Side::Left) => tr("left of this computer"),
        Some(Side::Right) => tr("right of this computer"),
        Some(Side::Top) => tr("above this computer"),
        Some(Side::Bottom) => tr("below this computer"),
        None => tr("not placed"),
    }
}

/// "5 s", "3 min", "2 h" since `t`.
pub fn ago(t: std::time::SystemTime) -> String {
    let secs = t.elapsed().map_or(0, |d| d.as_secs());
    match secs {
        0..=4 => tr("now").into(),
        5..=59 => trf("{} s ago", &[&secs]),
        60..=3599 => trf("{} min ago", &[&(secs / 60)]),
        _ => trf("{} h ago", &[&(secs / 3600)]),
    }
}

/// "1.4 GB", "350 KB"
pub fn bytes(n: u64) -> String {
    let n = n as f64;
    if n >= 1e9 {
        format!("{:.1} GB", n / 1e9)
    } else if n >= 1e6 {
        format!("{:.1} MB", n / 1e6)
    } else if n >= 1e3 {
        format!("{:.0} KB", n / 1e3)
    } else {
        format!("{n:.0} B")
    }
}
