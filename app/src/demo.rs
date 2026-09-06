//! Made-up computers for screenshots and trying the interface (`TC_DEMO=1`).

use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime};

use tc_core::Side;
use tc_net::{Disconnect, Discovered, Fingerprint, LinkInfo};
use tc_protocol::Screen;
use tc_session::{InputState, Notice, PeerView, Status};

fn screen(width: u32, height: u32, scale: f32) -> Screen {
    Screen {
        x: 0,
        y: 0,
        width,
        height,
        scale,
    }
}

pub fn status() -> Status {
    let now = SystemTime::now();
    let ago = |s: u64| now - Duration::from_secs(s);
    let address: SocketAddr = "192.168.178.24:24800".parse().expect("valid");
    Status {
        name: "MacBook".into(),
        fingerprint: Some(Fingerprint([0x5A; 32])),
        port: 24800,
        controlling: None,
        controlled_by: None,
        peers: vec![
            PeerView {
                fingerprint: Fingerprint([0x11; 32]),
                name: "Office-PC".into(),
                link: Some(LinkInfo {
                    address,
                    dialed: true,
                    connected_since: Instant::now(),
                    rtt: Some(Duration::from_micros(1830)),
                    quic_rtt: Duration::from_micros(2100),
                    probes_sent: 1240,
                    probes_lost: 1,
                    name: "Office-PC".into(),
                    screens: vec![screen(2560, 1440, 1.5)],
                }),
                reconnects: 2,
                refusal: None,
                side: Some(Side::Right),
                swap_cmd_ctrl: true,
            },
            PeerView {
                fingerprint: Fingerprint([0x22; 32]),
                name: "Linux-Laptop".into(),
                link: None,
                reconnects: 0,
                refusal: None,
                side: None,
                swap_cmd_ctrl: false,
            },
        ],
        discovered: vec![Discovered {
            fingerprint: Fingerprint([0x33; 32]),
            name: "Studio-Mac".into(),
            addresses: vec!["192.168.178.31:24800".parse().expect("valid")],
            pairing: true,
            version: tc_protocol::PROTOCOL_VERSION,
        }],
        pairing: None,
        pairing_busy: false,
        input: InputState::Running,
        problems: vec![tc_input::Problem::HelperMissing],
        screens: vec![screen(1512, 982, 1.0), screen(2560, 1440, 1.0)],
        notices: [
            (ago(3600), Notice::Connected("Office-PC".into())),
            (
                ago(1800),
                Notice::Disconnected("Office-PC".into(), Disconnect::PeerSlept),
            ),
            (ago(1795), Notice::Connected("Office-PC".into())),
            (ago(240), Notice::Controlling("Office-PC".into())),
            (ago(12), Notice::Local),
        ]
        .into(),
        layout: Default::default(),
        transfers: vec![
            tc_session::TransferView {
                id: 1,
                peer: "Office-PC".into(),
                incoming: true,
                label: "Fotos, Notizen.txt".into(),
                count: 1240,
                done: 1_310_000_000,
                total: 2_050_000_000,
                state: tc_session::TransferState::Running,
            },
            tc_session::TransferView {
                id: 2,
                peer: "Office-PC".into(),
                incoming: false,
                label: "Angebot.pdf".into(),
                count: 1,
                done: 350_000,
                total: 350_000,
                state: tc_session::TransferState::Done(Some("/Users/demo/Downloads/TableCross".into())),
            },
        ],
        files_waiting: None,
    }
}
