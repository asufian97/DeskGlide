#![cfg_attr(all(not(debug_assertions), target_os = "windows"), windows_subsystem = "windows")]

mod app;
mod icon;
mod tray;

use std::sync::Arc;

use eframe::egui;
use tracing_subscriber::EnvFilter;

fn main() -> eframe::Result<()> {
    // Allow smoke checks (`deskglide-app --help` / `--version`) without opening a GUI window.
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!(
                    "DeskGlide Desktop App (deskglide-app {})\n\
                     Double-click or run without arguments to open the GUI & tray app.",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            "-V" | "--version" => {
                println!("deskglide-app {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            _ => {}
        }
    }

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("deskglide")
            .with_title("DeskGlide")
            .with_inner_size([780.0, 540.0])
            .with_min_inner_size([700.0, 480.0])
            .with_icon(Arc::new(icon::egui_icon())),
        ..Default::default()
    };

    let res = eframe::run_native(
        "DeskGlide",
        native_options,
        Box::new(|cc| Ok(Box::new(app::DeskGlideApp::new(cc)))),
    );
    if let Err(ref e) = res {
        let msg = format!("DeskGlide failed to start: {e}\n");
        eprint!("{msg}");
        let log_dir = deskglide_net::config_dir();
        let _ = std::fs::create_dir_all(&log_dir);
        let _ = std::fs::write(log_dir.join("startup-error.log"), &msg);
    }
    res
}
