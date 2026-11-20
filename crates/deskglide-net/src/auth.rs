//! RFC 5705 TLS-exporter channel-bound HMAC-SHA256 mutual authentication.

use anyhow::{bail, Result};
use deskglide_proto::{Message, OsKind, PROTOCOL_VERSION};
use hmac::{Hmac, Mac};
use quinn::{Connection, RecvStream, SendStream};
use rand::{Rng, RngCore};
use sha2::Sha256;

use crate::session::{read_single_frame, write_frame};

const EKM_LABEL: &[u8] = b"deskglide-auth-v1";
const CLIENT_ROLE: &[u8] = b"client";
const SERVER_ROLE: &[u8] = b"server";
const SECRET_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

pub struct AuthPeerInfo {
    pub name: String,
    pub os: OsKind,
}

/// Generate a human-friendly 8-character pairing code (e.g. `K7M2-9X4P`).
pub fn generate_pairing_secret() -> String {
    let mut rng = rand::rng();
    let mut s = String::with_capacity(9);
    for i in 0..8 {
        if i == 4 {
            s.push('-');
        }
        let idx = rng.random_range(0..SECRET_ALPHABET.len());
        s.push(SECRET_ALPHABET[idx] as char);
    }
    s
}

fn export_binding(conn: &Connection) -> Result<[u8; 32]> {
    let mut ekm = [0u8; 32];
    conn.export_keying_material(&mut ekm, EKM_LABEL, &[])
        .map_err(|_| anyhow::anyhow!("TLS keying material export failed"))?;
    Ok(ekm)
}

fn compute_proof(
    secret: &str,
    ekm: &[u8; 32],
    role: &[u8],
    client_nonce: &[u8; 32],
    server_nonce: &[u8; 32],
) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC allows any key");
    mac.update(ekm);
    mac.update(role);
    mac.update(client_nonce);
    mac.update(server_nonce);
    mac.finalize().into_bytes().into()
}

fn verify_proof(
    secret: &str,
    ekm: &[u8; 32],
    role: &[u8],
    client_nonce: &[u8; 32],
    server_nonce: &[u8; 32],
    proof: &[u8; 32],
) -> bool {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC allows any key");
    mac.update(ekm);
    mac.update(role);
    mac.update(client_nonce);
    mac.update(server_nonce);
    mac.verify_slice(proof).is_ok()
}

pub async fn server_authenticate(
    conn: &Connection,
    send: &mut SendStream,
    recv: &mut RecvStream,
    secret: &str,
) -> Result<AuthPeerInfo> {
    let (version, os, client_nonce, name) = match read_single_frame(recv).await? {
        Message::Hello {
            version,
            os,
            nonce,
            name,
        } => (version, os, nonce, name),
        other => bail!("expected Hello message, got {other:?}"),
    };

    if version != PROTOCOL_VERSION {
        let _ = write_frame(
            send,
            &Message::AuthResult {
                accepted: false,
                proof: [0; 32],
                reason: format!("protocol mismatch: server v{PROTOCOL_VERSION}, client v{version}"),
            },
        )
        .await;
        bail!("client protocol version {version} != {PROTOCOL_VERSION}");
    }

    let mut server_nonce = [0u8; 32];
    rand::rng().fill_bytes(&mut server_nonce);
    write_frame(send, &Message::AuthChallenge { nonce: server_nonce }).await?;

    let client_proof = match read_single_frame(recv).await? {
        Message::AuthResponse { proof } => proof,
        other => bail!("expected AuthResponse, got {other:?}"),
    };

    let ekm = export_binding(conn)?;
    if !verify_proof(secret, &ekm, CLIENT_ROLE, &client_nonce, &server_nonce, &client_proof) {
        let _ = write_frame(
            send,
            &Message::AuthResult {
                accepted: false,
                proof: [0; 32],
                reason: "invalid pairing secret".into(),
            },
        )
        .await;
        bail!("client failed pairing secret authentication");
    }

    let server_proof = compute_proof(secret, &ekm, SERVER_ROLE, &client_nonce, &server_nonce);
    write_frame(
        send,
        &Message::AuthResult {
            accepted: true,
            proof: server_proof,
            reason: String::new(),
        },
    )
    .await?;

    Ok(AuthPeerInfo { name, os })
}

pub async fn client_authenticate(
    conn: &Connection,
    send: &mut SendStream,
    recv: &mut RecvStream,
    secret: &str,
    client_name: &str,
) -> Result<()> {
    let mut client_nonce = [0u8; 32];
    rand::rng().fill_bytes(&mut client_nonce);

    write_frame(
        send,
        &Message::Hello {
            version: PROTOCOL_VERSION,
            os: OsKind::current(),
            nonce: client_nonce,
            name: client_name.to_string(),
        },
    )
    .await?;

    let server_nonce = match read_single_frame(recv).await? {
        Message::AuthChallenge { nonce } => nonce,
        Message::AuthResult {
            accepted: false,
            reason,
            ..
        } => bail!("server rejected handshake: {reason}"),
        other => bail!("expected AuthChallenge, got {other:?}"),
    };

    let ekm = export_binding(conn)?;
    let client_proof = compute_proof(secret, &ekm, CLIENT_ROLE, &client_nonce, &server_nonce);
    write_frame(send, &Message::AuthResponse { proof: client_proof }).await?;

    match read_single_frame(recv).await? {
        Message::AuthResult {
            accepted: true,
            proof,
            ..
        } => {
            if !verify_proof(secret, &ekm, SERVER_ROLE, &client_nonce, &server_nonce, &proof) {
                bail!("server failed mutual authentication proof");
            }
            Ok(())
        }
        Message::AuthResult {
            accepted: false,
            reason,
            ..
        } => bail!("authentication rejected by server: {reason}"),
        other => bail!("expected AuthResult, got {other:?}"),
    }
}
