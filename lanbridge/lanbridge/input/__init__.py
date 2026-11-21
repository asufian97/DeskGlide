"""Input backend factory selecting Portal, XTest, uinput, or Win32 SendInput."""

from __future__ import annotations

import sys
from typing import Optional

from lanbridge.capture.base import CaptureBackend, CaptureDescriptor
from lanbridge.capture.linux_portal import LinuxPortalBackend
from lanbridge.input.base import HeldInputTracker, InputBackend


def create_input_backend(
    capture_backend: Optional[CaptureBackend] = None,
    descriptor: Optional[CaptureDescriptor] = None,
) -> HeldInputTracker:
    """Create the appropriate OS input backend wrapped in HeldInputTracker."""
    backend: InputBackend
    w = descriptor.width if descriptor else 1920
    h = descriptor.height if descriptor else 1080

    if sys.platform.startswith("win"):
        from lanbridge.input.windows import WindowsInputBackend

        backend = WindowsInputBackend()
        return HeldInputTracker(backend)

    if isinstance(capture_backend, LinuxPortalBackend):
        from lanbridge.input.linux_portal import LinuxPortalInputBackend

        backend = LinuxPortalInputBackend(capture_backend)
        return HeldInputTracker(backend)

    try:
        from lanbridge.input.linux_x11 import LinuxX11InputBackend

        backend = LinuxX11InputBackend(width=w, height=h)
    except Exception:
        from lanbridge.input.linux_uinput import LinuxUinputBackend

        backend = LinuxUinputBackend(width=w, height=h)

    return HeldInputTracker(backend)


__all__ = ["HeldInputTracker", "InputBackend", "create_input_backend"]
