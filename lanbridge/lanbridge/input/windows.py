"""Win32 `SendInput` keyboard & mouse injection backend via ctypes."""

from __future__ import annotations

import ctypes
from ctypes import wintypes

from lanbridge.input.base import InputBackend
from lanbridge.protocol import evdev_to_win_scancode

INPUT_MOUSE = 0
INPUT_KEYBOARD = 1

MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
MOUSEEVENTF_RIGHTDOWN = 0x0008
MOUSEEVENTF_RIGHTUP = 0x0010
MOUSEEVENTF_MIDDLEDOWN = 0x0020
MOUSEEVENTF_MIDDLEUP = 0x0040
MOUSEEVENTF_XDOWN = 0x0080
MOUSEEVENTF_XUP = 0x0100
MOUSEEVENTF_WHEEL = 0x0800
MOUSEEVENTF_HWHEEL = 0x1000
MOUSEEVENTF_VIRTUALDESK = 0x4000
MOUSEEVENTF_ABSOLUTE = 0x8000

XBUTTON1 = 0x0001
XBUTTON2 = 0x0002
WHEEL_DELTA = 120

KEYEVENTF_EXTENDEDKEY = 0x0001
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_SCANCODE = 0x0008

ULONG_PTR = ctypes.c_size_t


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", wintypes.LONG),
        ("dy", wintypes.LONG),
        ("mouseData", wintypes.DWORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ULONG_PTR),
    ]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wintypes.WORD),
        ("wScan", wintypes.WORD),
        ("dwFlags", wintypes.DWORD),
        ("time", wintypes.DWORD),
        ("dwExtraInfo", ULONG_PTR),
    ]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [
        ("uMsg", wintypes.DWORD),
        ("wParamL", wintypes.WORD),
        ("wParamH", wintypes.WORD),
    ]


class _INPUT_UNION(ctypes.Union):
    _fields_ = [
        ("mi", MOUSEINPUT),
        ("ki", KEYBDINPUT),
        ("hi", HARDWAREINPUT),
    ]


class INPUT(ctypes.Structure):
    _anonymous_ = ("_u",)
    _fields_ = [
        ("type", wintypes.DWORD),
        ("_u", _INPUT_UNION),
    ]


class WindowsInputBackend(InputBackend):
    """Injects input on Windows via `user32.SendInput`."""

    def __init__(self) -> None:
        self._user32 = ctypes.windll.user32  # type: ignore[attr-defined]
        try:
            self._user32.SetProcessDPIAware()
        except Exception:
            pass
        self._user32.SendInput.argtypes = [wintypes.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
        self._user32.SendInput.restype = wintypes.UINT

    def _send(self, inp: INPUT) -> None:
        self._user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))

    def move_abs(self, x_norm: int, y_norm: int) -> None:
        inp = INPUT(
            type=INPUT_MOUSE,
            mi=MOUSEINPUT(
                dx=max(0, min(65535, int(x_norm))),
                dy=max(0, min(65535, int(y_norm))),
                mouseData=0,
                dwFlags=MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                time=0,
                dwExtraInfo=0,
            ),
        )
        self._send(inp)

    def move_rel(self, dx: int, dy: int) -> None:
        if not dx and not dy:
            return
        inp = INPUT(
            type=INPUT_MOUSE,
            mi=MOUSEINPUT(
                dx=int(dx),
                dy=int(dy),
                mouseData=0,
                dwFlags=MOUSEEVENTF_MOVE,
                time=0,
                dwExtraInfo=0,
            ),
        )
        self._send(inp)

    def mouse_button(self, web_button: int, pressed: bool) -> None:
        flags = 0
        mouse_data = 0
        if web_button == 0:
            flags = MOUSEEVENTF_LEFTDOWN if pressed else MOUSEEVENTF_LEFTUP
        elif web_button == 1:
            flags = MOUSEEVENTF_MIDDLEDOWN if pressed else MOUSEEVENTF_MIDDLEUP
        elif web_button == 2:
            flags = MOUSEEVENTF_RIGHTDOWN if pressed else MOUSEEVENTF_RIGHTUP
        elif web_button == 3:
            flags = MOUSEEVENTF_XDOWN if pressed else MOUSEEVENTF_XUP
            mouse_data = XBUTTON1
        elif web_button == 4:
            flags = MOUSEEVENTF_XDOWN if pressed else MOUSEEVENTF_XUP
            mouse_data = XBUTTON2
        else:
            return

        inp = INPUT(
            type=INPUT_MOUSE,
            mi=MOUSEINPUT(dx=0, dy=0, mouseData=mouse_data, dwFlags=flags, time=0, dwExtraInfo=0),
        )
        self._send(inp)

    def scroll(self, dx: float, dy: float) -> None:
        if dy:
            delta = int(round((-dy / 100.0) * WHEEL_DELTA))
            if delta:
                self._send(
                    INPUT(
                        type=INPUT_MOUSE,
                        mi=MOUSEINPUT(
                            dx=0, dy=0, mouseData=delta, dwFlags=MOUSEEVENTF_WHEEL, time=0, dwExtraInfo=0
                        ),
                    )
                )
        if dx:
            delta = int(round((dx / 100.0) * WHEEL_DELTA))
            if delta:
                self._send(
                    INPUT(
                        type=INPUT_MOUSE,
                        mi=MOUSEINPUT(
                            dx=0, dy=0, mouseData=delta, dwFlags=MOUSEEVENTF_HWHEEL, time=0, dwExtraInfo=0
                        ),
                    )
                )

    def key_event(self, evdev_code: int, pressed: bool) -> None:
        wk = evdev_to_win_scancode(evdev_code)
        if wk is None:
            return
        scan_code, extended = wk
        flags = KEYEVENTF_SCANCODE
        if extended:
            flags |= KEYEVENTF_EXTENDEDKEY
        if not pressed:
            flags |= KEYEVENTF_KEYUP

        inp = INPUT(
            type=INPUT_KEYBOARD,
            ki=KEYBDINPUT(wVk=0, wScan=scan_code, dwFlags=flags, time=0, dwExtraInfo=0),
        )
        self._send(inp)

    def close(self) -> None:
        return None
