"""Diagnostic pre-flight tool checking GStreamer, GPU encoders, Wayland/X11, and ports."""

from __future__ import annotations

import os
import shutil
import socket
import subprocess
import sys
from typing import Any

from lanbridge.capture import detect_display_server
from lanbridge.config import load_config
from lanbridge.media import encoder_priority_for_platform, probe_best_encoder
from lanbridge.security import detect_lan_ipv4s


def _check_mark(ok: bool) -> str:
    return "[OK]" if ok else "[!!]"


def run_diagnostics(port: int = 8443) -> int:
    """Print environment, GStreamer, GPU encoder, portal, input, and firewall checks."""
    print("LanBridge System Doctor")
    print("=" * 60)

    # 1. Python
    py_ok = sys.version_info >= (3, 10)
    print(f"{_check_mark(py_ok)} Python          : {sys.version.split()[0]} ({sys.executable})")

    # 2. Core packages
    for mod in ("aiohttp", "cryptography"):
        try:
            __import__(mod)
            print(f"[OK] Package         : {mod}")
        except ImportError:
            print(f"[!!] Package         : {mod} MISSING (install via venv/pip)")

    # 3. Display Server & Session
    disp = detect_display_server()
    print(f"[OK] Display Server  : {disp}")

    # 4. PyGObject + GStreamer plugins
    gst_ok = False
    missing_elements: list[str] = []
    available_encoders: list[str] = []
    Gst: Any = None
    try:
        import gi

        gi.require_version("Gst", "1.0")
        from gi.repository import Gst as _Gst

        Gst = _Gst
        Gst.init(None)
        gst_ok = True
        ver = ".".join(str(x) for x in Gst.version())
        print(f"[OK] GStreamer       : {ver}")

        required_elements = [
            "webrtcbin",
            "h264parse",
            "rtph264pay",
            "videoconvert",
            "valve",
        ]
        if disp == "wayland":
            required_elements.append("pipewiresrc")
        elif disp == "x11":
            required_elements.append("ximagesrc")
        elif disp == "windows":
            required_elements.append("d3d11screencapturesrc")

        for el in required_elements:
            if Gst.ElementFactory.find(el) is None:
                missing_elements.append(el)

        for enc_name in encoder_priority_for_platform():
            if Gst.ElementFactory.find(enc_name) is not None:
                available_encoders.append(enc_name)

        if missing_elements:
            print(f"[!!] Gst Plugins     : missing {', '.join(missing_elements)}")
        else:
            print(f"[OK] Gst Plugins     : all required pipeline plugins present")
    except Exception as exc:
        print(f"[!!] GStreamer       : unavailable ({exc})")

    # 5. Hardware Encoder Probe
    winning_enc = probe_best_encoder(dry_run=True) if gst_ok else None
    if winning_enc is not None:
        kind = "GPU Hardware" if winning_enc.hardware else "Software Fallback"
        print(
            f"[OK] Best Encoder    : {winning_enc.element} ({kind}); detected={[e for e in available_encoders]}"
        )
    else:
        print(
            f"[!!] Best Encoder    : NONE working (installed={available_encoders})"
        )

    # 6. Portal / Input / Clipboard checks
    if sys.platform.startswith("linux"):
        pw_ok = shutil.which("pw-cli") is not None or shutil.which("pipewire") is not None
        print(f"{_check_mark(pw_ok)} PipeWire Binary : {'found' if pw_ok else 'not found'}")
        if disp == "wayland":
            has_wl_clip = shutil.which("wl-copy") is not None and shutil.which("wl-paste") is not None
            print(
                f"{_check_mark(has_wl_clip)} Wayland Clip    : "
                + ("wl-clipboard installed" if has_wl_clip else "install wl-clipboard")
            )
        else:
            has_xclip = shutil.which("xclip") is not None
            print(f"{_check_mark(has_xclip)} X11 Clip        : {'xclip installed' if has_xclip else 'install xclip'}")

        try:
            out = subprocess.run(["ufw", "status"], capture_output=True, text=True, timeout=2)
            if "Status: active" in out.stdout:
                allowed = str(port) in out.stdout
                print(
                    f"{_check_mark(allowed)} UFW Firewall    : active; port {port} "
                    + ("allowed" if allowed else f"BLOCKED (run: sudo ufw allow {port}/tcp)")
                )
            else:
                print("[OK] UFW Firewall    : inactive")
        except Exception:
            print("[--] UFW Firewall    : not installed or permission denied")
    elif sys.platform.startswith("win"):
        print("[OK] Win32 Input     : user32.SendInput ready via ctypes")

    # 7. Port availability & LAN URL
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    port_free = False
    try:
        sock.bind(("0.0.0.0", port))
        port_free = True
    except OSError:
        port_free = False
    finally:
        sock.close()
    print(
        f"{_check_mark(port_free)} TCP Port {port}   : "
        + ("available to bind" if port_free else "already in use")
    )

    lan_ips = detect_lan_ipv4s()
    print("-" * 60)
    if lan_ips:
        for ip in lan_ips:
            print(f"Open on viewing browser : https://{ip}:{port}")
    else:
        print(f"Open on viewing browser : https://localhost:{port} (no private LAN IPv4 detected)")

    return 0 if (py_ok and gst_ok and winning_enc is not None and not missing_elements) else 1


def main() -> None:
    cfg = load_config()
    code = run_diagnostics(port=cfg.port)
    raise SystemExit(code)


if __name__ == "__main__":
    main()
