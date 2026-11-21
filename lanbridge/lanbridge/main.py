"""CLI entrypoint for launching the LanBridge Host Agent."""

from __future__ import annotations

import argparse
import asyncio
import logging
import socket
import sys
from pathlib import Path
from typing import Optional

from aiohttp import web

from lanbridge.capture import detect_display_server
from lanbridge.config import cert_dir, generate_pin, load_config, save_config
from lanbridge.media import probe_best_encoder
from lanbridge.security import detect_lan_ipv4s, ensure_tls_cert, is_private_lan_ip
from lanbridge.signaling import build_ssl_context, create_app


def parse_args(argv: Optional[list[str]] = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="lanbridge",
        description="LanBridge — zero-cloud LAN WebRTC Remote Desktop & Hybrid KVM Host",
    )
    parser.add_argument("--port", type=int, default=None, help="HTTPS/WSS port (default: 8443)")
    parser.add_argument(
        "--bind",
        default="0.0.0.0",
        help="Bind address (private LAN IP or 0.0.0.0 with LAN-only request guard)",
    )
    parser.add_argument("--pin", default=None, help="Override 6-digit pairing PIN")
    parser.add_argument("--rotate-pin", action="store_true", help="Generate a new 6-digit PIN")
    parser.add_argument(
        "--quality",
        choices=("crisp", "balanced", "low"),
        default=None,
        help="Initial video quality preset",
    )
    parser.add_argument("--no-clipboard", action="store_true", help="Disable clipboard sync")
    parser.add_argument("--host-ip", default=None, help="Preferred LAN IPv4 displayed at startup")
    return parser.parse_args(argv)


async def _serve(args: argparse.Namespace) -> None:
    cfg = load_config()
    changed = False

    if args.port is not None:
        cfg.port = int(args.port)
        changed = True
    if args.pin is not None:
        if len(args.pin) != 6 or not args.pin.isdigit():
            raise SystemExit("ERROR: --pin must be a 6-digit number (e.g. 482910)")
        cfg.pin = args.pin
        changed = True
    elif args.rotate_pin:
        cfg.pin = generate_pin()
        changed = True
    if args.quality is not None:
        cfg.quality = args.quality
        changed = True
    if args.no_clipboard:
        cfg.clipboard_sync = False
        changed = True
    if changed:
        save_config(cfg)

    lan_ips = detect_lan_ipv4s()
    if args.host_ip:
        if not is_private_lan_ip(args.host_ip):
            raise SystemExit(f"ERROR: --host-ip '{args.host_ip}' is not a private LAN IP.")
        if args.host_ip not in lan_ips:
            lan_ips.insert(0, args.host_ip)

    cert_path, key_path, fingerprint = ensure_tls_cert(cert_dir(), lan_ips)
    ssl_ctx = build_ssl_context(cert_path, key_path)

    display_server = detect_display_server()
    encoder = probe_best_encoder(dry_run=True)
    enc_label = (
        f"{encoder.element} ({'GPU hardware' if encoder.hardware else 'software'})"
        if encoder
        else "NONE DETECTED (run doctor.py)"
    )

    hostname = socket.gethostname()
    primary_ip = lan_ips[0] if lan_ips else "127.0.0.1"
    host_info = {
        "hostname": hostname,
        "display_server": display_server,
        "encoder": encoder.element if encoder else "none",
        "hardware": bool(encoder and encoder.hardware),
        "quality": cfg.quality,
    }

    async def handle_ws(ws: web.WebSocketResponse, conf) -> None:
        from lanbridge.session import default_session_handler

        await default_session_handler(ws, conf, encoder=encoder)

    app = create_app(cfg, session_handler=handle_ws, host_info=host_info)
    runner = web.AppRunner(app, access_log=None)
    await runner.setup()

    bind_host = args.bind.strip()
    if bind_host != "0.0.0.0" and not is_private_lan_ip(bind_host):
        raise SystemExit(f"ERROR: --bind '{bind_host}' must be 0.0.0.0 or a private LAN IP.")

    site = web.TCPSite(runner, host=bind_host, port=cfg.port, ssl_context=ssl_ctx)
    await site.start()

    print("=" * 64)
    print(f" LanBridge Host Agent  ({hostname} · {display_server})")
    print("=" * 64)
    print(f" Viewer URL   : https://{primary_ip}:{cfg.port}")
    for extra in lan_ips[1:]:
        print(f" Alternate    : https://{extra}:{cfg.port}")
    print(f" Pairing PIN  : {cfg.pin}")
    print(f" H.264 Codec  : {enc_label}")
    print(f" TLS SHA-256  : {fingerprint}")
    print("=" * 64)
    print(" Open the Viewer URL in Chrome/Edge on your desktop PC (Ctrl+C to stop).")
    sys.stdout.flush()

    stop_event = asyncio.Event()
    try:
        while not stop_event.is_set():
            await asyncio.sleep(3600)
    finally:
        await runner.cleanup()


def main(argv: Optional[list[str]] = None) -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="[%(asctime)s] %(levelname)s %(name)s: %(message)s",
    )
    args = parse_args(argv)
    try:
        asyncio.run(_serve(args))
    except KeyboardInterrupt:
        print("\nLanBridge stopped cleanly.")


if __name__ == "__main__":
    main()
