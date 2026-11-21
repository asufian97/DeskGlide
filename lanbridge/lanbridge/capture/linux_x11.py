"""X11 screen capture descriptor using GStreamer `ximagesrc`."""

from __future__ import annotations

import os
import re
import subprocess

from lanbridge.capture.base import CaptureBackend, CaptureDescriptor


def detect_x11_resolution() -> tuple[int, int]:
    """Query primary X11 screen resolution via xrandr/xdpyinfo, defaulting to 1920x1080."""
    for cmd in (["xrandr", "--current"], ["xdpyinfo"]):
        try:
            out = subprocess.check_output(cmd, text=True, stderr=subprocess.DEVNULL, timeout=2)
            m = re.search(r"current\s+(\d+)\s*x\s*(\d+)", out)
            if m:
                return max(1, int(m.group(1))), max(1, int(m.group(2)))
            m = re.search(r"dimensions:\s+(\d+)x(\d+)\s+pixels", out)
            if m:
                return max(1, int(m.group(1))), max(1, int(m.group(2)))
        except Exception:
            continue
    return (1920, 1080)


class LinuxX11CaptureBackend(CaptureBackend):
    """Captures X11 root desktop via GStreamer `ximagesrc`."""

    def start(self) -> CaptureDescriptor:
        w, h = detect_x11_resolution()
        display = os.environ.get("DISPLAY", ":0")
        return CaptureDescriptor(
            backend="linux_x11",
            display_server="x11",
            width=w,
            height=h,
            gst_source_name="ximagesrc",
            extra_props={
                "display-name": display,
                "use-damage": False,
                "show-pointer": True,
            },
        )

    def close(self) -> None:
        return None
