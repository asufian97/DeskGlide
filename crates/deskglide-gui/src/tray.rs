//! Cross-platform system tray icon (Win32 `tray-icon` on Windows, D-Bus `ksni` on Linux).

use std::sync::mpsc;

use eframe::egui;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    ShowWindow,
    Quit,
}

pub struct SystemTray {
    rx: mpsc::Receiver<TrayAction>,
    #[cfg(target_os = "windows")]
    win: Option<win_tray::WinTray>,
    #[cfg(target_os = "linux")]
    _linux: Option<ksni::blocking::Handle<linux_tray::DeskGlideTray>>,
}

impl SystemTray {
    pub fn start(repaint_ctx: egui::Context) -> Self {
        let (tx, rx) = mpsc::channel();

        #[cfg(target_os = "windows")]
        let win = match win_tray::WinTray::new(tx.clone(), repaint_ctx.clone()) {
            Ok(t) => Some(t),
            Err(e) => {
                warn!("system tray unavailable on Windows: {e:#}");
                None
            }
        };

        #[cfg(target_os = "linux")]
        let _linux = match linux_tray::spawn(tx.clone(), repaint_ctx.clone()) {
            Ok(h) => Some(h),
            Err(e) => {
                warn!("system tray unavailable on Linux: {e:#}");
                None
            }
        };

        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        let _ = (tx, repaint_ctx);

        Self {
            rx,
            #[cfg(target_os = "windows")]
            win,
            #[cfg(target_os = "linux")]
            _linux,
        }
    }

    pub fn is_available(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            self.win.is_some()
        }
        #[cfg(target_os = "linux")]
        {
            self._linux.is_some()
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            false
        }
    }

    pub fn poll_action(&mut self) -> Option<TrayAction> {
        self.rx.try_recv().ok()
    }
}

#[cfg(target_os = "windows")]
mod win_tray {
    use super::*;
    use anyhow::Result;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use crate::icon::{rgba_32x32, ICON_SIZE};

    pub struct WinTray {
        _tray: TrayIcon,
    }

    impl WinTray {
        pub fn new(tx: mpsc::Sender<TrayAction>, repaint_ctx: egui::Context) -> Result<Self> {
            let menu = Menu::new();
            let show_item = MenuItem::new("Show DeskGlide", true, None);
            let quit_item = MenuItem::new("Quit", true, None);
            let show_id = show_item.id().clone();
            let quit_id = quit_item.id().clone();
            menu.append(&show_item)?;
            menu.append(&quit_item)?;

            let icon = Icon::from_rgba(rgba_32x32(), ICON_SIZE as u32, ICON_SIZE as u32)?;
            let tray = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("DeskGlide — Encrypted Software KVM")
                .with_icon(icon)
                .build()?;

            let tx_menu = tx.clone();
            let ctx_menu = repaint_ctx.clone();
            MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
                if ev.id() == &show_id {
                    let _ = tx_menu.send(TrayAction::ShowWindow);
                    ctx_menu.request_repaint();
                } else if ev.id() == &quit_id {
                    let _ = tx_menu.send(TrayAction::Quit);
                    ctx_menu.request_repaint();
                }
            }));

            TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| {
                if matches!(
                    ev,
                    TrayIconEvent::Click { .. } | TrayIconEvent::DoubleClick { .. }
                ) {
                    let _ = tx.send(TrayAction::ShowWindow);
                    repaint_ctx.request_repaint();
                }
            }));

            Ok(Self { _tray: tray })
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_tray {
    use super::*;
    use ksni::blocking::TrayMethods;
    use ksni::menu::StandardItem;

    use crate::icon::{rgba_32x32, ICON_SIZE};

    pub struct DeskGlideTray {
        tx: mpsc::Sender<TrayAction>,
        repaint_ctx: egui::Context,
        icon: ksni::Icon,
    }

    impl ksni::Tray for DeskGlideTray {
        fn id(&self) -> String {
            "deskglide".into()
        }

        fn title(&self) -> String {
            "DeskGlide".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            vec![self.icon.clone()]
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            let _ = self.tx.send(TrayAction::ShowWindow);
            self.repaint_ctx.request_repaint();
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            vec![
                StandardItem {
                    label: "Show DeskGlide".into(),
                    activate: Box::new(|this: &mut Self| {
                        let _ = this.tx.send(TrayAction::ShowWindow);
                        this.repaint_ctx.request_repaint();
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Quit".into(),
                    activate: Box::new(|this: &mut Self| {
                        let _ = this.tx.send(TrayAction::Quit);
                        this.repaint_ctx.request_repaint();
                    }),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    pub fn spawn(
        tx: mpsc::Sender<TrayAction>,
        repaint_ctx: egui::Context,
    ) -> Result<ksni::blocking::Handle<DeskGlideTray>, ksni::Error> {
        let rgba = rgba_32x32();
        let mut argb = Vec::with_capacity(rgba.len());
        for px in rgba.chunks_exact(4) {
            argb.push(px[3]);
            argb.push(px[0]);
            argb.push(px[1]);
            argb.push(px[2]);
        }
        let icon = ksni::Icon {
            width: ICON_SIZE as i32,
            height: ICON_SIZE as i32,
            data: argb,
        };
        DeskGlideTray {
            tx,
            repaint_ctx,
            icon,
        }
        .spawn()
    }
}
