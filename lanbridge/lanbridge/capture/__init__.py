"""Capture backend factory & display server detection."""

from __future__ import annotations

import os
import sys
from typing import Callable, Optional

from lanbridge.capture.base import CaptureBackend, CaptureDescriptor


def detect_display_server() -> str:
    """Return 'wayland', 'x11', or 'windows' for the current session."""
    if sys.platform.startswith("win"):
        return "windows"
    xdg_type = os.environ.get("XDG_SESSION_TYPE", "").strip().lower()
    if xdg_type in ("wayland", "x11"):
        return xdg_type
    if os.environ.get("WAYLAND_DISPLAY"):
        return "wayland"
    if os.environ.get("DISPLAY"):
        return "x11"
    return "wayland"


def create_capture_backend(
    restore_token: Optional[str] = None,
    on_restore_token: Optional[Callable[[str], None]] = None,
) -> CaptureBackend:
    """Instantiate the best modular screen capture backend for this OS/session."""
    display = detect_display_server()
    if display == "windows":
        from lanbridge.capture.windows import WindowsCaptureBackend

        return WindowsCaptureBackend()
    if display == "wayland":
        from lanbridge.capture.linux_portal import LinuxPortalBackend

        return LinuxPortalBackend(
            restore_token=restore_token,
            on_restore_token=on_restore_token,
        )
    from lanbridge.capture.linux_x11 import LinuxX11CaptureBackend

    return LinuxX11CaptureBackend()


__all__ = ["CaptureBackend", "CaptureDescriptor", "create_capture_backend", "detect_display_server"]
