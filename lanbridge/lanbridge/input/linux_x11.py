"""X11 `libXtst` (XTest) input injection backend using ctypes (zero C compilation)."""

from __future__ import annotations

import ctypes
import ctypes.util
import os
import threading

from lanbridge.input.base import InputBackend


class LinuxX11InputBackend(InputBackend):
    """Injects keyboard & mouse input directly into Xorg via libX11 + libXtst."""

    def __init__(self, width: int = 1920, height: int = 1080) -> None:
        x11_path = ctypes.util.find_library("X11") or "libX11.so.6"
        xtst_path = ctypes.util.find_library("Xtst") or "libXtst.so.6"
        self._x11 = ctypes.CDLL(x11_path)
        self._xtst = ctypes.CDLL(xtst_path)

        self._x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self._x11.XOpenDisplay.restype = ctypes.c_void_p
        self._x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
        self._x11.XFlush.argtypes = [ctypes.c_void_p]

        self._xtst.XTestFakeKeyEvent.argtypes = [
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_int,
            ctypes.c_ulong,
        ]
        self._xtst.XTestFakeButtonEvent.argtypes = [
            ctypes.c_void_p,
            ctypes.c_uint,
            ctypes.c_int,
            ctypes.c_ulong,
        ]
        self._xtst.XTestFakeMotionEvent.argtypes = [
            ctypes.c_void_p,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.c_ulong,
        ]
        self._xtst.XTestFakeRelativeMotionEvent.argtypes = [
            ctypes.c_void_p,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.c_ulong,
        ]

        display_name = os.environ.get("DISPLAY", ":0").encode("utf-8")
        self._display = self._x11.XOpenDisplay(display_name)
        if not self._display:
            raise RuntimeError("Failed to open X11 display for XTest input")

        self._width = max(1, width)
        self._height = max(1, height)
        self._lock = threading.Lock()
        self._scroll_v_acc = 0.0
        self._scroll_h_acc = 0.0

    def move_abs(self, x_norm: int, y_norm: int) -> None:
        x = int((max(0, min(65535, x_norm)) / 65535.0) * (self._width - 1))
        y = int((max(0, min(65535, y_norm)) / 65535.0) * (self._height - 1))
        with self._lock:
            self._xtst.XTestFakeMotionEvent(self._display, -1, x, y, 0)
            self._x11.XFlush(self._display)

    def move_rel(self, dx: int, dy: int) -> None:
        with self._lock:
            self._xtst.XTestFakeRelativeMotionEvent(self._display, int(dx), int(dy), 0)
            self._x11.XFlush(self._display)

    def mouse_button(self, web_button: int, pressed: bool) -> None:
        # X11 button mapping: 1=left, 2=middle, 3=right, 8=back, 9=forward
        x_btn_map = {0: 1, 1: 2, 2: 3, 3: 8, 4: 9}
        x_btn = x_btn_map.get(web_button)
        if x_btn is None:
            return
        with self._lock:
            self._xtst.XTestFakeButtonEvent(self._display, x_btn, 1 if pressed else 0, 0)
            self._x11.XFlush(self._display)

    def scroll(self, dx: float, dy: float) -> None:
        self._scroll_v_acc += dy / 100.0
        v_steps = int(self._scroll_v_acc)
        self._scroll_h_acc += dx / 100.0
        h_steps = int(self._scroll_h_acc)

        with self._lock:
            if v_steps != 0:
                self._scroll_v_acc -= v_steps
                btn = 5 if v_steps > 0 else 4
                for _ in range(abs(v_steps)):
                    self._xtst.XTestFakeButtonEvent(self._display, btn, 1, 0)
                    self._xtst.XTestFakeButtonEvent(self._display, btn, 0, 0)
            if h_steps != 0:
                self._scroll_h_acc -= h_steps
                btn = 7 if h_steps > 0 else 6
                for _ in range(abs(h_steps)):
                    self._xtst.XTestFakeButtonEvent(self._display, btn, 1, 0)
                    self._xtst.XTestFakeButtonEvent(self._display, btn, 0, 0)
            if v_steps or h_steps:
                self._x11.XFlush(self._display)

    def key_event(self, evdev_code: int, pressed: bool) -> None:
        # Under Xorg evdev, X11 keycode = Linux evdev keycode + 8
        x_keycode = int(evdev_code) + 8
        with self._lock:
            self._xtst.XTestFakeKeyEvent(self._display, x_keycode, 1 if pressed else 0, 0)
            self._x11.XFlush(self._display)

    def close(self) -> None:
        if self._display:
            try:
                self._x11.XCloseDisplay(self._display)
            except Exception:
                pass
            self._display = None
