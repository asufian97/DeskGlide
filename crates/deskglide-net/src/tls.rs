//! Persistent self-signed TLS 1.3 certificates, SHA-256 pinning, and TOFU known_hosts.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use deskglide_proto::ALPN;
use directories::ProjectDirs;
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use rcgen::{CertificateParams, KeyPair};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

/// Persistent server TLS certificate and private key.
#[derive(Debug, Clone)]
pub struct ServerIdentity {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub fingerprint: [u8; 32],
    pub config_dir: PathBuf,
}

impl ServerIdentity {
    pub fn fingerprint_str(&self) -> String {
        format_fingerprint(&self.fingerprint)
    }
}

/// How the client validates the server's self-signed TLS certificate.
#[derive(Debug, Clone)]
pub enum PinPolicy {
    /// Require this exact SHA-256 fingerprint.
    Exact([u8; 32]),
    /// Look up `peer_key` in `known_hosts_path`; if absent and `allow_tofu` is true, pin it.
    KnownOrTofu {
        known_hosts_path: PathBuf,
        peer_key: String,
        allow_tofu: bool,
    },
}

/// Default per-user configuration directory (`%APPDATA%\deskglide` or `~/.config/deskglide`).
pub fn config_dir() -> PathBuf {
    if let Some(proj) = ProjectDirs::from("org", "deskglide", "deskglide") {
        proj.config_dir().to_path_buf()
    } else {
        PathBuf::from(".deskglide")
    }
}

/// Load the persistent self-signed server certificate, or generate one on first run.
pub fn load_or_create_identity(dir_override: Option<&Path>) -> Result<ServerIdentity> {
    let dir = dir_override.map(Path::to_path_buf).unwrap_or_else(config_dir);
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create config dir {}", dir.display()))?;

    let cert_path = dir.join("cert.der");
    let key_path = dir.join("key.der");

    let (cert_der, key_der) = if cert_path.exists() && key_path.exists() {
        let cert_der = fs::read(&cert_path)
            .with_context(|| format!("failed to read {}", cert_path.display()))?;
        let key_der = fs::read(&key_path)
            .with_context(|| format!("failed to read {}", key_path.display()))?;
        (cert_der, key_der)
    } else {
        let params = CertificateParams::new(vec!["deskglide".to_string()])?;
        let key_pair = KeyPair::generate()?;
        let cert = params.self_signed(&key_pair)?;
        let cert_der = cert.der().to_vec();
        let key_der = key_pair.serialize_der();

        fs::write(&cert_path, &cert_der)
            .with_context(|| format!("failed to write {}", cert_path.display()))?;
        fs::write(&key_path, &key_der)
            .with_context(|| format!("failed to write {}", key_path.display()))?;
        restrict_private_key(&key_path);
        (cert_der, key_der)
    };

    let fingerprint = cert_fingerprint(&cert_der);
    Ok(ServerIdentity {
        cert_der,
        key_der,
        fingerprint,
        config_dir: dir,
    })
}

#[cfg(unix)]
fn restrict_private_key(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_private_key(_path: &Path) {}

/// Compute SHA-256 digest of a DER-encoded X.509 certificate.
pub fn cert_fingerprint(der: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(der);
    h.finalize().into()
}

/// Format a SHA-256 fingerprint as colon-separated uppercase hex (`AA:BB:...`).
pub fn format_fingerprint(fp: &[u8; 32]) -> String {
    fp.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Parse a SHA-256 fingerprint (with or without colons, dashes, spaces, or `sha256:` prefix).
pub fn parse_fingerprint(input: &str) -> Result<[u8; 32]> {
    let trimmed = input
        .trim()
        .strip_prefix("sha256:")
        .or_else(|| input.trim().strip_prefix("SHA256:"))
        .unwrap_or(input.trim());

    let hex: String = trimmed
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | ' '))
        .collect();
    if hex.len() != 64 {
        bail!(
            "expected 64 hex chars (32-byte SHA-256) in fingerprint, got {}",
            hex.len()
        );
    }

    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)
            .with_context(|| format!("invalid hex in fingerprint at byte {i}"))?;
    }
    Ok(out)
}

fn load_known_hosts(path: &Path) -> Result<HashMap<String, [u8; 32]>> {
    let mut map = HashMap::new();
    if !path.exists() {
        return Ok(map);
    }
    let content = fs::read_to_string(path)?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((peer, fp_str)) = line.split_once(char::is_whitespace) {
            if let Ok(fp) = parse_fingerprint(fp_str) {
                map.insert(peer.to_string(), fp);
            }
        }
    }
    Ok(map)
}

fn save_known_host(path: &Path, peer: &str, fp: &[u8; 32]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut map = load_known_hosts(path)?;
    map.insert(peer.to_string(), *fp);
    let mut entries: Vec<(String, [u8; 32])> = map.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::from("# DeskGlide pinned server fingerprints\n");
    for (k, v) in entries {
        out.push_str(&format!("{k} {}\n", format_fingerprint(&v)));
    }
    fs::write(path, out)?;
    Ok(())
}

#[derive(Debug)]
struct PinnedCertVerifier {
    policy: PinPolicy,
    algs: WebPkiSupportedAlgorithms,
    io_lock: Mutex<()>,
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let actual = cert_fingerprint(end_entity.as_ref());
        match &self.policy {
            PinPolicy::Exact(expected) => {
                if &actual == expected {
                    Ok(ServerCertVerified::assertion())
                } else {
                    Err(rustls::Error::General(format!(
                        "server certificate fingerprint mismatch: expected {}, got {}",
                        format_fingerprint(expected),
                        format_fingerprint(&actual)
                    )))
                }
            }
            PinPolicy::KnownOrTofu {
                known_hosts_path,
                peer_key,
                allow_tofu,
            } => {
                let _guard = self.io_lock.lock().unwrap();
                let map = load_known_hosts(known_hosts_path)
                    .map_err(|e| rustls::Error::General(format!("failed reading known_hosts: {e}")))?;

                if let Some(pinned) = map.get(peer_key) {
                    if &actual == pinned {
                        Ok(ServerCertVerified::assertion())
                    } else {
                        Err(rustls::Error::General(format!(
                            "SERVER CERTIFICATE CHANGED for {peer_key}! Pinned {}, got {}. Refusing connection (possible MITM or re-generated server cert in {}).",
                            format_fingerprint(pinned),
                            format_fingerprint(&actual),
                            known_hosts_path.display()
                        )))
                    }
                } else if *allow_tofu {
                    save_known_host(known_hosts_path, peer_key, &actual)
                        .map_err(|e| rustls::Error::General(format!("failed saving known_hosts: {e}")))?;
                    tracing::info!(
                        peer = %peer_key,
                        fingerprint = %format_fingerprint(&actual),
                        "pinned server certificate on first use (TOFU)"
                    );
                    Ok(ServerCertVerified::assertion())
                } else {
                    Err(rustls::Error::General(format!(
                        "unknown server certificate {} for {peer_key}; pass --fingerprint <FP> or --tofu on first connection",
                        format_fingerprint(&actual)
                    )))
                }
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algs)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algs)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algs.supported_schemes()
    }
}

fn shared_transport_config() -> Arc<quinn::TransportConfig> {
    let mut cfg = quinn::TransportConfig::default();
    cfg.keep_alive_interval(Some(Duration::from_secs(1)));
    cfg.max_idle_timeout(Some(Duration::from_secs(4).try_into().expect("valid idle timeout")));
    cfg.datagram_receive_buffer_size(Some(65536));
    Arc::new(cfg)
}

pub fn build_server_config(id: &ServerIdentity) -> Result<quinn::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cert = CertificateDer::from(id.cert_der.clone());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key_der.clone()));

    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    tls.alpn_protocols = vec![ALPN.to_vec()];

    let quic_crypto = QuicServerConfig::try_from(tls)?;
    let mut server_cfg = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));
    server_cfg.transport_config(shared_transport_config());
    Ok(server_cfg)
}

pub fn build_client_config(policy: PinPolicy) -> Result<quinn::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let algs = provider.signature_verification_algorithms;
    let verifier = PinnedCertVerifier {
        policy,
        algs,
        io_lock: Mutex::new(()),
    };

    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];

    let quic_crypto = QuicClientConfig::try_from(tls)?;
    let mut client_cfg = quinn::ClientConfig::new(Arc::new(quic_crypto));
    client_cfg.transport_config(shared_transport_config());
    Ok(client_cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_roundtrip() {
        let bytes = [0xABu8; 32];
        let s = format_fingerprint(&bytes);
        assert_eq!(parse_fingerprint(&s).unwrap(), bytes);
        assert_eq!(parse_fingerprint(&s.replace(':', "")).unwrap(), bytes);
        assert_eq!(parse_fingerprint(&format!("sha256:{}", s.to_lowercase())).unwrap(), bytes);
        assert!(parse_fingerprint("deadbeef").is_err());
    }
}
