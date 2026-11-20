//! Live status snapshots and runtime control commands shared with the GUI and CLI.

use std::net::{IpAddr, SocketAddr, UdpSocket};

use deskglide_proto::ScreenEdge;

/// Current state of the Host or Client engine.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EngineStatus {
    #[default]
    Idle,
    Listening {
        bind_addr: SocketAddr,
        lan_ip: Option<IpAddr>,
        secret: String,
        fingerprint: String,
    },
    Connecting {
        target: SocketAddr,
        attempt: u32,
    },
    Connected {
        peer_name: String,
        peer_os: String,
        peer_addr: SocketAddr,
        controlling_remote: bool,
        rtt_ms: Option<u64>,
    },
    Error(String),
}

/// Live control commands sent to a running Host engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCommand {
    SetSwitchEdge(ScreenEdge),
    SetClipboardSync(bool),
    ToggleControl,
    Stop,
}

/// Live control commands sent to a running Client engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientCommand {
    SetClipboardSync(bool),
    Stop,
}

/// Best-effort offline probe for this machine's primary LAN IP (sends zero packets).
pub fn detect_lan_ip() -> Option<IpAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:80").ok()?;
    Some(sock.local_addr().ok()?.ip())
}
