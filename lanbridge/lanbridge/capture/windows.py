"""Windows desktop capture descriptor using GStreamer `d3d11screencapturesrc`."""

from __future__ import annotations

import ctypes

from lanbridge.capture.base import CaptureBackend, CaptureDescriptor


def detect_windows_resolution() -> tuple[int, int]:
    """Query Windows primary desktop resolution via user32 GetSystemMetrics."""
    try:
        user32 = ctypes.windll.user32  # type: ignore[attr-defined]
        try:
            user32.SetProcessDPIAware()
        except Exception:
            pass
        w = int(user32.GetSystemMetrics(0))
        h = int(user32.GetSystemMetrics(1))
        if w > 0 and h > 0:
            return (w, h)
    except Exception:
        pass
    return (1920, 1080)


class WindowsCaptureBackend(CaptureBackend):
    """Captures Windows display via `d3d11screencapturesrc`."""

    def start(self) -> CaptureDescriptor:
        w, h = detect_windows_resolution()
        return CaptureDescriptor(
            backend="windows",
            display_server="windows",
            width=w,
            height=h,
            gst_source_name="d3d11screencapturesrc",
            extra_props={"show-cursor": True},
        )

    def close(self) -> None:
        return None
