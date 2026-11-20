//! High-level QUIC KVM server, client, and cancel-safe session stream/datagram reader.

use std::net::SocketAddr;

use anyhow::{bail, Context, Result};
use bytes::{Buf, BytesMut};
use deskglide_proto::{InputEvent, Message, MAX_FRAME_LEN};
use quinn::{Connection, Endpoint, RecvStream, SendStream};

use crate::auth::{client_authenticate, server_authenticate, AuthPeerInfo};
use crate::tls::{build_client_config, build_server_config, PinPolicy, ServerIdentity};

/// Write one length-prefixed frame on the reliable QUIC stream.
pub(crate) async fn write_frame(send: &mut SendStream, msg: &Message) -> Result<()> {
    let frame = msg.encode_frame();
    send.write_all(&frame).await?;
    Ok(())
}

/// Read one length-prefixed frame (only used sequentially during auth before `select!`).
pub(crate) async fn read_single_frame(recv: &mut RecvStream) -> Result<Message> {
    let mut len_buf = [0u8; 2];
    recv.read_exact(&mut len_buf)
        .await
        .context("failed reading frame length")?;
    let len = u16::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME_LEN {
        bail!("invalid frame length {len}");
    }
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body)
        .await
        .context("failed reading frame body")?;
    Ok(Message::decode(&body)?)
}

/// Authenticated QUIC KVM session exposing both reliable stream and unreliable datagrams.
pub struct KvmSession {
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
    rx_buf: BytesMut,
    peer_info: Option<AuthPeerInfo>,
}

impl KvmSession {
    fn new(
        conn: Connection,
        send: SendStream,
        recv: RecvStream,
        peer_info: Option<AuthPeerInfo>,
    ) -> Self {
        Self {
            conn,
            send,
            recv,
            rx_buf: BytesMut::with_capacity(4096),
            peer_info,
        }
    }

    pub fn remote_addr(&self) -> SocketAddr {
        self.conn.remote_address()
    }

    pub fn peer_name(&self) -> Option<&str> {
        self.peer_info.as_ref().map(|p| p.name.as_str())
    }

    pub fn peer_os(&self) -> Option<deskglide_proto::OsKind> {
        self.peer_info.as_ref().map(|p| p.os)
    }

    /// Send an ordered reliable control or input message.
    pub async fn send_reliable(&mut self, msg: &Message) -> Result<()> {
        write_frame(&mut self.send, msg).await
    }

    /// Send a relative mouse delta over an unreliable QUIC datagram (zero head-of-line blocking).
    /// Falls back to reliable stream if the datagram exceeds peer limits or is unsupported.
    pub async fn send_mouse_datagram(&mut self, dx: i32, dy: i32) -> Result<()> {
        let msg = Message::Input(InputEvent::MouseMove { dx, dy });
        let bytes = bytes::Bytes::from(msg.encode());
        if self.conn.send_datagram(bytes).is_err() {
            self.send_reliable(&msg).await?;
        }
        Ok(())
    }

    fn try_pop_buffered_frame(&mut self) -> Result<Option<Message>> {
        if self.rx_buf.len() < 2 {
            return Ok(None);
        }
        let len = u16::from_be_bytes([self.rx_buf[0], self.rx_buf[1]]) as usize;
        if len == 0 || len > MAX_FRAME_LEN {
            bail!("invalid frame length {len}");
        }
        if self.rx_buf.len() < 2 + len {
            return Ok(None);
        }
        self.rx_buf.advance(2);
        let body = self.rx_buf.split_to(len);
        Ok(Some(Message::decode(&body)?))
    }

    /// Receive the next message from either the reliable stream or QUIC datagrams.
    /// Returns `Ok(None)` on clean remote shutdown.
    /// Completely cancel-safe across `select!`.
    pub async fn recv(&mut self) -> Result<Option<Message>> {
        loop {
            if let Some(msg) = self.try_pop_buffered_frame()? {
                return Ok(Some(msg));
            }

            let mut chunk = [0u8; 2048];
            tokio::select! {
                stream_res = self.recv.read(&mut chunk) => {
                    match stream_res {
                        Ok(Some(n)) => {
                            self.rx_buf.extend_from_slice(&chunk[..n]);
                        }
                        Ok(None) => return Ok(None),
                        Err(e) => return Err(e.into()),
                    }
                }
                dgram_res = self.conn.read_datagram() => {
                    match dgram_res {
                        Ok(bytes) => {
                            return Ok(Some(Message::decode(&bytes)?));
                        }
                        Err(quinn::ConnectionError::ApplicationClosed(_))
                        | Err(quinn::ConnectionError::LocallyClosed) => {
                            return Ok(None);
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
            }
        }
    }

    pub async fn close(&mut self, reason: &[u8]) {
        let _ = self.send.finish();
        self.conn.close(0u32.into(), reason);
    }
}

/// QUIC KVM Server listener.
pub struct KvmServer {
    endpoint: Endpoint,
}

impl KvmServer {
    pub fn bind(addr: SocketAddr, identity: &ServerIdentity) -> Result<Self> {
        let cfg = build_server_config(identity)?;
        let endpoint = Endpoint::server(cfg, addr)
            .with_context(|| format!("failed to bind QUIC UDP socket on {addr}"))?;
        Ok(Self { endpoint })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    /// Accept the next incoming QUIC connection and verify channel-bound pairing secret.
    pub async fn accept_authenticated(&self, secret: &str) -> Result<KvmSession> {
        let incoming = self
            .endpoint
            .accept()
            .await
            .ok_or_else(|| anyhow::anyhow!("QUIC server endpoint closed"))?;

        let conn = incoming
            .await
            .context("QUIC/TLS handshake with client failed")?;

        let (mut send, mut recv) = conn
            .accept_bi()
            .await
            .context("client failed to open control stream")?;

        let peer = match server_authenticate(&conn, &mut send, &mut recv, secret).await {
            Ok(p) => p,
            Err(e) => {
                conn.close(1u32.into(), b"auth failed");
                return Err(e);
            }
        };

        Ok(KvmSession::new(conn, send, recv, Some(peer)))
    }

    pub fn close(&self) {
        self.endpoint.close(0u32.into(), b"server shutdown");
    }
}

/// QUIC KVM Client connector.
pub struct KvmClient;

impl KvmClient {
    pub async fn connect(
        server_addr: SocketAddr,
        secret: &str,
        policy: PinPolicy,
        client_name: &str,
    ) -> Result<KvmSession> {
        let bind_addr: SocketAddr = if server_addr.is_ipv4() {
            "0.0.0.0:0".parse().unwrap()
        } else {
            "[::]:0".parse().unwrap()
        };

        let mut endpoint = Endpoint::client(bind_addr)
            .context("failed to bind client UDP socket")?;
        endpoint.set_default_client_config(build_client_config(policy)?);

        let conn = endpoint
            .connect(server_addr, "deskglide")?
            .await
            .with_context(|| format!("failed to connect to QUIC server at {server_addr}"))?;

        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .context("failed to open QUIC bidirectional stream")?;

        if let Err(e) = client_authenticate(&conn, &mut send, &mut recv, secret, client_name).await
        {
            conn.close(1u32.into(), b"auth failed");
            return Err(e);
        }

        Ok(KvmSession::new(conn, send, recv, None))
    }
}
