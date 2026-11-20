//! Host (Server) async event loop.

use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use deskglide_input::{open_capture, CaptureEvent, Hotkey, InputCapture, MouseCoalescer};
use deskglide_net::{KvmServer, KvmSession, MdnsAdvertiser, ServerIdentity};
use deskglide_proto::{InputEvent, Message, ScreenEdge};
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, MissedTickBehavior};
use tracing::{info, warn};

use crate::clipboard::ClipboardSync;
use crate::status::{detect_lan_ip, EngineStatus, HostCommand};

#[derive(Debug, Clone)]
pub struct HostOptions {
    pub bind_addr: SocketAddr,
    pub identity: ServerIdentity,
    pub device_name: String,
    pub secret: String,
    pub hotkey: Hotkey,
    pub switch_edge: ScreenEdge,
    pub coalesce_ms: u64,
    pub clipboard_sync: bool,
    pub enable_mdns: bool,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub async fn run_host(
    opts: HostOptions,
    status_tx: watch::Sender<EngineStatus>,
    mut cmd_rx: mpsc::Receiver<HostCommand>,
) -> Result<()> {
    let server = KvmServer::bind(opts.bind_addr, &opts.identity)?;
    let actual_bind = server.local_addr().unwrap_or(opts.bind_addr);
    let mut current_edge = opts.switch_edge;
    let mut clipboard_enabled = opts.clipboard_sync;

    let mut capture = open_capture(opts.hotkey, current_edge)?;

    let _mdns = if opts.enable_mdns {
        match MdnsAdvertiser::register(
            &opts.device_name,
            actual_bind.port(),
            &opts.identity.fingerprint_str(),
        ) {
            Ok(adv) => Some(adv),
            Err(e) => {
                warn!("mDNS advertisement unavailable: {e:#}");
                None
            }
        }
    } else {
        None
    };

    let lan_ip = detect_lan_ip();
    let listening_status = EngineStatus::Listening {
        bind_addr: actual_bind,
        lan_ip,
        secret: opts.secret.clone(),
        fingerprint: opts.identity.fingerprint_str(),
    };

    loop {
        let _ = status_tx.send(listening_status.clone());
        info!("Waiting for encrypted client connection on {actual_bind}...");

        let session = loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(HostCommand::SetSwitchEdge(edge)) => {
                            current_edge = edge;
                            capture.set_switch_edge(edge);
                        }
                        Some(HostCommand::SetClipboardSync(enabled)) => {
                            clipboard_enabled = enabled;
                        }
                        Some(HostCommand::ToggleControl) => {
                            warn!("No client connected yet; staying in LOCAL mode");
                        }
                        Some(HostCommand::Stop) | None => {
                            let _ = capture.set_remote_active(false);
                            server.close();
                            let _ = status_tx.send(EngineStatus::Idle);
                            return Ok(());
                        }
                    }
                }
                ev = capture.events_rx().recv() => {
                    if let Some(CaptureEvent::ActiveChanged(true)) = ev {
                        warn!("Switch triggered while no client is connected; staying in LOCAL mode");
                        let _ = capture.set_remote_active(false);
                    }
                }
                res = server.accept_authenticated(&opts.secret) => {
                    match res {
                        Ok(s) => break s,
                        Err(e) => {
                            warn!("Rejected client connection: {e:#}");
                        }
                    }
                }
            }
        };

        let peer_name = session.peer_name().unwrap_or("Client").to_string();
        let peer_os = session
            .peer_os()
            .map(|o| o.to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        let peer_addr = session.remote_addr();
        info!("Client connected: {peer_name} ({peer_os}) @ {peer_addr}");

        let stop = run_active_session(
            session,
            capture.as_mut(),
            &opts,
            &mut current_edge,
            &mut clipboard_enabled,
            &peer_name,
            &peer_os,
            peer_addr,
            &status_tx,
            &mut cmd_rx,
        )
        .await;

        let _ = capture.set_remote_active(false);

        if stop {
            server.close();
            let _ = status_tx.send(EngineStatus::Idle);
            return Ok(());
        }
        info!("Client disconnected ({peer_name}); returned to LOCAL control");
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_active_session(
    mut session: KvmSession,
    capture: &mut dyn InputCapture,
    opts: &HostOptions,
    current_edge: &mut ScreenEdge,
    clipboard_enabled: &mut bool,
    peer_name: &str,
    peer_os: &str,
    peer_addr: SocketAddr,
    status_tx: &watch::Sender<EngineStatus>,
    cmd_rx: &mut mpsc::Receiver<HostCommand>,
) -> bool {
    let mut controlling_remote = false;
    let mut last_rtt_ms: Option<u64> = None;
    let update_status = |remote: bool, rtt: Option<u64>| {
        let _ = status_tx.send(EngineStatus::Connected {
            peer_name: peer_name.to_string(),
            peer_os: peer_os.to_string(),
            peer_addr,
            controlling_remote: remote,
            rtt_ms: rtt,
        });
    };
    update_status(false, None);

    let mut coalescer = MouseCoalescer::new();
    let mut coalesce_tick = interval(Duration::from_millis(opts.coalesce_ms.clamp(1, 50)));
    coalesce_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut heartbeat_tick = interval(Duration::from_secs(1));
    heartbeat_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut clipboard_tick = interval(Duration::from_millis(400));
    clipboard_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut clipboard = ClipboardSync::new();

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(HostCommand::SetSwitchEdge(edge)) => {
                        *current_edge = edge;
                        capture.set_switch_edge(edge);
                    }
                    Some(HostCommand::SetClipboardSync(enabled)) => {
                        *clipboard_enabled = enabled;
                    }
                    Some(HostCommand::ToggleControl) => {
                        let next = !capture.is_remote_active();
                        let _ = capture.set_remote_active(next);
                        controlling_remote = next;
                        update_status(controlling_remote, last_rtt_ms);
                        if !next {
                            let _ = session.send_reliable(&Message::ReleaseAllKeys).await;
                        }
                        let entry_edge = if next { current_edge.opposite() } else { ScreenEdge::None };
                        if session.send_reliable(&Message::Control { active: next, entry_edge }).await.is_err() {
                            return false;
                        }
                    }
                    Some(HostCommand::Stop) | None => {
                        let _ = session.send_reliable(&Message::ReleaseAllKeys).await;
                        let _ = session.send_reliable(&Message::Control {
                            active: false,
                            entry_edge: ScreenEdge::None,
                        }).await;
                        session.close(b"server shutdown").await;
                        return true;
                    }
                }
            }
            maybe_ev = capture.events_rx().recv() => {
                let Some(ev) = maybe_ev else {
                    warn!("input capture channel closed unexpectedly");
                    return true;
                };
                match ev {
                    CaptureEvent::ActiveChanged(active) => {
                        controlling_remote = active;
                        update_status(controlling_remote, last_rtt_ms);
                        if let Some((dx, dy)) = coalescer.take() {
                            if session
                                .send_reliable(&Message::Input(InputEvent::MouseMove { dx, dy }))
                                .await
                                .is_err()
                            {
                                return false;
                            }
                        }
                        if !active && session.send_reliable(&Message::ReleaseAllKeys).await.is_err() {
                            return false;
                        }
                        let entry_edge = if active { current_edge.opposite() } else { ScreenEdge::None };
                        if session.send_reliable(&Message::Control { active, entry_edge }).await.is_err() {
                            return false;
                        }
                        if active {
                            info!(">>> Switched control to REMOTE client");
                        } else {
                            info!("<<< Switched control back to LOCAL machine");
                        }
                    }
                    CaptureEvent::Input(InputEvent::MouseMove { dx, dy }) => {
                        coalescer.add(dx, dy);
                    }
                    CaptureEvent::Input(ev @ (InputEvent::MouseButton { .. } | InputEvent::MouseScroll { .. })) => {
                        if let Some((dx, dy)) = coalescer.take() {
                            if session
                                .send_reliable(&Message::Input(InputEvent::MouseMove { dx, dy }))
                                .await
                                .is_err()
                            {
                                return false;
                            }
                        }
                        if session.send_reliable(&Message::Input(ev)).await.is_err() {
                            return false;
                        }
                    }
                    CaptureEvent::Input(ev @ InputEvent::Key { .. }) => {
                        if session.send_reliable(&Message::Input(ev)).await.is_err() {
                            return false;
                        }
                    }
                }
            }
            _ = coalesce_tick.tick() => {
                if let Some((dx, dy)) = coalescer.take() {
                    if session.send_mouse_datagram(dx, dy).await.is_err() {
                        return false;
                    }
                }
            }
            _ = clipboard_tick.tick() => {
                if *clipboard_enabled {
                    if let Some(text) = clipboard.poll_local_change() {
                        if session.send_reliable(&Message::ClipboardText(text)).await.is_err() {
                            return false;
                        }
                    }
                }
            }
            _ = heartbeat_tick.tick() => {
                if session.send_reliable(&Message::Heartbeat { timestamp_ms: now_ms() }).await.is_err() {
                    return false;
                }
            }
            net_msg = session.recv() => {
                match net_msg {
                    Ok(Some(Message::EdgeReturn)) => {
                        coalescer.clear();
                        controlling_remote = false;
                        update_status(false, last_rtt_ms);
                        let _ = capture.restore_from_edge(*current_edge);
                        if session.send_reliable(&Message::ReleaseAllKeys).await.is_err() {
                            return false;
                        }
                        if session.send_reliable(&Message::Control {
                            active: false,
                            entry_edge: ScreenEdge::None,
                        }).await.is_err() {
                            return false;
                        }
                        info!("<<< Pointer returned from client edge to LOCAL machine");
                    }
                    Ok(Some(Message::ClipboardText(text))) => {
                        if *clipboard_enabled {
                            clipboard.apply_remote(&text);
                        }
                    }
                    Ok(Some(Message::Heartbeat { timestamp_ms })) => {
                        let rtt = now_ms().saturating_sub(timestamp_ms).min(9999);
                        last_rtt_ms = Some(rtt);
                        update_status(controlling_remote, last_rtt_ms);
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => return false,
                    Err(e) => {
                        warn!("session error: {e:#}");
                        return false;
                    }
                }
            }
        }
    }
}
