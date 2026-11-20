//! Shared async Host and Client runtime used by both `deskglide` (CLI) and `deskglide-app` (GUI).

pub mod client;
pub mod clipboard;
pub mod host;
pub mod status;

pub use client::{run_client, ClientOptions};
pub use host::{run_host, HostOptions};
pub use status::{detect_lan_ip, ClientCommand, EngineStatus, HostCommand};
