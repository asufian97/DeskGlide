//! Native `egui`/`eframe` desktop GUI for DeskGlide.

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

use deskglide_engine::{
    detect_lan_ip, run_client, run_host, ClientCommand, ClientOptions, EngineStatus, HostCommand,
    HostOptions,
};
use deskglide_input::Hotkey;
use deskglide_net::{
    config_dir, generate_pairing_secret, load_or_create_identity, parse_fingerprint, AppConfig,
    AppMode, DiscoveredHost, MdnsBrowser, PairedHost, PinPolicy, ServerIdentity,
};
use deskglide_proto::{ScreenEdge, DEFAULT_PORT};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::tray::{SystemTray, TrayAction};

const ACCENT: Color32 = Color32::from_rgb(45, 212, 191); // #2DD4BF
const BG_CARD: Color32 = Color32::from_rgb(24, 33, 47);
const BG_ELEVATED: Color32 = Color32::from_rgb(30, 41, 59);
const BORDER_SUBTLE: Color32 = Color32::from_rgb(51, 65, 85);
const TEXT_MUTED: Color32 = Color32::from_rgb(148, 163, 184);
const GREEN: Color32 = Color32::from_rgb(74, 222, 128);
const AMBER: Color32 = Color32::from_rgb(251, 191, 36);
const RED: Color32 = Color32::from_rgb(248, 113, 113);

enum RunningEngine {
    Host {
        cmd_tx: mpsc::Sender<HostCommand>,
        task: JoinHandle<()>,
    },
    Client {
        cmd_tx: mpsc::Sender<ClientCommand>,
        task: JoinHandle<()>,
    },
}

pub struct DeskGlideApp {
    rt: tokio::runtime::Runtime,
    config: AppConfig,
    identity: Option<ServerIdentity>,
    lan_ip: Option<IpAddr>,

    status_tx: watch::Sender<EngineStatus>,
    status_rx: watch::Receiver<EngineStatus>,
    running: Option<RunningEngine>,

    mdns_browser: Option<MdnsBrowser>,
    tray: SystemTray,
    force_quit: bool,

    pairing_target: Option<DiscoveredHost>,
    pairing_code_input: String,
    manual_open: bool,
    manual_addr: String,
    manual_secret: String,
    manual_fingerprint: String,
    manual_tofu: bool,

    notice: Option<(String, bool)>,
    #[cfg(target_os = "linux")]
    uinput_writable: bool,
}

impl DeskGlideApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed creating Tokio runtime");

        let config = AppConfig::load(None);
        if !AppConfig::default_path().exists() {
            let _ = config.save(None);
        }

        let (identity, init_notice) = match load_or_create_identity(None) {
            Ok(id) => (Some(id), None),
            Err(e) => (None, Some((format!("Certificate init failed: {e:#}"), true))),
        };

        let mdns_browser = match MdnsBrowser::start() {
            Ok(b) => Some(b),
            Err(e) => {
                warn!("mDNS discovery unavailable: {e:#}");
                None
            }
        };

        let (status_tx, status_rx) = watch::channel(EngineStatus::Idle);
        let tray = SystemTray::start();
        let lan_ip = detect_lan_ip();

        #[cfg(target_os = "linux")]
        let uinput_writable = check_uinput_writable();

        let mut app = Self {
            rt,
            config,
            identity,
            lan_ip,
            status_tx,
            status_rx,
            running: None,
            mdns_browser,
            tray,
            force_quit: false,
            pairing_target: None,
            pairing_code_input: String::new(),
            manual_open: false,
            manual_addr: String::new(),
            manual_secret: String::new(),
            manual_fingerprint: String::new(),
            manual_tofu: true,
            notice: init_notice,
            #[cfg(target_os = "linux")]
            uinput_writable,
        };

        if app.config.mode == AppMode::Client && app.config.auto_connect {
            if let Some(first) = app.config.paired_hosts.first().cloned() {
                app.connect_to_saved(&first);
            }
        }

        app
    }

    fn save_config(&mut self) {
        if let Err(e) = self.config.save(None) {
            self.notice = Some((format!("Could not save settings: {e:#}"), true));
        }
    }

    fn is_running(&self) -> bool {
        if let Some(r) = &self.running {
            match r {
                RunningEngine::Host { task, .. } | RunningEngine::Client { task, .. } => {
                    !task.is_finished()
                }
            }
        } else {
            false
        }
    }

    fn stop_engine(&mut self) {
        if let Some(running) = self.running.take() {
            match running {
                RunningEngine::Host { cmd_tx, task } => {
                    let _ = cmd_tx.try_send(HostCommand::Stop);
                    let _ = self.rt.block_on(async {
                        tokio::time::timeout(Duration::from_secs(2), task).await
                    });
                }
                RunningEngine::Client { cmd_tx, task } => {
                    let _ = cmd_tx.try_send(ClientCommand::Stop);
                    let _ = self.rt.block_on(async {
                        tokio::time::timeout(Duration::from_secs(2), task).await
                    });
                }
            }
            let _ = self.status_tx.send(EngineStatus::Idle);
        }
    }

    fn start_host(&mut self) {
        self.stop_engine();
        let Some(identity) = self.identity.clone() else {
            self.notice = Some(("Server TLS certificate could not be loaded.".into(), true));
            return;
        };
        let hotkey = match Hotkey::parse(&self.config.hotkey) {
            Ok(hk) => hk,
            Err(e) => {
                self.notice = Some((format!("Invalid hotkey '{}': {e}", self.config.hotkey), true));
                return;
            }
        };

        let opts = HostOptions {
            bind_addr: SocketAddr::from(([0, 0, 0, 0], self.config.port)),
            identity,
            device_name: self.config.device_name.clone(),
            secret: self.config.pairing_secret.clone(),
            hotkey,
            switch_edge: self.config.parsed_edge(),
            coalesce_ms: self.config.coalesce_ms,
            clipboard_sync: self.config.clipboard_sync,
            enable_mdns: true,
        };

        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let status_tx = self.status_tx.clone();
        let task = self.rt.spawn(async move {
            if let Err(e) = run_host(opts, status_tx.clone(), cmd_rx).await {
                error!("Host stopped with error: {e:#}");
                let _ = status_tx.send(EngineStatus::Error(format!("{e:#}")));
            }
        });
        self.running = Some(RunningEngine::Host { cmd_tx, task });
        self.notice = None;
    }

    fn start_client(&mut self, server_addr: SocketAddr, secret: String, pin_policy: PinPolicy) {
        self.stop_engine();
        let opts = ClientOptions {
            server_addr,
            secret,
            pin_policy,
            client_name: self.config.device_name.clone(),
            clipboard_sync: self.config.clipboard_sync,
            auto_reconnect: self.config.auto_connect,
        };
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let status_tx = self.status_tx.clone();
        let task = self.rt.spawn(async move {
            if let Err(e) = run_client(opts, status_tx.clone(), cmd_rx).await {
                error!("Client stopped with error: {e:#}");
                let _ = status_tx.send(EngineStatus::Error(format!("{e:#}")));
            }
        });
        self.running = Some(RunningEngine::Client { cmd_tx, task });
        self.notice = None;
    }

    fn connect_to_saved(&mut self, host: &PairedHost) {
        match (resolve_addr(&host.addr), parse_fingerprint(&host.fingerprint)) {
            (Ok(addr), Ok(fp)) => {
                self.start_client(addr, host.secret.clone(), PinPolicy::Exact(fp));
            }
            (Err(e), _) | (_, Err(e)) => {
                self.notice = Some((format!("Cannot connect to {}: {e:#}", host.name), true));
            }
        }
    }

    fn connect_to_discovered(&mut self, host: &DiscoveredHost, secret: String) {
        match parse_fingerprint(&host.fingerprint) {
            Ok(fp) => {
                self.config.upsert_paired_host(PairedHost {
                    name: host.name.clone(),
                    addr: host.addr.to_string(),
                    fingerprint: host.fingerprint.clone(),
                    secret: secret.clone(),
                });
                self.save_config();
                self.start_client(host.addr, secret, PinPolicy::Exact(fp));
            }
            Err(e) => {
                self.notice = Some((format!("Invalid mDNS fingerprint for {}: {e:#}", host.name), true));
            }
        }
    }

    fn handle_tray_and_close(&mut self, ctx: &egui::Context) {
        while let Some(action) = self.tray.poll_action() {
            match action {
                TrayAction::ShowWindow => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                TrayAction::Quit => {
                    self.force_quit = true;
                    self.stop_engine();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.force_quit && self.tray.is_available() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }
}

impl Drop for DeskGlideApp {
    fn drop(&mut self) {
        self.stop_engine();
    }
}

impl eframe::App for DeskGlideApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(120));
        self.handle_tray_and_close(&ctx);

        // Clean up finished task handle automatically.
        if let Some(r) = &self.running {
            let finished = match r {
                RunningEngine::Host { task, .. } | RunningEngine::Client { task, .. } => {
                    task.is_finished()
                }
            };
            if finished {
                self.running = None;
            }
        }

        let status = self.status_rx.borrow().clone();

        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(18.0))
            .show(ui, |ui| {
                self.draw_header(ui, &status);
                ui.add_space(12.0);

                egui::ScrollArea::vertical().show(ui, |ui| {
                    #[cfg(target_os = "linux")]
                    self.draw_linux_permissions_banner(ui);

                    if let Some((msg, is_err)) = self.notice.clone() {
                        let color = if is_err { RED } else { GREEN };
                        card_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.colored_label(color, &msg);
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("Dismiss").clicked() {
                                        self.notice = None;
                                    }
                                });
                            });
                        });
                        ui.add_space(10.0);
                    }

                    self.draw_status_banner(ui, &status);
                    ui.add_space(12.0);

                    match self.config.mode {
                        AppMode::Host => self.draw_host_panel(ui, &status),
                        AppMode::Client => self.draw_client_panel(ui, &status),
                    }

                    ui.add_space(12.0);
                    self.draw_settings_panel(ui);
                });
            });
    }
}

impl DeskGlideApp {
    fn draw_header(&mut self, ui: &mut egui::Ui, status: &EngineStatus) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("DeskGlide").size(22.0).strong().color(Color32::WHITE));
                ui.label(
                    RichText::new("Encrypted Wi-Fi/LAN Keyboard & Mouse Sharing")
                        .size(12.0)
                        .color(TEXT_MUTED),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Quit").clicked() {
                    self.force_quit = true;
                    self.stop_engine();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if self.tray.is_available() && ui.button("Hide to Tray").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }
                let (dot_color, label) = status_badge(status);
                egui::Frame::new()
                    .fill(BG_ELEVATED)
                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                    .corner_radius(14.0)
                    .inner_margin(egui::Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), 4.0, dot_color);
                            ui.label(RichText::new(label).size(12.0).strong());
                        });
                    });
            });
        });

        ui.add_space(10.0);

        // Role switcher: Host vs Client
        ui.horizontal(|ui| {
            let host_selected = self.config.mode == AppMode::Host;
            let client_selected = self.config.mode == AppMode::Client;
            let btn_w = (ui.available_width() - 8.0) / 2.0;

            if ui
                .add_sized(
                    [btn_w, 34.0],
                    egui::Button::selectable(
                        host_selected,
                        RichText::new("Host (Share my keyboard & mouse)").strong(),
                    ),
                )
                .clicked()
                && !host_selected
            {
                self.stop_engine();
                self.config.mode = AppMode::Host;
                self.save_config();
            }

            if ui
                .add_sized(
                    [btn_w, 34.0],
                    egui::Button::selectable(
                        client_selected,
                        RichText::new("Client (Control this PC)").strong(),
                    ),
                )
                .clicked()
                && !client_selected
            {
                self.stop_engine();
                self.config.mode = AppMode::Client;
                self.save_config();
            }
        });
    }

    fn draw_status_banner(&mut self, ui: &mut egui::Ui, status: &EngineStatus) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            match status {
                EngineStatus::Idle => {
                    let hint = match self.config.mode {
                        AppMode::Host => "Click 'Start Sharing' so another PC on your Wi-Fi can connect.",
                        AppMode::Client => "Pick a discovered Host below (or enter an IP) to connect.",
                    };
                    ui.label(RichText::new(hint).color(TEXT_MUTED));
                }
                EngineStatus::Listening { lan_ip, bind_addr, .. } => {
                    let ip_txt = lan_ip
                        .map(|ip| ip.to_string())
                        .unwrap_or_else(|| "0.0.0.0".to_string());
                    ui.horizontal(|ui| {
                        ui.colored_label(ACCENT, "Waiting for client on Wi-Fi...");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{ip_txt}:{} (QUIC TLS 1.3)", bind_addr.port()))
                                    .monospace()
                                    .size(12.0)
                                    .color(TEXT_MUTED),
                            );
                        });
                    });
                }
                EngineStatus::Connecting { target, attempt } => {
                    ui.colored_label(
                        AMBER,
                        format!("Connecting to {target} (attempt {attempt})..."),
                    );
                }
                EngineStatus::Connected {
                    peer_name,
                    peer_os,
                    controlling_remote,
                    rtt_ms,
                    ..
                } => {
                    ui.horizontal(|ui| {
                        let msg = if *controlling_remote {
                            format!("Controlling: {peer_name} ({peer_os})")
                        } else {
                            format!("Connected to {peer_name} ({peer_os}) — Local active")
                        };
                        ui.colored_label(GREEN, RichText::new(msg).strong());
                        if let Some(rtt) = rtt_ms {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("{rtt} ms · TLS 1.3"))
                                        .size(12.0)
                                        .color(TEXT_MUTED),
                                );
                            });
                        }
                    });
                }
                EngineStatus::Error(err) => {
                    ui.colored_label(RED, format!("Error: {err}"));
                }
            }
        });
    }

    fn draw_host_panel(&mut self, ui: &mut egui::Ui, status: &EngineStatus) {
        let running = self.is_running();

        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if running {
                    if ui
                        .add_sized(
                            [160.0, 36.0],
                            egui::Button::new(RichText::new("Stop Sharing").strong())
                                .fill(Color32::from_rgb(127, 29, 29)),
                        )
                        .clicked()
                    {
                        self.stop_engine();
                    }
                    if matches!(status, EngineStatus::Connected { .. })
                        && ui
                            .add_sized([160.0, 36.0], egui::Button::new("Switch Control Now"))
                            .clicked()
                    {
                        if let Some(RunningEngine::Host { cmd_tx, .. }) = &self.running {
                            let _ = cmd_tx.try_send(HostCommand::ToggleControl);
                        }
                    }
                } else if ui
                    .add_sized(
                        [180.0, 36.0],
                        egui::Button::new(
                            RichText::new("Start Sharing").strong().color(Color32::BLACK),
                        )
                        .fill(ACCENT),
                    )
                    .clicked()
                {
                    self.start_host();
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("Hotkey: {}", self.config.hotkey))
                            .size(12.0)
                            .color(TEXT_MUTED),
                    );
                });
            });

            ui.add_space(12.0);
            ui.separator();
            ui.add_space(8.0);

            // Pairing Code & Fingerprint
            ui.label(RichText::new("Pairing Details").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Pairing Code:");
                ui.label(
                    RichText::new(&self.config.pairing_secret)
                        .monospace()
                        .size(18.0)
                        .strong()
                        .color(ACCENT),
                );
                if ui.small_button("Copy Code").clicked() {
                    ui.ctx().copy_text(self.config.pairing_secret.clone());
                    self.notice = Some(("Pairing code copied.".into(), false));
                }
                if !running && ui.small_button("New Code").clicked() {
                    self.config.pairing_secret = generate_pairing_secret();
                    self.save_config();
                }
            });

            if let Some(ip) = self.lan_ip {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("LAN IP:").color(TEXT_MUTED));
                    ui.label(RichText::new(format!("{ip}:{}", self.config.port)).monospace());
                });
            }
            if let Some(id) = &self.identity {
                let fp = id.fingerprint_str();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("TLS Fingerprint:").color(TEXT_MUTED));
                    ui.label(RichText::new(&fp[..23]).monospace().size(11.5));
                    if ui.small_button("Copy Full").clicked() {
                        ui.ctx().copy_text(fp);
                        self.notice = Some(("Full SHA-256 fingerprint copied.".into(), false));
                    }
                });
            }

            #[cfg(target_os = "windows")]
            {
                ui.add_space(6.0);
                if ui
                    .small_button("Allow Windows Firewall (UDP 24910)")
                    .clicked()
                {
                    match open_windows_firewall_uac(self.config.port) {
                        Ok(()) => {
                            self.notice = Some((
                                "Firewall prompt opened — click Yes on UAC to allow UDP 24910."
                                    .into(),
                                false,
                            ));
                        }
                        Err(e) => {
                            self.notice =
                                Some((format!("Could not launch firewall helper: {e}"), true));
                        }
                    }
                }
            }
        });

        ui.add_space(10.0);

        // Visual Monitor Edge Layout Selector
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Where is your other PC? (Screen-Edge Switch)").strong());
            ui.label(
                RichText::new(
                    "Move your mouse past the chosen monitor edge to jump automatically.",
                )
                .size(12.0)
                .color(TEXT_MUTED),
            );
            ui.add_space(8.0);

            let current = self.config.parsed_edge();
            let mut next_edge: Option<ScreenEdge> = None;

            ui.vertical_centered(|ui| {
                if edge_btn(ui, "Top", current == ScreenEdge::Top) {
                    next_edge = Some(ScreenEdge::Top);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let pad = ((ui.available_width() - 280.0) / 2.0).max(0.0);
                    ui.add_space(pad);
                    if edge_btn(ui, "Left", current == ScreenEdge::Left) {
                        next_edge = Some(ScreenEdge::Left);
                    }
                    egui::Frame::new()
                        .fill(BG_ELEVATED)
                        .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(18, 10))
                        .show(ui, |ui| {
                            ui.label(RichText::new("THIS PC").strong().size(12.0));
                        });
                    if edge_btn(ui, "Right", current == ScreenEdge::Right) {
                        next_edge = Some(ScreenEdge::Right);
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let pad = ((ui.available_width() - 210.0) / 2.0).max(0.0);
                    ui.add_space(pad);
                    if edge_btn(ui, "Bottom", current == ScreenEdge::Bottom) {
                        next_edge = Some(ScreenEdge::Bottom);
                    }
                    if edge_btn(ui, "Hotkey Only", current == ScreenEdge::None) {
                        next_edge = Some(ScreenEdge::None);
                    }
                });
            });

            if let Some(edge) = next_edge {
                self.config.set_edge(edge);
                self.save_config();
                if let Some(RunningEngine::Host { cmd_tx, .. }) = &self.running {
                    let _ = cmd_tx.try_send(HostCommand::SetSwitchEdge(edge));
                }
            }
        });
    }

    fn draw_client_panel(&mut self, ui: &mut egui::Ui, _status: &EngineStatus) {
        let running = self.is_running();
        let self_fp = self
            .identity
            .as_ref()
            .map(|id| id.fingerprint_str())
            .unwrap_or_default();

        // Discovered Hosts on Wi-Fi (mDNS)
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("PCs Found on Your Wi-Fi").strong());
                if running {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(RichText::new("Disconnect").color(RED))
                            .clicked()
                        {
                            self.stop_engine();
                        }
                    });
                }
            });
            ui.add_space(6.0);

            let discovered: Vec<DiscoveredHost> = self
                .mdns_browser
                .as_ref()
                .map(|b| {
                    b.snapshot()
                        .into_iter()
                        .filter(|h| !h.fingerprint.eq_ignore_ascii_case(&self_fp))
                        .collect()
                })
                .unwrap_or_default();

            if discovered.is_empty() {
                ui.label(
                    RichText::new(
                        "Searching... Start DeskGlide in Host mode on your other PC.",
                    )
                    .size(12.0)
                    .color(TEXT_MUTED),
                );
            } else {
                for host in discovered {
                    let paired = self.config.find_paired(&host.fingerprint).cloned();
                    egui::Frame::new()
                        .fill(BG_ELEVATED)
                        .corner_radius(6.0)
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new(format!("{} ({})", host.name, host.os))
                                            .strong(),
                                    );
                                    ui.label(
                                        RichText::new(host.addr.to_string())
                                            .monospace()
                                            .size(11.5)
                                            .color(TEXT_MUTED),
                                    );
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if let Some(saved) = paired {
                                            if ui
                                                .add(egui::Button::new("Connect").fill(ACCENT))
                                                .clicked()
                                            {
                                                let secret = saved.secret;
                                                self.connect_to_discovered(&host, secret);
                                            }
                                            ui.label(
                                                RichText::new("Paired")
                                                    .size(11.5)
                                                    .color(GREEN),
                                            );
                                        } else if ui.button("Pair & Connect").clicked() {
                                            self.pairing_target = Some(host);
                                            self.pairing_code_input.clear();
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(4.0);
                }
            }

            if let Some(target) = self.pairing_target.clone() {
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(BG_ELEVATED)
                    .stroke(Stroke::new(1.0, ACCENT))
                    .corner_radius(6.0)
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("Enter pairing code shown on {}:", target.name))
                                .strong(),
                        );
                        ui.horizontal(|ui| {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.pairing_code_input)
                                    .hint_text("e.g. K7M2-9X4P")
                                    .desired_width(180.0),
                            );
                            let enter = resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if (ui.button("Pair").clicked() || enter)
                                && !self.pairing_code_input.trim().is_empty()
                            {
                                let code = self.pairing_code_input.trim().to_string();
                                self.pairing_target = None;
                                self.connect_to_discovered(&target, code);
                            }
                            if ui.button("Cancel").clicked() {
                                self.pairing_target = None;
                            }
                        });
                    });
            }
        });

        // Saved Paired Hosts
        if !self.config.paired_hosts.is_empty() {
            ui.add_space(10.0);
            card_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Saved Devices (Pair Once)").strong());
                ui.add_space(6.0);

                let list = self.config.paired_hosts.clone();
                for saved in list {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&saved.name).strong());
                        ui.label(
                            RichText::new(&saved.addr)
                                .monospace()
                                .size(11.5)
                                .color(TEXT_MUTED),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Forget").clicked() {
                                self.config.remove_paired_host(&saved.fingerprint);
                                self.save_config();
                            }
                            if ui.small_button("Connect").clicked() {
                                self.connect_to_saved(&saved);
                            }
                        });
                    });
                }
            });
        }

        // Manual IP Fallback
        ui.add_space(10.0);
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if ui
                .selectable_label(self.manual_open, "Manual IP Connection (Fallback)")
                .clicked()
            {
                self.manual_open = !self.manual_open;
            }
            if self.manual_open {
                ui.add_space(6.0);
                egui::Grid::new("manual_grid")
                    .num_columns(2)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Host IP:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.manual_addr)
                                .hint_text("192.168.1.50:24910"),
                        );
                        ui.end_row();

                        ui.label("Pairing Code:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.manual_secret)
                                .hint_text("XXXX-XXXX"),
                        );
                        ui.end_row();

                        ui.label("Fingerprint:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.manual_fingerprint)
                                .hint_text("Optional if TOFU checked"),
                        );
                        ui.end_row();
                    });
                ui.checkbox(&mut self.manual_tofu, "Trust & pin certificate on first connect (TOFU)");
                if ui.button("Connect Manually").clicked() {
                    match resolve_addr(&self.manual_addr) {
                        Ok(addr) => {
                            let policy = if !self.manual_fingerprint.trim().is_empty() {
                                match parse_fingerprint(&self.manual_fingerprint) {
                                    Ok(fp) => PinPolicy::Exact(fp),
                                    Err(e) => {
                                        self.notice = Some((format!("{e:#}"), true));
                                        return;
                                    }
                                }
                            } else {
                                PinPolicy::KnownOrTofu {
                                    known_hosts_path: config_dir().join("known_hosts"),
                                    peer_key: addr.to_string(),
                                    allow_tofu: self.manual_tofu,
                                }
                            };
                            let secret = self.manual_secret.trim().to_string();
                            self.start_client(addr, secret, policy);
                        }
                        Err(e) => {
                            self.notice = Some((format!("{e:#}"), true));
                        }
                    }
                }
            }
        });
    }

    fn draw_settings_panel(&mut self, ui: &mut egui::Ui) {
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Settings").strong());
            ui.add_space(6.0);

            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label("This PC's Name:");
                if ui
                    .add(egui::TextEdit::singleline(&mut self.config.device_name).desired_width(180.0))
                    .changed()
                {
                    changed = true;
                }
            });

            if ui
                .checkbox(&mut self.config.clipboard_sync, "Sync Clipboard Text (Encrypted)")
                .changed()
            {
                changed = true;
                match &self.running {
                    Some(RunningEngine::Host { cmd_tx, .. }) => {
                        let _ = cmd_tx.try_send(HostCommand::SetClipboardSync(self.config.clipboard_sync));
                    }
                    Some(RunningEngine::Client { cmd_tx, .. }) => {
                        let _ = cmd_tx.try_send(ClientCommand::SetClipboardSync(self.config.clipboard_sync));
                    }
                    None => {}
                }
            }

            if ui
                .checkbox(
                    &mut self.config.auto_connect,
                    "Auto-Reconnect (and connect on launch in Client mode)",
                )
                .changed()
            {
                changed = true;
            }

            ui.horizontal(|ui| {
                ui.label("Mouse Coalescing:");
                if ui
                    .add(egui::Slider::new(&mut self.config.coalesce_ms, 1..=16).suffix(" ms"))
                    .changed()
                {
                    changed = true;
                }
            });

            if changed {
                self.save_config();
            }
        });
    }

    #[cfg(target_os = "linux")]
    fn draw_linux_permissions_banner(&mut self, ui: &mut egui::Ui) {
        if self.uinput_writable {
            return;
        }
        card_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.colored_label(
                AMBER,
                RichText::new("Linux input permissions required for /dev/uinput").strong(),
            );
            ui.label(
                RichText::new(
                    "Click once below to install the udev rule (uses system password prompt).",
                )
                .size(12.0)
                .color(TEXT_MUTED),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Enable Permissions (1-Click)").clicked() {
                    match run_linux_pkexec_setup() {
                        Ok(()) => {
                            self.uinput_writable = check_uinput_writable();
                            if self.uinput_writable {
                                self.notice = Some(("Linux input permissions enabled!".into(), false));
                            } else {
                                self.notice = Some((
                                    "Rule installed! Log out and back in once if Host mode still needs 'input' group."
                                        .into(),
                                    false,
                                ));
                            }
                        }
                        Err(e) => {
                            self.notice = Some((format!("Permission setup failed: {e}"), true));
                        }
                    }
                }
                if ui.small_button("Re-check").clicked() {
                    self.uinput_writable = check_uinput_writable();
                }
            });
        });
        ui.add_space(10.0);
    }
}

fn status_badge(status: &EngineStatus) -> (Color32, &'static str) {
    match status {
        EngineStatus::Idle => (TEXT_MUTED, "Idle"),
        EngineStatus::Listening { .. } => (ACCENT, "Listening"),
        EngineStatus::Connecting { .. } => (AMBER, "Connecting"),
        EngineStatus::Connected {
            controlling_remote, ..
        } => {
            if *controlling_remote {
                (GREEN, "Remote Active")
            } else {
                (GREEN, "Connected")
            }
        }
        EngineStatus::Error(_) => (RED, "Error"),
    }
}

fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(BG_CARD)
        .stroke(Stroke::new(1.0, BORDER_SUBTLE))
        .corner_radius(10.0)
        .inner_margin(14.0)
}

fn edge_btn(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let btn = egui::Button::selectable(selected, RichText::new(label).size(12.0))
        .min_size(Vec2::new(84.0, 28.0));
    ui.add(btn).clicked()
}

fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(15, 23, 42); // #0F172A
    visuals.widgets.noninteractive.bg_fill = BG_CARD;
    visuals.widgets.inactive.bg_fill = BG_ELEVATED;
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(51, 65, 85);
    visuals.selection.bg_fill = Color32::from_rgb(20, 184, 166);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    ctx.set_visuals(visuals);
}

fn resolve_addr(raw: &str) -> anyhow::Result<SocketAddr> {
    let trimmed = raw.trim();
    if let Ok(addr) = trimmed.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    let with_port = if trimmed.contains(':') {
        trimmed.to_string()
    } else {
        format!("{trimmed}:{DEFAULT_PORT}")
    };
    with_port
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("could not resolve '{trimmed}'"))
}

#[cfg(target_os = "windows")]
fn open_windows_firewall_uac(port: u16) -> anyhow::Result<()> {
    let args = format!(
        "advfirewall firewall add rule name=\"DeskGlide (QUIC/UDP)\" dir=in action=allow protocol=UDP localport={port}"
    );
    let ps_cmd = format!(
        "Start-Process -FilePath netsh -ArgumentList '{}' -Verb RunAs",
        args.replace('\'', "''")
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps_cmd])
        .spawn()?;
    info!("Launched UAC prompt to add Windows Firewall rule for UDP {port}");
    Ok(())
}

#[cfg(target_os = "linux")]
fn check_uinput_writable() -> bool {
    std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok()
}

#[cfg(target_os = "linux")]
fn run_linux_pkexec_setup() -> anyhow::Result<()> {
    let user = std::env::var("USER").unwrap_or_default();
    let script = format!(
        "getent group input >/dev/null || groupadd --system input; \
         [ -n '{user}' ] && usermod -aG input '{user}' || true; \
         printf 'uinput\\n' > /etc/modules-load.d/deskglide.conf; \
         printf 'KERNEL==\"uinput\", SUBSYSTEM==\"misc\", MODE=\"0660\", GROUP=\"input\", TAG+=\"uaccess\", OPTIONS+=\"static_node=uinput\"\\n' > /etc/udev/rules.d/70-deskglide.rules; \
         modprobe uinput || true; \
         udevadm control --reload-rules; \
         udevadm trigger --subsystem-match=misc --sysname-match=uinput || true"
    );
    let status = std::process::Command::new("pkexec")
        .args(["sh", "-c", &script])
        .status()?;
    if !status.success() {
        anyhow::bail!("pkexec exited with {status}");
    }
    Ok(())
}
