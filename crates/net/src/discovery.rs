//! Finding peers on the local network with mDNS (`_tablecross._udp`).
//!
//! Every node announces itself on all interfaces with its fingerprint in the TXT record and collects
//! the announcements of others. All addresses of a peer are kept; the caller tries them all.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, SocketAddrV6};
use std::sync::{Arc, Mutex};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tc_protocol::PROTOCOL_VERSION;
use tokio::sync::watch;

use crate::identity::Fingerprint;

pub const SERVICE_TYPE: &str = "_tablecross._udp.local.";

/// A TableCross instance seen on the network.
#[derive(Debug, Clone, PartialEq)]
pub struct Discovered {
    pub fingerprint: Fingerprint,
    pub name: String,
    /// IPv4 first, then IPv6.
    pub addresses: Vec<SocketAddr>,
    /// It currently shows a pairing code.
    pub pairing: bool,
    pub version: u16,
}

pub(crate) struct Discovery {
    daemon: ServiceDaemon,
    own: Fingerprint,
    name: String,
    port: u16,
    fullname: Mutex<Option<String>>,
    found: Arc<Mutex<HashMap<String, Discovered>>>,
}

impl Discovery {
    /// Starts announcing and browsing. `changed` is bumped whenever the list of found peers changes.
    pub fn start(own: Fingerprint, name: &str, port: u16, changed: watch::Sender<()>) -> Result<Self, mdns_sd::Error> {
        let daemon = ServiceDaemon::new()?;
        let found = Arc::new(Mutex::new(HashMap::new()));
        let receiver = daemon.browse(SERVICE_TYPE)?;
        let found_task = found.clone();
        tokio::spawn(async move {
            while let Ok(event) = receiver.recv_async().await {
                let mut found = found_task.lock().expect("discovery lock");
                let change = match event {
                    ServiceEvent::ServiceResolved(service) => match parse(&service) {
                        Some(peer) if peer.fingerprint != own => {
                            found.insert(service.fullname.clone(), peer.clone()) != Some(peer)
                        }
                        _ => false,
                    },
                    ServiceEvent::ServiceRemoved(_, fullname) => found.remove(&fullname).is_some(),
                    _ => false,
                };
                drop(found);
                if change {
                    changed.send_replace(());
                }
            }
        });
        let discovery = Self {
            daemon,
            own,
            name: name.to_string(),
            port,
            fullname: Mutex::new(None),
            found,
        };
        discovery.announce(false)?;
        Ok(discovery)
    }

    /// (Re-)announces this node, with the pairing flag set while a code is shown.
    pub fn announce(&self, pairing: bool) -> Result<(), mdns_sd::Error> {
        let short = self.own.short();
        let name: String = self.name.chars().take(40).collect();
        let properties: HashMap<String, String> = [
            ("fp".to_string(), self.own.to_string()),
            ("v".to_string(), PROTOCOL_VERSION.to_string()),
            ("name".to_string(), name.clone()),
            ("pair".to_string(), if pairing { "1" } else { "0" }.to_string()),
        ]
        .into();
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &format!("{name} ({short})"),
            &format!("tablecross-{short}.local."),
            "",
            self.port,
            properties,
        )?
        .enable_addr_auto();
        *self.fullname.lock().expect("discovery lock") = Some(info.get_fullname().to_string());
        self.daemon.register(info)
    }

    pub fn found(&self) -> Vec<Discovered> {
        let mut list: Vec<_> = self.found.lock().expect("discovery lock").values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name).then(a.fingerprint.cmp(&b.fingerprint)));
        list.dedup_by_key(|d| d.fingerprint);
        list
    }

    pub fn shutdown(&self) {
        if let Some(fullname) = self.fullname.lock().expect("discovery lock").take() {
            let _ = self.daemon.unregister(&fullname);
        }
        let _ = self.daemon.shutdown();
    }
}

fn parse(service: &mdns_sd::ResolvedService) -> Option<Discovered> {
    let props = &service.txt_properties;
    let fingerprint = props.get_property_val_str("fp")?.parse().ok()?;
    let version = props
        .get_property_val_str("v")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let name = props.get_property_val_str("name").unwrap_or_default().to_string();
    let pairing = props.get_property_val_str("pair") == Some("1");
    let mut addresses: Vec<SocketAddr> = service
        .addresses
        .iter()
        .filter(|ip| !ip.is_loopback())
        .map(|ip| match ip {
            mdns_sd::ScopedIp::V6(v6) => {
                SocketAddr::V6(SocketAddrV6::new(*v6.addr(), service.port, 0, v6.scope_id().index))
            }
            other => SocketAddr::new(other.to_ip_addr(), service.port),
        })
        .collect();
    // IPv4 first, then global IPv6, then link-local
    addresses.sort_by_key(|a| match a.ip() {
        IpAddr::V4(_) => 0,
        IpAddr::V6(v6) if v6.is_unicast_link_local() => 2,
        IpAddr::V6(_) => 1,
    });
    Some(Discovered {
        fingerprint,
        name,
        addresses,
        pairing,
        version,
    })
}
