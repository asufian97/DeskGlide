//! Zero-config local Wi-Fi/LAN host advertisement and discovery via mDNS.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use deskglide_proto::{OsKind, PROTOCOL_VERSION};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::watch;
use tracing::{debug, warn};

pub const MDNS_SERVICE_TYPE: &str = "_deskglide._udp.local.";

/// A DeskGlide Host discovered on the local Wi-Fi/LAN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredHost {
    pub fullname: String,
    pub name: String,
    pub addr: SocketAddr,
    pub fingerprint: String,
    pub os: String,
}

/// Advertises a running DeskGlide Server on the local network until dropped.
pub struct MdnsAdvertiser {
    daemon: ServiceDaemon,
    fullname: String,
}

impl MdnsAdvertiser {
    pub fn register(device_name: &str, port: u16, fingerprint: &str) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("failed to start mDNS daemon")?;

        let clean_instance: String = device_name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == ' ' { c } else { '-' })
            .collect();
        let instance = if clean_instance.trim().is_empty() {
            "DeskGlide-Host".to_string()
        } else {
            clean_instance.trim().to_string()
        };

        let host_label: String = instance
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
            .collect();
        let host_name = format!("{host_label}.local.");

        let os_str = OsKind::current().to_string();
        let ver_str = PROTOCOL_VERSION.to_string();
        let props = [
            ("fp", fingerprint),
            ("os", os_str.as_str()),
            ("ver", ver_str.as_str()),
        ];

        let info = ServiceInfo::new(
            MDNS_SERVICE_TYPE,
            &instance,
            &host_name,
            "",
            port,
            &props[..],
        )?
        .enable_addr_auto();

        let fullname = info.get_fullname().to_string();
        daemon
            .register(info)
            .context("failed to register DeskGlide mDNS service")?;
        debug!(%fullname, port, "registered DeskGlide mDNS service");

        Ok(Self { daemon, fullname })
    }
}

impl Drop for MdnsAdvertiser {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Continuously browses for DeskGlide Hosts on the local network.
pub struct MdnsBrowser {
    daemon: ServiceDaemon,
    hosts_rx: watch::Receiver<Vec<DiscoveredHost>>,
    thread: Option<JoinHandle<()>>,
}

impl MdnsBrowser {
    pub fn start() -> Result<Self> {
        let daemon = ServiceDaemon::new().context("failed to start mDNS browser daemon")?;
        let receiver = daemon
            .browse(MDNS_SERVICE_TYPE)
            .context("failed to browse DeskGlide mDNS service")?;

        let (tx, rx) = watch::channel(Vec::new());
        let thread = thread::Builder::new()
            .name("deskglide-mdns-browser".into())
            .spawn(move || {
                let mut map: HashMap<String, DiscoveredHost> = HashMap::new();
                while let Ok(event) = receiver.recv() {
                    let mut changed = false;
                    match event {
                        ServiceEvent::ServiceResolved(resolved) => {
                            let port = resolved.get_port();
                            let v4s = resolved.get_addresses_v4();
                            let ip: Option<IpAddr> = v4s
                                .iter()
                                .find(|a| !a.is_loopback())
                                .or_else(|| v4s.iter().next())
                                .copied()
                                .map(IpAddr::V4)
                                .or_else(|| {
                                    resolved
                                        .get_addresses()
                                        .iter()
                                        .next()
                                        .map(|s| s.to_ip_addr())
                                });

                            if let Some(ip) = ip {
                                let fullname = resolved.get_fullname().to_string();
                                let display_name = fullname
                                    .strip_suffix(&format!(".{MDNS_SERVICE_TYPE}"))
                                    .unwrap_or(&fullname)
                                    .to_string();
                                let fingerprint = resolved
                                    .get_property_val_str("fp")
                                    .unwrap_or("")
                                    .to_string();
                                let os = resolved
                                    .get_property_val_str("os")
                                    .unwrap_or("Unknown")
                                    .to_string();

                                if !fingerprint.is_empty() {
                                    map.insert(
                                        fullname.clone(),
                                        DiscoveredHost {
                                            fullname,
                                            name: display_name,
                                            addr: SocketAddr::new(ip, port),
                                            fingerprint,
                                            os,
                                        },
                                    );
                                    changed = true;
                                }
                            }
                        }
                        ServiceEvent::ServiceRemoved(_, fullname)
                            if map.remove(&fullname).is_some() =>
                        {
                            changed = true;
                        }
                        _ => {}
                    }

                    if changed {
                        let mut list: Vec<DiscoveredHost> = map.values().cloned().collect();
                        list.sort_by(|a, b| a.name.cmp(&b.name).then(a.addr.cmp(&b.addr)));
                        if tx.send(list).is_err() {
                            break;
                        }
                    }
                }
            })
            .context("failed to spawn mDNS browser thread")?;

        Ok(Self {
            daemon,
            hosts_rx: rx,
            thread: Some(thread),
        })
    }

    pub fn snapshot(&self) -> Vec<DiscoveredHost> {
        self.hosts_rx.borrow().clone()
    }

    pub fn watch(&self) -> watch::Receiver<Vec<DiscoveredHost>> {
        self.hosts_rx.clone()
    }
}

impl Drop for MdnsBrowser {
    fn drop(&mut self) {
        if let Err(e) = self.daemon.stop_browse(MDNS_SERVICE_TYPE) {
            warn!("failed stopping mDNS browse: {e}");
        }
        let _ = self.daemon.shutdown();
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
