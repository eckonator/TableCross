//! Paired peers, saved as JSON in the settings folder.
//!
//! Loading is tolerant: unknown fields are ignored and missing ones get defaults, so files written by
//! newer or older versions still load. A file that cannot be read at all is kept aside as `.broken`.

use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::identity::Fingerprint;

const FILE: &str = "peers.json";
/// Remembered addresses per peer; the most recent success comes first.
const MAX_ADDRESSES: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedPeer {
    pub fingerprint: Fingerprint,
    #[serde(default)]
    pub name: String,
    /// Addresses that worked before, most recent first.
    #[serde(default)]
    pub addresses: Vec<SocketAddr>,
    /// Seconds since 1970.
    #[serde(default)]
    pub paired_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    peers: Vec<PairedPeer>,
}

#[derive(Debug)]
pub struct PeerStore {
    /// `None` keeps everything in memory (tests).
    path: Option<PathBuf>,
    peers: Vec<PairedPeer>,
}

impl PeerStore {
    pub fn in_memory() -> Self {
        Self {
            path: None,
            peers: Vec::new(),
        }
    }

    pub fn load(dir: &Path) -> io::Result<Self> {
        let path = dir.join(FILE);
        let peers = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<File>(&bytes) {
                Ok(file) => file.peers,
                Err(e) => {
                    tracing::warn!(
                        "{} is unreadable ({e}), keeping it as peers.json.broken",
                        path.display()
                    );
                    fs::rename(&path, path.with_extension("json.broken"))?;
                    Vec::new()
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        let mut store = Self {
            path: Some(path),
            peers,
        };
        let mut seen = std::collections::HashSet::new();
        store.peers.retain(|p| seen.insert(p.fingerprint));
        Ok(store)
    }

    pub fn peers(&self) -> &[PairedPeer] {
        &self.peers
    }

    pub fn get(&self, fingerprint: &Fingerprint) -> Option<&PairedPeer> {
        self.peers.iter().find(|p| p.fingerprint == *fingerprint)
    }

    pub fn contains(&self, fingerprint: &Fingerprint) -> bool {
        self.get(fingerprint).is_some()
    }

    /// Adds a peer or updates its name; known addresses are kept.
    pub fn insert(&mut self, fingerprint: Fingerprint, name: &str, address: Option<SocketAddr>) -> io::Result<()> {
        let paired_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        match self.peers.iter_mut().find(|p| p.fingerprint == fingerprint) {
            Some(peer) => {
                peer.name = name.to_string();
                peer.paired_at = paired_at;
            }
            None => self.peers.push(PairedPeer {
                fingerprint,
                name: name.to_string(),
                addresses: Vec::new(),
                paired_at,
            }),
        }
        if let Some(address) = address {
            self.remember_address_unsaved(&fingerprint, address);
        }
        self.save()
    }

    pub fn remove(&mut self, fingerprint: &Fingerprint) -> io::Result<bool> {
        let before = self.peers.len();
        self.peers.retain(|p| p.fingerprint != *fingerprint);
        if self.peers.len() == before {
            return Ok(false);
        }
        self.save()?;
        Ok(true)
    }

    /// Moves `address` to the front of the peer's list. Saves only if something changed.
    pub fn remember_address(&mut self, fingerprint: &Fingerprint, address: SocketAddr) -> io::Result<()> {
        if self.remember_address_unsaved(fingerprint, address) {
            self.save()?;
        }
        Ok(())
    }

    pub fn set_name(&mut self, fingerprint: &Fingerprint, name: &str) -> io::Result<()> {
        match self.peers.iter_mut().find(|p| p.fingerprint == *fingerprint) {
            Some(peer) if peer.name != name => {
                peer.name = name.to_string();
                self.save()
            }
            _ => Ok(()),
        }
    }

    fn remember_address_unsaved(&mut self, fingerprint: &Fingerprint, address: SocketAddr) -> bool {
        let Some(peer) = self.peers.iter_mut().find(|p| p.fingerprint == *fingerprint) else {
            return false;
        };
        let address = crate::canonical(address);
        if peer.addresses.first() == Some(&address) {
            return false;
        }
        peer.addresses.retain(|a| *a != address);
        peer.addresses.insert(0, address);
        peer.addresses.truncate(MAX_ADDRESSES);
        true
    }

    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(&File {
            peers: self.peers.clone(),
        })
        .map_err(io::Error::other)?;
        // Write next to it and rename, so a crash never leaves a half-written file
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(n: u8) -> Fingerprint {
        Fingerprint([n; 32])
    }

    #[test]
    fn peers_survive_reload() {
        let dir = crate::test_dir("store");
        let mut store = PeerStore::load(&dir).unwrap();
        store
            .insert(fp(1), "Office Mac", Some("192.168.1.5:24800".parse().unwrap()))
            .unwrap();
        store.insert(fp(2), "Gaming PC", None).unwrap();
        let reloaded = PeerStore::load(&dir).unwrap();
        assert_eq!(reloaded.peers(), store.peers());
        assert_eq!(reloaded.get(&fp(1)).unwrap().addresses.len(), 1);
    }

    #[test]
    fn latest_address_comes_first_without_duplicates() {
        let mut store = PeerStore::in_memory();
        store.insert(fp(1), "a", None).unwrap();
        let a: SocketAddr = "10.0.0.1:1".parse().unwrap();
        let b: SocketAddr = "10.0.0.2:1".parse().unwrap();
        store.remember_address(&fp(1), a).unwrap();
        store.remember_address(&fp(1), b).unwrap();
        store.remember_address(&fp(1), a).unwrap();
        assert_eq!(store.get(&fp(1)).unwrap().addresses, vec![a, b]);
        // IPv4-mapped IPv6 addresses count as the IPv4 address
        store
            .remember_address(&fp(1), "[::ffff:10.0.0.2]:1".parse().unwrap())
            .unwrap();
        assert_eq!(store.get(&fp(1)).unwrap().addresses, vec![b, a]);
        for i in 0..20 {
            store
                .remember_address(&fp(1), SocketAddr::from(([10, 1, 0, i], 1)))
                .unwrap();
        }
        assert_eq!(store.get(&fp(1)).unwrap().addresses.len(), MAX_ADDRESSES);
    }

    #[test]
    fn tolerant_loading() {
        let dir = crate::test_dir("store-tolerant");
        let json = format!(
            r#"{{"peers":[{{"fingerprint":"{}","future_field":1}}],"other":true}}"#,
            fp(3)
        );
        fs::write(dir.join(FILE), json).unwrap();
        let store = PeerStore::load(&dir).unwrap();
        assert_eq!(store.peers().len(), 1);
        assert_eq!(store.peers()[0].name, "");

        fs::write(dir.join(FILE), "not json").unwrap();
        let store = PeerStore::load(&dir).unwrap();
        assert!(store.peers().is_empty());
        assert!(dir.join("peers.json.broken").exists());
    }

    #[test]
    fn remove_peer() {
        let mut store = PeerStore::in_memory();
        store.insert(fp(1), "a", None).unwrap();
        assert!(store.remove(&fp(1)).unwrap());
        assert!(!store.remove(&fp(1)).unwrap());
        assert!(!store.contains(&fp(1)));
    }
}
