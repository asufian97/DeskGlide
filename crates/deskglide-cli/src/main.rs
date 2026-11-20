use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use deskglide_engine::{
    detect_lan_ip, run_client, run_host, ClientCommand, ClientOptions, EngineStatus, HostCommand,
    HostOptions,
};
use deskglide_input::Hotkey;
use deskglide_net::{
    config_dir, generate_pairing_secret, load_or_create_identity, parse_fingerprint, PinPolicy,
};
use deskglide_proto::{ScreenEdge, DEFAULT_PORT};
use tokio::sync::{mpsc, watch};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "deskglide",
    version,
    about = "Free, open-source, encrypted cross-platform software KVM (Windows & Linux)"
)]
struct Cli {
    /// Enable verbose debug logging.
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run as Server (Host PC with the physical wireless keyboard & mouse).
    Server(ServerArgs),
    /// Run as Client (Controlled PC receiving encrypted keyboard & mouse events).
    Client(ClientArgs),
    /// Print this machine's persistent server TLS SHA-256 certificate fingerprint.
    Fingerprint,
}

#[derive(Args)]
struct ServerArgs {
    /// UDP address and port to listen on.
    #[arg(long, default_value_t = SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT)))]
    bind: SocketAddr,

    /// Shared pairing secret / PIN (auto-generated if omitted).
    #[arg(long, env = "DESKGLIDE_SECRET")]
    secret: Option<String>,

    /// Toggle hotkey between local and remote control (e.g. "Ctrl+Alt+S", "Ctrl+Shift+F12").
    #[arg(long, default_value = "Ctrl+Alt+S")]
    hotkey: String,

    /// Screen edge for automatic pointer switching (none, left, right, top, bottom).
    #[arg(long, default_value = "right")]
    edge: String,

    /// Mouse movement coalescing interval in milliseconds (1-50 ms).
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=50))]
    coalesce_ms: u64,

    /// Disable mDNS local-network advertisement.
    #[arg(long)]
    no_mdns: bool,

    /// Disable encrypted clipboard text synchronization.
    #[arg(long)]
    no_clipboard: bool,
}

#[derive(Args)]
struct ClientArgs {
    /// Server IP/hostname, with optional :port (defaults to :24910).
    server: String,

    /// Shared pairing secret / PIN printed by the server.
    #[arg(long, env = "DESKGLIDE_SECRET")]
    secret: String,

    /// Expected SHA-256 certificate fingerprint printed by the server.
    #[arg(long, env = "DESKGLIDE_FINGERPRINT")]
    fingerprint: Option<String>,

    /// Trust-On-First-Use: pin the server certificate into known_hosts on first connect.
    #[arg(long)]
    tofu: bool,

    /// Friendly display name for this client machine.
    #[arg(long)]
    name: Option<String>,

    /// Automatically reconnect if connection drops or server restarts.
    #[arg(long)]
    auto_reconnect: bool,

    /// Disable encrypted clipboard text synchronization.
    #[arg(long)]
    no_clipboard: bool,
}

fn init_logging(verbose: bool) {
    let default_filter = if verbose { "debug" } else { "info" };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

fn resolve_server_addr(raw: &str) -> Result<SocketAddr> {
    if let Ok(addr) = raw.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Ok(ip) = raw.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    let with_port = if raw.contains(':') {
        raw.to_string()
    } else {
        format!("{raw}:{DEFAULT_PORT}")
    };
    with_port
        .to_socket_addrs()
        .with_context(|| format!("failed to resolve server address '{raw}'"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("no socket addresses resolved for '{raw}'"))
}

fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| deskglide_proto::OsKind::current().to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    // When double-clicked with no arguments, launch the sibling desktop GUI if present.
    if std::env::args_os().len() == 1 {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let gui_name = if cfg!(target_os = "windows") {
                    "deskglide-app.exe"
                } else {
                    "deskglide-app"
                };
                let gui_path = dir.join(gui_name);
                if gui_path.exists() && std::process::Command::new(&gui_path).spawn().is_ok() {
                    return Ok(());
                }
            }
        }
    }

    let cli = Cli::parse();
    init_logging(cli.verbose);

    match cli.command {
        Command::Fingerprint => {
            let id = load_or_create_identity(None)?;
            println!("Config directory: {}", id.config_dir.display());
            println!("SHA-256 fingerprint: {}", id.fingerprint_str());
        }
        Command::Server(args) => {
            let hotkey = Hotkey::parse(&args.hotkey)?;
            let switch_edge = ScreenEdge::from_name(&args.edge)
                .ok_or_else(|| anyhow::anyhow!("invalid --edge '{}'; use none, left, right, top, or bottom", args.edge))?;
            let secret = args.secret.unwrap_or_else(generate_pairing_secret);
            let identity = load_or_create_identity(None)?;
            let lan_ip = detect_lan_ip()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "<SERVER_IP>".to_string());

            println!("=== DeskGlide Server (Host) ===");
            println!("Listening on  : {} (QUIC/UDP)", args.bind);
            println!("Hotkey        : {} ({hotkey})", args.hotkey);
            println!("Switch edge   : {switch_edge}");
            println!("Coalesce      : {} ms", args.coalesce_ms);
            println!("Pairing secret: {secret}");
            println!("Fingerprint   : {}", identity.fingerprint_str());
            println!();
            println!("Run this on your Client device:");
            println!(
                "  deskglide client {lan_ip}:{} --secret {secret} --fingerprint {}",
                args.bind.port(),
                identity.fingerprint_str()
            );
            println!();

            let (status_tx, _status_rx) = watch::channel(EngineStatus::Idle);
            let (cmd_tx, cmd_rx) = mpsc::channel(16);

            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    let _ = cmd_tx.send(HostCommand::Stop).await;
                }
            });

            run_host(
                HostOptions {
                    bind_addr: args.bind,
                    identity,
                    device_name: default_device_name(),
                    secret,
                    hotkey,
                    switch_edge,
                    coalesce_ms: args.coalesce_ms,
                    clipboard_sync: !args.no_clipboard,
                    enable_mdns: !args.no_mdns,
                },
                status_tx,
                cmd_rx,
            )
            .await?;
        }
        Command::Client(args) => {
            let server_addr = resolve_server_addr(&args.server)?;
            let pin_policy = match (&args.fingerprint, args.tofu) {
                (Some(fp_str), _) => PinPolicy::Exact(parse_fingerprint(fp_str)?),
                (None, allow_tofu) => {
                    let known_hosts_path = config_dir().join("known_hosts");
                    if !allow_tofu && !known_hosts_path.exists() {
                        bail!(
                            "no --fingerprint provided and {} does not exist yet.\n\
                             Pass --fingerprint <SHA256> from the server (recommended) or --tofu on first trusted connect.",
                            known_hosts_path.display()
                        );
                    }
                    PinPolicy::KnownOrTofu {
                        known_hosts_path,
                        peer_key: args.server.clone(),
                        allow_tofu,
                    }
                }
            };

            let name = args.name.unwrap_or_else(default_device_name);
            let (status_tx, _status_rx) = watch::channel(EngineStatus::Idle);
            let (cmd_tx, cmd_rx) = mpsc::channel(16);

            tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    let _ = cmd_tx.send(ClientCommand::Stop).await;
                }
            });

            run_client(
                ClientOptions {
                    server_addr,
                    secret: args.secret,
                    pin_policy,
                    client_name: name,
                    clipboard_sync: !args.no_clipboard,
                    auto_reconnect: args.auto_reconnect,
                },
                status_tx,
                cmd_rx,
            )
            .await?;
        }
    }

    Ok(())
}
