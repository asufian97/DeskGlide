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
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Pos2, Rect, RichText, Sense,
    Stroke, StrokeKind, Vec2,
};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::tray::{SystemTray, TrayAction};

// Cohesive slate + teal palette
const BG_CANVAS: Color32 = Color32::from_rgb(11, 15, 23); // #0B0F17
const BG_HEADER: Color32 = Color32::from_rgb(17, 24, 39); // #111827
const BG_CARD: Color32 = Color32::from_rgb(21, 29, 43); // #151D2B
const BG_RECESSED: Color32 = Color32::from_rgb(13, 19, 31); // #0D131F
const BG_ELEVATED: Color32 = Color32::from_rgb(30, 41, 59); // #1E293B
const BG_HOVER: Color32 = Color32::from_rgb(41, 55, 78); // #29374E
const BORDER_SUBTLE: Color32 = Color32::from_rgb(36, 48, 68); // #243044
const BORDER_WIDGET: Color32 = Color32::from_rgb(51, 65, 85); // #334155
const BORDER_HOVER: Color32 = Color32::from_rgb(100, 116, 139); // #64748B

const ACCENT: Color32 = Color32::from_rgb(45, 212, 191); // #2DD4BF
const ACCENT_TEXT_DARK: Color32 = Color32::from_rgb(4, 47, 46); // #042F2E
const ACCENT_TINT: Color32 = Color32::from_rgb(18, 58, 59); // #123A3B
const ACCENT_LIGHT: Color32 = Color32::from_rgb(204, 251, 241); // #CCFBF1

const TEXT_PRIMARY: Color32 = Color32::from_rgb(241, 245, 249); // #F1F5F9
const TEXT_MUTED: Color32 = Color32::from_rgb(148, 163, 184); // #94A3B8
const GREEN: Color32 = Color32::from_rgb(74, 222, 128);
const AMBER: Color32 = Color32::from_rgb(251, 191, 36);
const RED: Color32 = Color32::from_rgb(251, 113, 133);
const DANGER_BG: Color32 = Color32::from_rgb(59, 24, 34);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Host,
    Client,
    Settings,
}

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
    tab: ActiveTab,
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

    hotkey_draft: String,
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
        let tab = match config.mode {
            AppMode::Host => ActiveTab::Host,
            AppMode::Client => ActiveTab::Client,
        };
        let hotkey_draft = config.hotkey.clone();

        #[cfg(target_os = "linux")]
        let uinput_writable = check_uinput_writable();

        let mut app = Self {
            rt,
            config,
            tab,
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
            hotkey_draft,
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
                self.notice = Some((
                    format!("Invalid mDNS fingerprint for {}: {e:#}", host.name),
                    true,
                ));
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

        if ctx.input(|i| i.viewport().close_requested())
            && !self.force_quit
            && self.tray.is_available()
        {
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
            .frame(egui::Frame::new().fill(BG_CANVAS).inner_margin(0.0))
            .show(ui, |ui| {
                egui::Frame::new()
                    .fill(BG_HEADER)
                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                    .inner_margin(egui::Margin::symmetric(18, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.draw_header(ui, &status);
                    });

                egui::Frame::new()
                    .fill(BG_CANVAS)
                    .inner_margin(egui::Margin::symmetric(18, 16))
                    .show(ui, |ui| {
                        #[cfg(target_os = "linux")]
                        self.draw_linux_permissions_banner(ui);

                        if let Some((msg, is_err)) = self.notice.clone() {
                            let color = if is_err { RED } else { GREEN };
                            egui::Frame::new()
                                .fill(if is_err { DANGER_BG } else { ACCENT_TINT })
                                .stroke(Stroke::new(1.0, color))
                                .corner_radius(8.0)
                                .inner_margin(egui::Margin::symmetric(12, 8))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.colored_label(color, &msg);
                                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                            if ui.small_button("Dismiss").clicked() {
                                                self.notice = None;
                                            }
                                        });
                                    });
                                });
                            ui.add_space(10.0);
                        }

                        match self.tab {
                            ActiveTab::Host => self.draw_host_tab(ui, &status),
                            ActiveTab::Client => self.draw_client_tab(ui, &status),
                            ActiveTab::Settings => self.draw_settings_tab(ui),
                        }
                    });
            });
    }
}

impl DeskGlideApp {
    fn draw_header(&mut self, ui: &mut egui::Ui, status: &EngineStatus) {
        ui.horizontal(|ui| {
            // Left: painted badge + title
            let (badge_rect, _) = ui.allocate_exact_size(Vec2::new(26.0, 26.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(badge_rect, 6.0, BG_ELEVATED);
            p.rect_stroke(
                badge_rect,
                6.0,
                Stroke::new(1.2, ACCENT),
                StrokeKind::Inside,
            );
            let c = badge_rect.center();
            p.line_segment(
                [Pos2::new(c.x - 6.0, c.y), Pos2::new(c.x + 6.0, c.y)],
                Stroke::new(2.0, ACCENT),
            );
            p.line_segment(
                [Pos2::new(c.x + 2.5, c.y - 3.5), Pos2::new(c.x + 6.0, c.y)],
                Stroke::new(2.0, ACCENT),
            );
            p.line_segment(
                [Pos2::new(c.x + 2.5, c.y + 3.5), Pos2::new(c.x + 6.0, c.y)],
                Stroke::new(2.0, ACCENT),
            );

            ui.add_space(4.0);
            ui.label(
                RichText::new("DeskGlide")
                    .size(18.0)
                    .strong()
                    .color(TEXT_PRIMARY),
            );

            ui.add_space(16.0);

            // Center: Segmented navigation pill (Host | Client | Settings)
            egui::Frame::new()
                .fill(BG_RECESSED)
                .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        if nav_pill(ui, "Host", self.tab == ActiveTab::Host) {
                            if self.config.mode != AppMode::Host {
                                self.stop_engine();
                                self.config.mode = AppMode::Host;
                                self.save_config();
                            }
                            self.tab = ActiveTab::Host;
                        }
                        if nav_pill(ui, "Client", self.tab == ActiveTab::Client) {
                            if self.config.mode != AppMode::Client {
                                self.stop_engine();
                                self.config.mode = AppMode::Client;
                                self.save_config();
                            }
                            self.tab = ActiveTab::Client;
                        }
                        if nav_pill(ui, "Settings", self.tab == ActiveTab::Settings) {
                            self.tab = ActiveTab::Settings;
                        }
                    });
                });

            // Right: Status pill + Hide to Tray
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.tray.is_available() && ui.button("Hide").clicked() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Visible(false));
                }

                let (dot_color, status_text) = status_badge(status);
                egui::Frame::new()
                    .fill(BG_RECESSED)
                    .stroke(Stroke::new(1.0, BORDER_WIDGET))
                    .corner_radius(14.0)
                    .inner_margin(egui::Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::new(8.0, 8.0), Sense::hover());
                            ui.painter().circle_filled(rect.center(), 4.0, dot_color);
                            ui.label(
                                RichText::new(status_text)
                                    .size(12.0)
                                    .strong()
                                    .color(TEXT_PRIMARY),
                            );
                        });
                    });
            });
        });
    }

    fn draw_host_tab(&mut self, ui: &mut egui::Ui, status: &EngineStatus) {
        let running = self.is_running();
        let avail_w = ui.available_width();
        let gap = 14.0;
        let left_w = ((avail_w - gap) * 0.50).floor();
        let right_w = avail_w - gap - left_w;
        let card_h = ui.available_height().max(400.0);

        ui.horizontal_top(|ui| {
            // LEFT CARD: Share & Pair
            ui.allocate_ui_with_layout(
                Vec2::new(left_w, card_h),
                Layout::top_down(Align::Min),
                |ui| {
                    card_frame().show(ui, |ui| {
                        ui.set_min_height(card_h - 32.0);
                        ui.set_width(ui.available_width());

                        let (title, subtitle, title_color) = match status {
                            EngineStatus::Connected {
                                peer_name,
                                peer_os,
                                controlling_remote,
                                rtt_ms,
                                ..
                            } => {
                                let sub = match (controlling_remote, rtt_ms) {
                                    (true, Some(r)) => {
                                        format!("Controlling {peer_name} ({peer_os}) · {r} ms")
                                    }
                                    (true, None) => format!("Controlling {peer_name} ({peer_os})"),
                                    (false, Some(r)) => {
                                        format!("Paired with {peer_name} ({peer_os}) · {r} ms")
                                    }
                                    (false, None) => format!("Paired with {peer_name} ({peer_os})"),
                                };
                                ("Client Connected", sub, GREEN)
                            }
                            EngineStatus::Listening { .. } => (
                                "Waiting for Client",
                                "Open DeskGlide on your second PC and pick this Host.".into(),
                                ACCENT,
                            ),
                            EngineStatus::Error(err) => ("Host Error", err.clone(), RED),
                            _ => (
                                "Share This Keyboard & Mouse",
                                "Start sharing, then pair once from your second PC.".into(),
                                TEXT_PRIMARY,
                            ),
                        };

                        ui.label(RichText::new(title).size(15.5).strong().color(title_color));
                        ui.label(RichText::new(subtitle).size(12.0).color(TEXT_MUTED));
                        ui.add_space(12.0);

                        // Primary CTA
                        if running {
                            let is_connected = matches!(status, EngineStatus::Connected { .. });
                            ui.horizontal(|ui| {
                                let stop_w = if is_connected {
                                    (ui.available_width() - 8.0) * 0.48
                                } else {
                                    ui.available_width()
                                };
                                if ui
                                    .add_sized(
                                        [stop_w, 38.0],
                                        egui::Button::new(
                                            RichText::new("Stop Sharing").strong().color(RED),
                                        )
                                        .fill(DANGER_BG)
                                        .stroke(Stroke::new(1.0, RED)),
                                    )
                                    .clicked()
                                {
                                    self.stop_engine();
                                }

                                if is_connected
                                    && ui
                                        .add_sized(
                                            [ui.available_width(), 38.0],
                                            egui::Button::new(
                                                RichText::new("Switch Control")
                                                    .strong()
                                                    .color(ACCENT_TEXT_DARK),
                                            )
                                            .fill(ACCENT),
                                        )
                                        .clicked()
                                {
                                    if let Some(RunningEngine::Host { cmd_tx, .. }) = &self.running
                                    {
                                        let _ = cmd_tx.try_send(HostCommand::ToggleControl);
                                    }
                                }
                            });
                        } else if ui
                            .add_sized(
                                [ui.available_width(), 38.0],
                                egui::Button::new(
                                    RichText::new("Start Sharing")
                                        .size(14.0)
                                        .strong()
                                        .color(ACCENT_TEXT_DARK),
                                )
                                .fill(ACCENT),
                            )
                            .clicked()
                        {
                            self.start_host();
                        }

                        ui.add_space(14.0);

                        // Recessed Pairing Code Box
                        egui::Frame::new()
                            .fill(BG_RECESSED)
                            .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                            .corner_radius(10.0)
                            .inner_margin(12.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(
                                    RichText::new("PAIRING CODE")
                                        .size(10.5)
                                        .strong()
                                        .color(TEXT_MUTED),
                                );
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&self.config.pairing_secret)
                                            .monospace()
                                            .size(22.0)
                                            .strong()
                                            .color(ACCENT),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if !running && ui.button("New").clicked() {
                                            self.config.pairing_secret = generate_pairing_secret();
                                            self.save_config();
                                        }
                                        if ui.button("Copy").clicked() {
                                            ui.ctx()
                                                .copy_text(self.config.pairing_secret.clone());
                                            self.notice =
                                                Some(("Pairing code copied.".into(), false));
                                        }
                                    });
                                });
                            });

                        ui.add_space(12.0);

                        // Clean key-value metadata rows
                        let ip_str = self
                            .lan_ip
                            .map(|ip| format!("{ip}:{}", self.config.port))
                            .unwrap_or_else(|| format!("0.0.0.0:{}", self.config.port));
                        info_row(ui, "LAN Address", &ip_str);
                        ui.add_space(4.0);
                        info_row(ui, "Toggle Hotkey", &self.config.hotkey);

                        if let Some(id) = &self.identity {
                            ui.add_space(4.0);
                            let fp = id.fingerprint_str();
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("TLS Fingerprint").size(12.0).color(TEXT_MUTED));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.small_button("Copy").clicked() {
                                        ui.ctx().copy_text(fp.clone());
                                        self.notice = Some((
                                            "Full SHA-256 fingerprint copied.".into(),
                                            false,
                                        ));
                                    }
                                    ui.label(
                                        RichText::new(&fp[..17])
                                            .monospace()
                                            .size(12.0)
                                            .color(TEXT_PRIMARY),
                                    );
                                });
                            });
                        }

                        #[cfg(target_os = "windows")]
                        {
                            ui.add_space(14.0);
                            if ui
                                .add_sized(
                                    [ui.available_width(), 32.0],
                                    egui::Button::new("Allow Windows Firewall (UDP 24910)"),
                                )
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
                                        self.notice = Some((
                                            format!("Could not launch firewall helper: {e}"),
                                            true,
                                        ));
                                    }
                                }
                            }
                        }
                    });
                },
            );

            ui.add_space(gap);

            // RIGHT CARD: Screen Arrangement Monitor Diagram
            ui.allocate_ui_with_layout(
                Vec2::new(right_w, card_h),
                Layout::top_down(Align::Min),
                |ui| {
                    card_frame().show(ui, |ui| {
                        ui.set_min_height(card_h - 32.0);
                        ui.set_width(ui.available_width());

                        ui.label(
                            RichText::new("Screen Arrangement")
                                .size(15.5)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                        ui.label(
                            RichText::new("Click where your second PC is placed.")
                                .size(12.0)
                                .color(TEXT_MUTED),
                        );
                        ui.add_space(10.0);

                        let current_edge = self.config.parsed_edge();
                        let picked = draw_monitor_diagram(ui, current_edge);
                        ui.add_space(10.0);

                        let mut next_edge = picked;
                        let hotkey_only = current_edge == ScreenEdge::None;
                        let btn = egui::Button::selectable(
                            hotkey_only,
                            RichText::new(format!(
                                "Hotkey Only ({})",
                                self.config.hotkey
                            ))
                            .size(12.5)
                            .color(if hotkey_only {
                                ACCENT_LIGHT
                            } else {
                                TEXT_PRIMARY
                            }),
                        );
                        if ui.add_sized([ui.available_width(), 32.0], btn).clicked() {
                            next_edge = Some(ScreenEdge::None);
                        }

                        if let Some(edge) = next_edge {
                            self.config.set_edge(edge);
                            self.save_config();
                            if let Some(RunningEngine::Host { cmd_tx, .. }) = &self.running {
                                let _ = cmd_tx.try_send(HostCommand::SetSwitchEdge(edge));
                            }
                        }
                    });
                },
            );
        });
    }

    fn draw_client_tab(&mut self, ui: &mut egui::Ui, _status: &EngineStatus) {
        let running = self.is_running();
        let self_fp = self
            .identity
            .as_ref()
            .map(|id| id.fingerprint_str())
            .unwrap_or_default();

        let avail_w = ui.available_width();
        let gap = 14.0;
        let left_w = ((avail_w - gap) * 0.56).floor();
        let right_w = avail_w - gap - left_w;
        let card_h = ui.available_height().max(400.0);

        ui.horizontal_top(|ui| {
            // LEFT CARD: Nearby Wi-Fi Hosts (mDNS)
            ui.allocate_ui_with_layout(
                Vec2::new(left_w, card_h),
                Layout::top_down(Align::Min),
                |ui| {
                    card_frame().show(ui, |ui| {
                        ui.set_min_height(card_h - 32.0);
                        ui.set_width(ui.available_width());

                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("PCs Found on Your Wi-Fi")
                                    .size(15.5)
                                    .strong()
                                    .color(TEXT_PRIMARY),
                            );
                            if running {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("Disconnect").strong().color(RED),
                                            )
                                            .fill(DANGER_BG)
                                            .stroke(Stroke::new(1.0, RED)),
                                        )
                                        .clicked()
                                    {
                                        self.stop_engine();
                                    }
                                });
                            }
                        });
                        ui.label(
                            RichText::new("Start Host mode on your other PC to see it here.")
                                .size(12.0)
                                .color(TEXT_MUTED),
                        );
                        ui.add_space(10.0);

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

                        egui::ScrollArea::vertical()
                            .id_salt("mdns_scroll")
                            .max_height(card_h - 110.0)
                            .show(ui, |ui| {
                                if discovered.is_empty() {
                                    egui::Frame::new()
                                        .fill(BG_RECESSED)
                                        .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                                        .corner_radius(8.0)
                                        .inner_margin(16.0)
                                        .show(ui, |ui| {
                                            ui.set_width(ui.available_width());
                                            ui.vertical_centered(|ui| {
                                                ui.label(
                                                    RichText::new("Searching local Wi-Fi...")
                                                        .color(TEXT_MUTED),
                                                );
                                                ui.label(
                                                    RichText::new(
                                                        "Make sure UDP port 24910 is allowed on the Host.",
                                                    )
                                                    .size(11.5)
                                                    .color(TEXT_MUTED),
                                                );
                                            });
                                        });
                                } else {
                                    for host in discovered {
                                        let paired =
                                            self.config.find_paired(&host.fingerprint).cloned();
                                        egui::Frame::new()
                                            .fill(BG_RECESSED)
                                            .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                                            .corner_radius(8.0)
                                            .inner_margin(10.0)
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.vertical(|ui| {
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "{} ({})",
                                                                host.name, host.os
                                                            ))
                                                            .strong()
                                                            .color(TEXT_PRIMARY),
                                                        );
                                                        ui.label(
                                                            RichText::new(host.addr.to_string())
                                                                .monospace()
                                                                .size(11.5)
                                                                .color(TEXT_MUTED),
                                                        );
                                                    });
                                                    ui.with_layout(
                                                        Layout::right_to_left(Align::Center),
                                                        |ui| {
                                                            if let Some(saved) = paired {
                                                                if ui
                                                                    .add(
                                                                        egui::Button::new(
                                                                            RichText::new("Connect")
                                                                                .strong()
                                                                                .color(ACCENT_TEXT_DARK),
                                                                        )
                                                                        .fill(ACCENT),
                                                                    )
                                                                    .clicked()
                                                                {
                                                                    self.connect_to_discovered(
                                                                        &host,
                                                                        saved.secret,
                                                                    );
                                                                }
                                                                ui.label(
                                                                    RichText::new("Paired")
                                                                        .size(11.5)
                                                                        .color(GREEN),
                                                                );
                                                            } else if ui
                                                                .button("Pair & Connect")
                                                                .clicked()
                                                            {
                                                                self.pairing_target = Some(host);
                                                                self.pairing_code_input.clear();
                                                            }
                                                        },
                                                    );
                                                });
                                            });
                                        ui.add_space(6.0);
                                    }
                                }

                                if let Some(target) = self.pairing_target.clone() {
                                    ui.add_space(8.0);
                                    egui::Frame::new()
                                        .fill(BG_ELEVATED)
                                        .stroke(Stroke::new(1.2, ACCENT))
                                        .corner_radius(8.0)
                                        .inner_margin(12.0)
                                        .show(ui, |ui| {
                                            ui.label(
                                                RichText::new(format!(
                                                    "Enter pairing code from {}:",
                                                    target.name
                                                ))
                                                .strong()
                                                .color(TEXT_PRIMARY),
                                            );
                                            ui.add_space(6.0);
                                            ui.horizontal(|ui| {
                                                let resp = ui.add(
                                                    egui::TextEdit::singleline(
                                                        &mut self.pairing_code_input,
                                                    )
                                                    .hint_text("XXXX-XXXX")
                                                    .desired_width(150.0),
                                                );
                                                let enter = resp.lost_focus()
                                                    && ui.input(|i| {
                                                        i.key_pressed(egui::Key::Enter)
                                                    });
                                                if (ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new("Connect")
                                                                .strong()
                                                                .color(ACCENT_TEXT_DARK),
                                                        )
                                                        .fill(ACCENT),
                                                    )
                                                    .clicked()
                                                    || enter)
                                                    && !self.pairing_code_input.trim().is_empty()
                                                {
                                                    let code =
                                                        self.pairing_code_input.trim().to_string();
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
                    });
                },
            );

            ui.add_space(gap);

            // RIGHT CARD: Saved Devices & Manual IP
            ui.allocate_ui_with_layout(
                Vec2::new(right_w, card_h),
                Layout::top_down(Align::Min),
                |ui| {
                    card_frame().show(ui, |ui| {
                        ui.set_min_height(card_h - 32.0);
                        ui.set_width(ui.available_width());

                        ui.label(
                            RichText::new("Saved Devices")
                                .size(15.5)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                        ui.label(
                            RichText::new("Auto-reconnect uses your most recent paired PC.")
                                .size(12.0)
                                .color(TEXT_MUTED),
                        );
                        ui.add_space(8.0);

                        if self.config.paired_hosts.is_empty() {
                            ui.label(
                                RichText::new("No saved devices yet.")
                                    .size(12.0)
                                    .color(TEXT_MUTED),
                            );
                        } else {
                            let list = self.config.paired_hosts.clone();
                            for saved in list {
                                egui::Frame::new()
                                    .fill(BG_RECESSED)
                                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                                    .corner_radius(6.0)
                                    .inner_margin(8.0)
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            ui.vertical(|ui| {
                                                ui.label(
                                                    RichText::new(&saved.name)
                                                        .strong()
                                                        .color(TEXT_PRIMARY),
                                                );
                                                ui.label(
                                                    RichText::new(&saved.addr)
                                                        .monospace()
                                                        .size(11.0)
                                                        .color(TEXT_MUTED),
                                                );
                                            });
                                            ui.with_layout(
                                                Layout::right_to_left(Align::Center),
                                                |ui| {
                                                    if ui.small_button("Forget").clicked() {
                                                        self.config
                                                            .remove_paired_host(&saved.fingerprint);
                                                        self.save_config();
                                                    }
                                                    if ui.small_button("Connect").clicked() {
                                                        self.connect_to_saved(&saved);
                                                    }
                                                },
                                            );
                                        });
                                    });
                                ui.add_space(4.0);
                            }
                        }

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(8.0);

                        if ui
                            .selectable_label(self.manual_open, "Manual IP Connection")
                            .clicked()
                        {
                            self.manual_open = !self.manual_open;
                        }

                        if self.manual_open {
                            ui.add_space(8.0);
                            egui::Grid::new("manual_grid")
                                .num_columns(2)
                                .spacing([8.0, 6.0])
                                .show(ui, |ui| {
                                    ui.label("Host IP:");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.manual_addr)
                                            .hint_text("192.168.1.50:24910")
                                            .desired_width(165.0),
                                    );
                                    ui.end_row();

                                    ui.label("Code:");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.manual_secret)
                                            .hint_text("XXXX-XXXX")
                                            .desired_width(165.0),
                                    );
                                    ui.end_row();

                                    ui.label("Cert FP:");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.manual_fingerprint)
                                            .hint_text("Optional if TOFU")
                                            .desired_width(165.0),
                                    );
                                    ui.end_row();
                                });
                            ui.add_space(4.0);
                            ui.checkbox(
                                &mut self.manual_tofu,
                                "Trust cert on first connect (TOFU)",
                            );
                            ui.add_space(6.0);
                            if ui
                                .add_sized(
                                    [ui.available_width(), 32.0],
                                    egui::Button::new(
                                        RichText::new("Connect Manually")
                                            .strong()
                                            .color(ACCENT_TEXT_DARK),
                                    )
                                    .fill(ACCENT),
                                )
                                .clicked()
                            {
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
                },
            );
        });
    }

    fn draw_settings_tab(&mut self, ui: &mut egui::Ui) {
        let avail_w = ui.available_width();
        let gap = 14.0;
        let col_w = ((avail_w - gap) / 2.0).floor();
        let card_h = ui.available_height().max(380.0);

        ui.horizontal_top(|ui| {
            // LEFT SETTINGS CARD: General & Input
            ui.allocate_ui_with_layout(
                Vec2::new(col_w, card_h),
                Layout::top_down(Align::Min),
                |ui| {
                    card_frame().show(ui, |ui| {
                        ui.set_min_height(card_h - 32.0);
                        ui.set_width(ui.available_width());

                        ui.label(
                            RichText::new("General & Input")
                                .size(15.5)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                        ui.add_space(10.0);

                        let mut changed = false;
                        ui.label(RichText::new("This PC's Name").size(12.0).color(TEXT_MUTED));
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut self.config.device_name)
                                    .desired_width(ui.available_width()),
                            )
                            .changed()
                        {
                            changed = true;
                        }

                        ui.add_space(10.0);
                        ui.label(RichText::new("Switch Hotkey").size(12.0).color(TEXT_MUTED));
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.hotkey_draft)
                                    .desired_width(180.0),
                            );
                            if ui.button("Apply").clicked() {
                                match Hotkey::parse(&self.hotkey_draft) {
                                    Ok(_) => {
                                        self.config.hotkey = self.hotkey_draft.trim().to_string();
                                        self.save_config();
                                        self.notice = Some((
                                            "Hotkey saved (restart Host sharing if active)."
                                                .into(),
                                            false,
                                        ));
                                    }
                                    Err(e) => {
                                        self.notice = Some((format!("Invalid hotkey: {e}"), true));
                                    }
                                }
                            }
                        });

                        ui.add_space(12.0);
                        if ui
                            .checkbox(
                                &mut self.config.clipboard_sync,
                                "Sync Clipboard Text (Encrypted)",
                            )
                            .changed()
                        {
                            changed = true;
                            match &self.running {
                                Some(RunningEngine::Host { cmd_tx, .. }) => {
                                    let _ = cmd_tx.try_send(HostCommand::SetClipboardSync(
                                        self.config.clipboard_sync,
                                    ));
                                }
                                Some(RunningEngine::Client { cmd_tx, .. }) => {
                                    let _ = cmd_tx.try_send(ClientCommand::SetClipboardSync(
                                        self.config.clipboard_sync,
                                    ));
                                }
                                None => {}
                            }
                        }

                        ui.add_space(4.0);
                        if ui
                            .checkbox(
                                &mut self.config.auto_connect,
                                "Auto-Reconnect (and auto-connect in Client mode)",
                            )
                            .changed()
                        {
                            changed = true;
                        }

                        ui.add_space(12.0);
                        let hz = 1000 / self.config.coalesce_ms.max(1);
                        ui.label(
                            RichText::new(format!(
                                "Mouse Coalescing: {} ms ({hz} Hz)",
                                self.config.coalesce_ms
                            ))
                            .size(12.5)
                            .color(TEXT_PRIMARY),
                        );
                        if ui
                            .add(egui::Slider::new(&mut self.config.coalesce_ms, 1..=16).show_value(false))
                            .changed()
                        {
                            changed = true;
                        }
                        ui.label(
                            RichText::new("Lower = ultra responsive · 4–8 ms smooth over Wi-Fi")
                                .size(11.0)
                                .color(TEXT_MUTED),
                        );

                        if changed {
                            self.save_config();
                        }
                    });
                },
            );

            ui.add_space(gap);

            // RIGHT SETTINGS CARD: Network, Security & System
            ui.allocate_ui_with_layout(
                Vec2::new(col_w, card_h),
                Layout::top_down(Align::Min),
                |ui| {
                    card_frame().show(ui, |ui| {
                        ui.set_min_height(card_h - 32.0);
                        ui.set_width(ui.available_width());

                        ui.label(
                            RichText::new("Security & System")
                                .size(15.5)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                        ui.add_space(10.0);

                        info_row(ui, "Transport", "QUIC + TLS 1.3 (UDP 24910)");
                        ui.add_space(6.0);
                        info_row(ui, "Discovery", "Local mDNS (_deskglide._udp)");
                        ui.add_space(6.0);
                        let cfg_path = AppConfig::default_path().display().to_string();
                        ui.label(RichText::new("Config File").size(12.0).color(TEXT_MUTED));
                        ui.label(
                            RichText::new(&cfg_path)
                                .monospace()
                                .size(11.0)
                                .color(TEXT_PRIMARY),
                        );

                        if let Some(id) = &self.identity {
                            ui.add_space(10.0);
                            let fp = id.fingerprint_str();
                            ui.label(
                                RichText::new("SHA-256 Certificate Fingerprint")
                                    .size(12.0)
                                    .color(TEXT_MUTED),
                            );
                            ui.label(
                                RichText::new(&fp)
                                    .monospace()
                                    .size(10.5)
                                    .color(TEXT_PRIMARY),
                            );
                            ui.add_space(4.0);
                            if ui.button("Copy Fingerprint").clicked() {
                                ui.ctx().copy_text(fp);
                                self.notice = Some(("Fingerprint copied.".into(), false));
                            }
                        }

                        ui.add_space(14.0);
                        #[cfg(target_os = "windows")]
                        if ui
                            .add_sized(
                                [ui.available_width(), 32.0],
                                egui::Button::new("Allow Windows Firewall (UDP 24910)"),
                            )
                            .clicked()
                        {
                            match open_windows_firewall_uac(self.config.port) {
                                Ok(()) => {
                                    self.notice = Some((
                                        "Firewall prompt opened — click Yes on UAC.".into(),
                                        false,
                                    ));
                                }
                                Err(e) => {
                                    self.notice =
                                        Some((format!("Firewall helper failed: {e}"), true));
                                }
                            }
                        }

                        ui.add_space(12.0);
                        if ui
                            .add_sized(
                                [ui.available_width(), 34.0],
                                egui::Button::new(RichText::new("Quit DeskGlide").strong().color(RED))
                                    .fill(DANGER_BG)
                                    .stroke(Stroke::new(1.0, RED)),
                            )
                            .clicked()
                        {
                            self.force_quit = true;
                            self.stop_engine();
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                },
            );
        });
    }

    #[cfg(target_os = "linux")]
    fn draw_linux_permissions_banner(&mut self, ui: &mut egui::Ui) {
        if self.uinput_writable {
            return;
        }
        egui::Frame::new()
            .fill(Color32::from_rgb(56, 38, 12))
            .stroke(Stroke::new(1.0, AMBER))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.colored_label(
                        AMBER,
                        RichText::new("Linux input permissions required for /dev/uinput").strong(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Re-check").clicked() {
                            self.uinput_writable = check_uinput_writable();
                        }
                        if ui.button("Enable Permissions (1-Click)").clicked() {
                            match run_linux_pkexec_setup() {
                                Ok(()) => {
                                    self.uinput_writable = check_uinput_writable();
                                    self.notice = Some((
                                        if self.uinput_writable {
                                            "Linux input permissions enabled!".into()
                                        } else {
                                            "Rule installed! Log out and back in once if Host mode still needs 'input' group.".into()
                                        },
                                        false,
                                    ));
                                }
                                Err(e) => {
                                    self.notice =
                                        Some((format!("Permission setup failed: {e}"), true));
                                }
                            }
                        }
                    });
                });
            });
        ui.add_space(10.0);
    }
}

/// Interactive mathematically centered monitor arrangement diagram.
fn draw_monitor_diagram(ui: &mut egui::Ui, current: ScreenEdge) -> Option<ScreenEdge> {
    let w = ui.available_width().max(260.0);
    let h = 236.0;
    let (canvas, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let center = canvas.center();

    let center_size = Vec2::new(92.0, 56.0);
    let side_size = Vec2::new(76.0, 46.0);
    let gap = 20.0;

    let this_rect = Rect::from_center_size(center, center_size);
    let slots = [
        (
            ScreenEdge::Left,
            "Left",
            Rect::from_center_size(
                Pos2::new(
                    center.x - center_size.x / 2.0 - gap - side_size.x / 2.0,
                    center.y,
                ),
                side_size,
            ),
        ),
        (
            ScreenEdge::Right,
            "Right",
            Rect::from_center_size(
                Pos2::new(
                    center.x + center_size.x / 2.0 + gap + side_size.x / 2.0,
                    center.y,
                ),
                side_size,
            ),
        ),
        (
            ScreenEdge::Top,
            "Top",
            Rect::from_center_size(
                Pos2::new(
                    center.x,
                    center.y - center_size.y / 2.0 - gap - side_size.y / 2.0,
                ),
                side_size,
            ),
        ),
        (
            ScreenEdge::Bottom,
            "Bottom",
            Rect::from_center_size(
                Pos2::new(
                    center.x,
                    center.y + center_size.y / 2.0 + gap + side_size.y / 2.0,
                ),
                side_size,
            ),
        ),
    ];

    // Check clicks first before painting
    let mut clicked: Option<ScreenEdge> = None;
    let mut hovered_edge: Option<ScreenEdge> = None;
    for (edge, _, rect) in &slots {
        let resp = ui.interact(
            *rect,
            ui.scope_id().with(("edge_slot", *edge as u8)),
            Sense::click(),
        );
        if resp.hovered() {
            hovered_edge = Some(*edge);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            clicked = Some(*edge);
        }
    }

    let painter = ui.painter();

    // Recessed backdrop grid box
    painter.rect_filled(canvas, 10.0, BG_RECESSED);
    painter.rect_stroke(
        canvas,
        10.0,
        Stroke::new(1.0, BORDER_SUBTLE),
        StrokeKind::Inside,
    );

    // Paint the 4 selectable monitor slots
    for (edge, label, rect) in &slots {
        let selected = current == *edge;
        let hovered = hovered_edge == Some(*edge);

        let (fill, stroke, text_col) = if selected {
            (ACCENT_TINT, Stroke::new(1.8, ACCENT), ACCENT_LIGHT)
        } else if hovered {
            (BG_ELEVATED, Stroke::new(1.2, BORDER_HOVER), TEXT_PRIMARY)
        } else {
            (
                Color32::from_rgb(17, 25, 39),
                Stroke::new(1.0, BORDER_WIDGET),
                TEXT_MUTED,
            )
        };

        painter.rect_filled(*rect, 8.0, fill);
        painter.rect_stroke(*rect, 8.0, stroke, StrokeKind::Inside);

        // Tiny monitor stand line under slot
        let stand_y = rect.bottom() + 3.0;
        painter.line_segment(
            [
                Pos2::new(rect.center().x - 10.0, stand_y),
                Pos2::new(rect.center().x + 10.0, stand_y),
            ],
            Stroke::new(2.0, if selected { ACCENT } else { BORDER_WIDGET }),
        );

        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            *label,
            FontId::proportional(12.5),
            text_col,
        );
    }

    // Paint center THIS PC monitor
    painter.rect_filled(this_rect, 8.0, BG_ELEVATED);
    painter.rect_stroke(
        this_rect,
        8.0,
        Stroke::new(1.5, BORDER_HOVER),
        StrokeKind::Inside,
    );
    painter.line_segment(
        [
            Pos2::new(this_rect.center().x - 14.0, this_rect.bottom() + 4.0),
            Pos2::new(this_rect.center().x + 14.0, this_rect.bottom() + 4.0),
        ],
        Stroke::new(2.5, TEXT_MUTED),
    );
    painter.text(
        Pos2::new(this_rect.center().x, this_rect.center().y - 6.0),
        Align2::CENTER_CENTER,
        "THIS PC",
        FontId::proportional(12.5),
        TEXT_PRIMARY,
    );
    painter.text(
        Pos2::new(this_rect.center().x, this_rect.center().y + 9.0),
        Align2::CENTER_CENTER,
        "Host",
        FontId::proportional(10.5),
        TEXT_MUTED,
    );

    // Paint glowing directional arrow in the gap toward the selected edge
    if current != ScreenEdge::None {
        let (a_start, a_end, head1, head2) = match current {
            ScreenEdge::Left => {
                let y = center.y;
                let s = Pos2::new(this_rect.left() - 3.0, y);
                let e = Pos2::new(this_rect.left() - gap + 4.0, y);
                (
                    s,
                    e,
                    Pos2::new(e.x + 5.0, y - 4.0),
                    Pos2::new(e.x + 5.0, y + 4.0),
                )
            }
            ScreenEdge::Right => {
                let y = center.y;
                let s = Pos2::new(this_rect.right() + 3.0, y);
                let e = Pos2::new(this_rect.right() + gap - 4.0, y);
                (
                    s,
                    e,
                    Pos2::new(e.x - 5.0, y - 4.0),
                    Pos2::new(e.x - 5.0, y + 4.0),
                )
            }
            ScreenEdge::Top => {
                let x = center.x;
                let s = Pos2::new(x, this_rect.top() - 3.0);
                let e = Pos2::new(x, this_rect.top() - gap + 4.0);
                (
                    s,
                    e,
                    Pos2::new(x - 4.0, e.y + 5.0),
                    Pos2::new(x + 4.0, e.y + 5.0),
                )
            }
            ScreenEdge::Bottom => {
                let x = center.x;
                let s = Pos2::new(x, this_rect.bottom() + 6.0);
                let e = Pos2::new(x, this_rect.bottom() + gap - 3.0);
                (
                    s,
                    e,
                    Pos2::new(x - 4.0, e.y - 5.0),
                    Pos2::new(x + 4.0, e.y - 5.0),
                )
            }
            ScreenEdge::None => unreachable!(),
        };
        let arrow_stroke = Stroke::new(2.0, ACCENT);
        painter.line_segment([a_start, a_end], arrow_stroke);
        painter.line_segment([head1, a_end], arrow_stroke);
        painter.line_segment([head2, a_end], arrow_stroke);
    }

    clicked
}

fn nav_pill(ui: &mut egui::Ui, label: &str, selected: bool) -> bool {
    let text = RichText::new(label)
        .size(13.0)
        .strong()
        .color(if selected { ACCENT_LIGHT } else { TEXT_MUTED });
    let btn = egui::Button::new(text)
        .min_size(Vec2::new(72.0, 26.0))
        .corner_radius(6.0)
        .fill(if selected {
            ACCENT_TINT
        } else {
            Color32::TRANSPARENT
        })
        .stroke(if selected {
            Stroke::new(1.0, ACCENT)
        } else {
            Stroke::NONE
        });
    ui.add(btn).clicked()
}

fn info_row(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(key).size(12.0).color(TEXT_MUTED));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(value)
                    .monospace()
                    .size(12.0)
                    .color(TEXT_PRIMARY),
            );
        });
    });
}

fn status_badge(status: &EngineStatus) -> (Color32, String) {
    match status {
        EngineStatus::Idle => (TEXT_MUTED, "Idle".into()),
        EngineStatus::Listening { .. } => (ACCENT, "Listening".into()),
        EngineStatus::Connecting { attempt, .. } => (AMBER, format!("Connecting #{attempt}")),
        EngineStatus::Connected {
            controlling_remote,
            rtt_ms,
            ..
        } => {
            let base = if *controlling_remote {
                "Remote Active"
            } else {
                "Connected"
            };
            let txt = match rtt_ms {
                Some(r) => format!("{base} · {r} ms"),
                None => base.to_string(),
            };
            (GREEN, txt)
        }
        EngineStatus::Error(_) => (RED, "Error".into()),
    }
}

fn card_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(BG_CARD)
        .stroke(Stroke::new(1.0, BORDER_SUBTLE))
        .corner_radius(12.0)
        .inner_margin(16.0)
}

fn apply_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG_CANVAS;
    v.window_fill = BG_CARD;
    v.extreme_bg_color = BG_RECESSED;
    v.faint_bg_color = BG_ELEVATED;

    v.widgets.noninteractive.bg_fill = BG_CARD;
    v.widgets.noninteractive.weak_bg_fill = BG_CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER_SUBTLE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    v.widgets.noninteractive.corner_radius = CornerRadius::same(8);

    v.widgets.inactive.bg_fill = BG_ELEVATED;
    v.widgets.inactive.weak_bg_fill = BG_ELEVATED;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER_WIDGET);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_PRIMARY);
    v.widgets.inactive.corner_radius = CornerRadius::same(7);

    v.widgets.hovered.bg_fill = BG_HOVER;
    v.widgets.hovered.weak_bg_fill = BG_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, BORDER_HOVER);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.hovered.corner_radius = CornerRadius::same(7);

    v.widgets.active.bg_fill = Color32::from_rgb(51, 65, 85);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(51, 65, 85);
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.corner_radius = CornerRadius::same(7);

    v.selection.bg_fill = ACCENT_TINT;
    v.selection.stroke = Stroke::new(1.5, ACCENT);

    style.visuals = v;
    ctx.set_style_of(egui::Theme::Dark, style);
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
