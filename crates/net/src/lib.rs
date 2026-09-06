//! Networking between TableCross peers.
//!
//! - QUIC over UDP (quinn, rustls with TLS 1.3); every device has its own certificate.
//! - Pairing with a 6-digit code (SPAKE2), afterwards only pinned certificates are accepted.
//! - mDNS discovery on all interfaces, all addresses of a peer are tried, the working one is remembered.
//! - Heartbeat every 500 ms, a peer silent for 2 s counts as lost; automatic reconnect with backoff,
//!   immediate reconnect after the computer woke from sleep.
//!
//! All peers are equal: each node accepts connections and dials its paired peers. If both dial at the
//! same moment, both sides keep the same one of the two connections (see `Node` for the rule).

mod discovery;
mod identity;
mod link;
mod node;
mod pairing;
mod store;
mod tls;
mod wire;

#[cfg(test)]
mod tests;

use std::net::SocketAddr;

pub use discovery::{Discovered, SERVICE_TYPE};
pub use identity::{Fingerprint, Identity, IdentityError};
pub use link::{InStream, LinkInfo, OutStream};
pub use node::{Disconnect, Event, Node, NodeConfig, NodeError, PairError, PeerStatus, Refusal, StreamSlot};
pub use pairing::{MAX_ATTEMPTS, PAIRING_WINDOW, PairingCode};
pub use store::{PairedPeer, PeerStore};

/// UDP port used when nothing else is configured.
pub const DEFAULT_PORT: u16 = 24800;

/// IPv4 addresses seen through a dual-stack socket arrive as `::ffff:a.b.c.d`; turn them back.
pub fn canonical(address: SocketAddr) -> SocketAddr {
    match address {
        SocketAddr::V6(v6) => match v6.ip().to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(v4.into(), v6.port()),
            None => address,
        },
        v4 => v4,
    }
}

/// The settings folder (`TableCross` in the platform's config directory), unless `TC_CONFIG_DIR` is set.
pub fn default_config_dir() -> Option<std::path::PathBuf> {
    if let Some(dir) = std::env::var_os("TC_CONFIG_DIR") {
        return Some(dir.into());
    }
    config_base().map(|d| d.join("TableCross"))
}

#[cfg(target_os = "macos")]
fn config_base() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Library/Application Support"))
}

#[cfg(windows)]
fn config_base() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(Into::into)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn config_base() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(Into::into)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
}

/// A fresh, empty folder for one test.
#[cfg(test)]
pub(crate) fn test_dir(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tablecross-test-{name}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
