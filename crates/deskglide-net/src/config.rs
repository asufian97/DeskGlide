//! Persistent TOML configuration and pair-once host credentials.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use deskglide_proto::{OsKind, ScreenEdge, DEFAULT_PORT};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::auth::generate_pairing_secret;
use crate::tls::config_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AppMode {
    #[default]
    Host,
    Client,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedHost {
    pub name: String,
    pub addr: String,
    pub fingerprint: String,
    pub secret: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub mode: AppMode,
    pub device_name: String,
    pub port: u16,
    pub pairing_secret: String,
    pub hotkey: String,
    pub switch_edge: String,
    pub coalesce_ms: u64,
    pub clipboard_sync: bool,
    pub auto_connect: bool,
    pub start_minimized: bool,
    pub paired_hosts: Vec<PairedHost>,
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{}-PC", OsKind::current()))
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            mode: AppMode::Host,
            device_name: default_device_name(),
            port: DEFAULT_PORT,
            pairing_secret: generate_pairing_secret(),
            hotkey: "Ctrl+Alt+S".to_string(),
            switch_edge: "right".to_string(),
            coalesce_ms: 5,
            clipboard_sync: true,
            auto_connect: true,
            start_minimized: false,
            paired_hosts: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn default_path() -> PathBuf {
        config_dir().join("config.toml")
    }

    pub fn load(path_override: Option<&Path>) -> Self {
        let path = path_override
            .map(Path::to_path_buf)
            .unwrap_or_else(Self::default_path);
        match fs::read_to_string(&path) {
            Ok(raw) => match toml::from_str::<AppConfig>(&raw) {
                Ok(mut cfg) => {
                    if cfg.pairing_secret.trim().is_empty() {
                        cfg.pairing_secret = generate_pairing_secret();
                    }
                    if cfg.device_name.trim().is_empty() {
                        cfg.device_name = default_device_name();
                    }
                    cfg.coalesce_ms = cfg.coalesce_ms.clamp(1, 50);
                    cfg
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "invalid config.toml; using defaults");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path_override: Option<&Path>) -> Result<()> {
        let path = path_override
            .map(Path::to_path_buf)
            .unwrap_or_else(Self::default_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create config dir {}", parent.display()))?;
        }
        let encoded = toml::to_string_pretty(self)?;
        fs::write(&path, encoded)
            .with_context(|| format!("failed to save {}", path.display()))?;
        restrict_config_perms(&path);
        Ok(())
    }

    pub fn parsed_edge(&self) -> ScreenEdge {
        ScreenEdge::from_name(&self.switch_edge).unwrap_or(ScreenEdge::Right)
    }

    pub fn set_edge(&mut self, edge: ScreenEdge) {
        self.switch_edge = edge.as_str().to_string();
    }

    pub fn find_paired(&self, fingerprint: &str) -> Option<&PairedHost> {
        self.paired_hosts
            .iter()
            .find(|h| h.fingerprint.eq_ignore_ascii_case(fingerprint))
    }

    pub fn upsert_paired_host(&mut self, host: PairedHost) {
        if let Some(existing) = self
            .paired_hosts
            .iter_mut()
            .find(|h| h.fingerprint.eq_ignore_ascii_case(&host.fingerprint))
        {
            *existing = host;
        } else {
            self.paired_hosts.push(host);
        }
    }

    pub fn remove_paired_host(&mut self, fingerprint: &str) {
        self.paired_hosts
            .retain(|h| !h.fingerprint.eq_ignore_ascii_case(fingerprint));
    }
}

#[cfg(unix)]
fn restrict_config_perms(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_config_perms(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn config_roundtrip_and_upsert() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("deskglide-cfg-{}-{nanos}.toml", std::process::id()));

        let mut cfg = AppConfig {
            mode: AppMode::Client,
            ..Default::default()
        };
        cfg.set_edge(ScreenEdge::Left);
        cfg.upsert_paired_host(PairedHost {
            name: "WinPC".into(),
            addr: "192.168.1.10:24910".into(),
            fingerprint: "AA:BB".into(),
            secret: "K7M2-9X4P".into(),
        });
        cfg.upsert_paired_host(PairedHost {
            name: "WinPC-Updated".into(),
            addr: "192.168.1.11:24910".into(),
            fingerprint: "aa:bb".into(),
            secret: "NEW-CODE".into(),
        });
        assert_eq!(cfg.paired_hosts.len(), 1);
        cfg.save(Some(&path)).unwrap();

        let loaded = AppConfig::load(Some(&path));
        assert_eq!(loaded, cfg);
        assert_eq!(loaded.parsed_edge(), ScreenEdge::Left);

        let _ = fs::remove_file(&path);
    }
}
