"""Cross-platform local clipboard reader/writer with SHA-256 loop suppression."""

from __future__ import annotations

import ctypes
import hashlib
import shutil
import subprocess
import sys
from ctypes import wintypes
from typing import Optional

from lanbridge.capture import detect_display_server
from lanbridge.protocol import MAX_CLIPBOARD_BYTES

CF_UNICODETEXT = 13
GMEM_MOVEABLE = 0x0002


def _hash_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _win_get_clipboard() -> Optional[str]:
    user32 = ctypes.windll.user32  # type: ignore[attr-defined]
    kernel32 = ctypes.windll.kernel32  # type: ignore[attr-defined]
    if not user32.OpenClipboard(None):
        return None
    try:
        handle = user32.GetClipboardData(CF_UNICODETEXT)
        if not handle:
            return None
        ptr = kernel32.GlobalLock(handle)
        if not ptr:
            return None
        try:
            return ctypes.wstring_at(ptr)
        finally:
            kernel32.GlobalUnlock(handle)
    finally:
        user32.CloseClipboard()


def _win_set_clipboard(text: str) -> bool:
    user32 = ctypes.windll.user32  # type: ignore[attr-defined]
    kernel32 = ctypes.windll.kernel32  # type: ignore[attr-defined]
    data = (text + "\0").encode("utf-16-le")
    hmem = kernel32.GlobalAlloc(GMEM_MOVEABLE, len(data))
    if not hmem:
        return False
    ptr = kernel32.GlobalLock(hmem)
    if not ptr:
        kernel32.GlobalFree(hmem)
        return False
    ctypes.memmove(ptr, data, len(data))
    kernel32.GlobalUnlock(hmem)

    if not user32.OpenClipboard(None):
        kernel32.GlobalFree(hmem)
        return False
    try:
        user32.EmptyClipboard()
        if not user32.SetClipboardData(CF_UNICODETEXT, hmem):
            kernel32.GlobalFree(hmem)
            return False
        return True
    finally:
        user32.CloseClipboard()


class ClipboardBridge:
    """Polls local clipboard safely without echoing remote updates back."""

    def __init__(self, display_server: Optional[str] = None) -> None:
        self._display = display_server or detect_display_server()
        self._last_hash: Optional[str] = None
        current = self._read_raw()
        if current is not None:
            self._last_hash = _hash_text(current)

    def _read_raw(self) -> Optional[str]:
        try:
            if sys.platform.startswith("win"):
                return _win_get_clipboard()
            if self._display == "wayland" and shutil.which("wl-paste"):
                res = subprocess.run(
                    ["wl-paste", "--no-newline", "--type", "text/plain"],
                    capture_output=True,
                    text=True,
                    timeout=1.5,
                )
                return res.stdout if res.returncode == 0 else None
            if shutil.which("xclip"):
                res = subprocess.run(
                    ["xclip", "-selection", "clipboard", "-o"],
                    capture_output=True,
                    text=True,
                    timeout=1.5,
                )
                return res.stdout if res.returncode == 0 else None
        except Exception:
            return None
        return None

    def set_remote_text(self, text: str) -> None:
        """Apply incoming remote clipboard text while recording hash to prevent echo."""
        encoded = text.encode("utf-8")
        if len(encoded) > MAX_CLIPBOARD_BYTES:
            text = encoded[:MAX_CLIPBOARD_BYTES].decode("utf-8", errors="ignore")
        self._last_hash = _hash_text(text)
        try:
            if sys.platform.startswith("win"):
                _win_set_clipboard(text)
            elif self._display == "wayland" and shutil.which("wl-copy"):
                subprocess.run(["wl-copy"], input=text, text=True, timeout=1.5)
            elif shutil.which("xclip"):
                subprocess.run(
                    ["xclip", "-selection", "clipboard", "-i"],
                    input=text,
                    text=True,
                    timeout=1.5,
                )
        except Exception:
            pass

    def poll_local_change(self) -> Optional[str]:
        """Return new clipboard text if changed locally since last poll or remote write."""
        text = self._read_raw()
        if not text:
            return None
        encoded = text.encode("utf-8")
        if len(encoded) > MAX_CLIPBOARD_BYTES:
            text = encoded[:MAX_CLIPBOARD_BYTES].decode("utf-8", errors="ignore")
        h = _hash_text(text)
        if h == self._last_hash:
            return None
        self._last_hash = h
        return text
