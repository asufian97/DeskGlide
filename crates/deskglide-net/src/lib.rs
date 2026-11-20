//! Encrypted QUIC networking, TLS 1.3 certificate pinning, channel-bound auth,
//! mDNS zero-config discovery, and persistent user configuration.

pub mod auth;
pub mod config;
pub mod discovery;
pub mod session;
pub mod tls;

pub use auth::generate_pairing_secret;
pub use config::{AppConfig, AppMode, PairedHost};
pub use discovery::{DiscoveredHost, MdnsAdvertiser, MdnsBrowser, MDNS_SERVICE_TYPE};
pub use session::{KvmClient, KvmServer, KvmSession};
pub use tls::{
    cert_fingerprint, config_dir, format_fingerprint, load_or_create_identity, parse_fingerprint,
    PinPolicy, ServerIdentity,
};
