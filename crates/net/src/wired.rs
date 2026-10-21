//! Which of this computer's addresses are on a cable (Ethernet, a Thunderbolt bridge) rather than
//! on Wi-Fi. Peers dial those first: less delay, no radio dropouts. Only this computer can tell, as
//! a laptop on a dock often has an address of each kind in the same network.

use std::net::IpAddr;

/// The addresses of interfaces that are up and not Wi-Fi, loopback, a tunnel or Apple's peer links.
pub fn addresses() -> Vec<IpAddr> {
    let mut out: Vec<IpAddr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback() && !i.is_p2p && i.oper_status != if_addrs::IfOperStatus::Down)
        .filter(|i| !virtual_or_radio(&i.name))
        .map(|i| i.ip())
        .filter(|ip| !matches!(ip, IpAddr::V6(v6) if v6.is_unicast_link_local()))
        .collect();
    out.sort();
    out.dedup();
    out
}

fn virtual_or_radio(name: &str) -> bool {
    [
        "utun", "awdl", "llw", "tun", "tap", "wg", "docker", "veth", "virbr", "vmnet",
    ]
    .iter()
    .any(|p| name.starts_with(p))
        || wifi(name)
}

/// Whether the interface with this name is Wi-Fi.
#[cfg(target_os = "macos")]
fn wifi(name: &str) -> bool {
    // "Hardware Port: Wi-Fi" followed by "Device: en0"
    static DEVICES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    DEVICES
        .get_or_init(|| {
            let out = std::process::Command::new("networksetup")
                .arg("-listallhardwareports")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default();
            let mut devices = Vec::new();
            let mut radio = false;
            for line in out.lines() {
                if let Some(port) = line.strip_prefix("Hardware Port: ") {
                    radio = port.contains("Wi-Fi") || port.contains("AirPort");
                } else if let Some(device) = line.strip_prefix("Device: ")
                    && radio
                {
                    devices.push(device.trim().to_string());
                }
            }
            devices
        })
        .iter()
        .any(|d| d == name)
}

#[cfg(target_os = "linux")]
fn wifi(name: &str) -> bool {
    let dir = std::path::Path::new("/sys/class/net").join(name);
    dir.join("wireless").exists() || dir.join("phy80211").exists()
}

/// Windows names its adapters for people ("WLAN", "Wi-Fi", "Ethernet"); the radio ones say so.
#[cfg(windows)]
fn wifi(name: &str) -> bool {
    let name = name.to_lowercase();
    ["wi-fi", "wifi", "wlan", "wireless", "drahtlos"]
        .iter()
        .any(|w| name.contains(w))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn wifi(_name: &str) -> bool {
    true
}

/// The addresses for the announcement, as few as fit into one TXT entry.
pub fn announced(addresses: &[IpAddr]) -> String {
    let mut out = String::new();
    for ip in addresses {
        let text = ip.to_string();
        if out.len() + text.len() + 1 > 200 {
            break;
        }
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(&text);
    }
    out
}

/// The addresses a peer announced as being on a cable.
pub fn parse(text: &str) -> Vec<IpAddr> {
    text.split(',').filter_map(|a| a.trim().parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announced_addresses_come_back() {
        let ips: Vec<IpAddr> = vec!["192.168.178.169".parse().unwrap(), "fd00::5".parse().unwrap()];
        assert_eq!(parse(&announced(&ips)), ips);
        assert_eq!(parse(""), Vec::<IpAddr>::new());
        // Too many for one entry: the first ones go
        let many: Vec<IpAddr> = (0..40).map(|i| IpAddr::from([10, 0, 0, i])).collect();
        let text = announced(&many);
        assert!(text.len() <= 200 && parse(&text).len() < 40);
    }

    #[test]
    fn this_computer_can_tell() {
        // Whatever the machine, no loopback and nothing that panics
        assert!(addresses().iter().all(|ip| !ip.is_loopback()));
    }
}
