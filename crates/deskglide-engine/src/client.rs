//! Client async event loop with auto-reconnect, edge return, and clipboard sync.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Result;
use deskglide_input::{open_injector, ClientEdgeTracker, HeldStateTracker, InputInjector};
use deskglide_net::{KvmClient, KvmSession, PinPolicy};
use deskglide_proto::{InputEvent, Message};
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, sleep, MissedTickBehavior};
use tracing::{info, warn};

use crate::clipboard::ClipboardSync;
use crate::status::{ClientCommand, EngineStatus};

#[derive(Debug, Clone)]
pub struct ClientOptions {
    pub server_addr: SocketAddr,
    pub secret: String,
    pub pin_policy: PinPolicy,
    pub client_name: String,
    pub clipboard_sync: bool,
    pub auto_reconnect: bool,
}

pub async fn run_client(
    opts: ClientOptions,
    status_tx: watch::Sender<EngineStatus>,
    mut cmd_rx: mpsc::Receiver<ClientCommand>,
) -> Result<()> {
    let mut injector = open_injector()?;
    let mut clipboard_enabled = opts.clipboard_sync;
    let mut attempt: u32 = 0;

    loop {
        attempt = attempt.saturating_add(1);
        let _ = status_tx.send(EngineStatus::Connecting {
            target: opts.server_addr,
            attempt,
        });
        info!("Connecting to DeskGlide server at {} (attempt {attempt})...", opts.server_addr);

        let connect_fut = KvmClient::connect(
            opts.server_addr,
            &opts.secret,
            opts.pin_policy.clone(),
            &opts.client_name,
        );

        let session = tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(ClientCommand::SetClipboardSync(enabled)) => {
                        clipboard_enabled = enabled;
                        continue;
                    }
                    Some(ClientCommand::Stop) | None => {
                        let _ = status_tx.send(EngineStatus::Idle);
                        return Ok(());
                    }
                }
            }
            res = connect_fut => {
                match res {
                    Ok(s) => {
                        attempt = 0;
                        s
                    }
                    Err(e) => {
                        if !opts.auto_reconnect {
                            let _ = status_tx.send(EngineStatus::Error(format!("{e:#}")));
                            return Err(e);
                        }
                        warn!("Connection failed ({e:#}); retrying soon...");
                        let _ = status_tx.send(EngineStatus::Error(format!("{e:#}")));
                        let backoff_secs = match attempt {
                            1 => 1,
                            2 => 2,
                            3 => 3,
                            _ => 5,
                        };
                        tokio::select! {
                            cmd = cmd_rx.recv() => {
                                match cmd {
                                    Some(ClientCommand::SetClipboardSync(enabled)) => {
                                        clipboard_enabled = enabled;
                                    }
                                    Some(ClientCommand::Stop) | None => {
                                        let _ = status_tx.send(EngineStatus::Idle);
                                        return Ok(());
                                    }
                                }
                            }
                            _ = sleep(Duration::from_secs(backoff_secs)) => {}
                        }
                        continue;
                    }
                }
            }
        };

        info!("Authenticated & encrypted (TLS 1.3) with {}", opts.server_addr);
        let stop = run_client_session(
            session,
            injector.as_mut(),
            &opts,
            &mut clipboard_enabled,
            &status_tx,
            &mut cmd_rx,
        )
        .await;

        if stop || !opts.auto_reconnect {
            let _ = status_tx.send(EngineStatus::Idle);
            return Ok(());
        }
    }
}

async fn run_client_session(
    mut session: KvmSession,
    injector: &mut dyn InputInjector,
    opts: &ClientOptions,
    clipboard_enabled: &mut bool,
    status_tx: &watch::Sender<EngineStatus>,
    cmd_rx: &mut mpsc::Receiver<ClientCommand>,
) -> bool {
    let mut held = HeldStateTracker::new();
    let mut edge_tracker = ClientEdgeTracker::default();
    let mut controlling_remote = false;

    let update_status = |remote: bool| {
        let _ = status_tx.send(EngineStatus::Connected {
            peer_name: opts.server_addr.ip().to_string(),
            peer_os: "Host".to_string(),
            peer_addr: opts.server_addr,
            controlling_remote: remote,
            rtt_ms: None,
        });
    };
    update_status(false);

    let mut clipboard_tick = interval(Duration::from_millis(750));
    clipboard_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut clipboard = (*clipboard_enabled).then(ClipboardSync::new);

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(ClientCommand::SetClipboardSync(enabled)) => {
                        *clipboard_enabled = enabled;
                        if enabled && clipboard.is_none() {
                            clipboard = Some(ClipboardSync::new());
                        } else if !enabled {
                            clipboard = None;
                        }
                    }
                    Some(ClientCommand::Stop) | None => {
                        let _ = held.release_all(injector);
                        session.close(b"client shutdown").await;
                        return true;
                    }
                }
            }
            _ = clipboard_tick.tick() => {
                if *clipboard_enabled {
                    let cb = clipboard.get_or_insert_with(ClipboardSync::new);
                    if let Some(text) = cb.poll_local_change() {
                        if session.send_reliable(&Message::ClipboardText(text)).await.is_err() {
                            let _ = held.release_all(injector);
                            return false;
                        }
                    }
                }
            }
            msg = session.recv() => {
                match msg {
                    Ok(Some(Message::Input(ev))) => {
                        if let Err(e) = held.apply(injector, ev) {
                            warn!("input injection error: {e:#}");
                        }
                        if let InputEvent::MouseMove { dx, dy } = ev {
                            if controlling_remote && edge_tracker.on_mouse_move(dx, dy) {
                                let _ = held.release_all(injector);
                                edge_tracker.on_deactivate();
                                controlling_remote = false;
                                update_status(false);
                                if session.send_reliable(&Message::EdgeReturn).await.is_err() {
                                    return false;
                                }
                            }
                        }
                    }
                    Ok(Some(Message::ReleaseAllKeys)) => {
                        let _ = held.release_all(injector);
                    }
                    Ok(Some(Message::Control { active, entry_edge })) => {
                        controlling_remote = active;
                        update_status(active);
                        if active {
                            edge_tracker.on_activate(entry_edge);
                            info!(?entry_edge, ">>> Host activated control on THIS machine");
                        } else {
                            edge_tracker.on_deactivate();
                            let _ = held.release_all(injector);
                            info!("<<< Host returned control");
                        }
                    }
                    Ok(Some(Message::ClipboardText(text))) => {
                        if *clipboard_enabled {
                            clipboard
                                .get_or_insert_with(ClipboardSync::new)
                                .apply_remote(&text);
                        }
                    }
                    Ok(Some(Message::Heartbeat { timestamp_ms })) => {
                        // Echo server's exact timestamp back for skew-free RTT calculation.
                        if session.send_reliable(&Message::Heartbeat { timestamp_ms }).await.is_err() {
                            let _ = held.release_all(injector);
                            return false;
                        }
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        info!("Server closed connection; releasing held keys");
                        let _ = held.release_all(injector);
                        return false;
                    }
                    Err(e) => {
                        warn!("Connection lost ({e:#}); releasing held keys");
                        let _ = held.release_all(injector);
                        return false;
                    }
                }
            }
        }
    }
}
