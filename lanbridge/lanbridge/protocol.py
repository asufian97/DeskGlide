"""Wire protocol (9-byte binary pointer datagrams, JSON control messages) & keycode maps."""

from __future__ import annotations

import json
import struct
from dataclasses import dataclass
from typing import Any, Literal, Optional, Tuple

QUALITY_PRESETS: dict[str, int] = {
    "crisp": 12000,     # 12 Mbps (kbps)
    "balanced": 6000,   # 6 Mbps (kbps)
    "low": 2500,        # 2.5 Mbps (kbps)
}

MOUSE_ABS = 1
MOUSE_REL = 2
_MOUSE_STRUCT = struct.Struct("!BhhHH")
assert _MOUSE_STRUCT.size == 9

MAX_CLIPBOARD_BYTES = 128 * 1024

# Linux evdev mouse buttons (linux/input-event-codes.h)
BTN_LEFT = 0x110
BTN_RIGHT = 0x111
BTN_MIDDLE = 0x112
BTN_SIDE = 0x113
BTN_EXTRA = 0x114

WEB_BUTTON_TO_EVDEV: dict[int, int] = {
    0: BTN_LEFT,
    1: BTN_MIDDLE,
    2: BTN_RIGHT,
    3: BTN_SIDE,
    4: BTN_EXTRA,
}


@dataclass(frozen=True)
class MouseMovePacket:
    kind: Literal[1, 2]
    dx: int
    dy: int
    x_norm: int  # 0..65535 when kind == MOUSE_ABS
    y_norm: int  # 0..65535 when kind == MOUSE_ABS


def _clamp_i16(val: int) -> int:
    return max(-32768, min(32767, int(val)))


def _clamp_u16(val: int) -> int:
    return max(0, min(65535, int(val)))


def encode_mouse_abs(x_norm: int, y_norm: int) -> bytes:
    """Encode normalized absolute pointer position (0..65535) into 9 bytes."""
    return _MOUSE_STRUCT.pack(MOUSE_ABS, 0, 0, _clamp_u16(x_norm), _clamp_u16(y_norm))


def encode_mouse_rel(dx: int, dy: int) -> bytes:
    """Encode relative mouse delta into 9 bytes."""
    return _MOUSE_STRUCT.pack(MOUSE_REL, _clamp_i16(dx), _clamp_i16(dy), 0, 0)


def decode_mouse_packet(data: bytes | bytearray | memoryview) -> MouseMovePacket:
    """Decode and validate a 9-byte `input-move` DataChannel packet."""
    if len(data) != _MOUSE_STRUCT.size:
        raise ValueError(f"invalid mouse packet length: {len(data)} (expected 9)")
    kind, dx, dy, x_norm, y_norm = _MOUSE_STRUCT.unpack(data)
    if kind not in (MOUSE_ABS, MOUSE_REL):
        raise ValueError(f"unknown mouse packet kind: {kind}")
    return MouseMovePacket(kind=kind, dx=dx, dy=dy, x_norm=x_norm, y_norm=y_norm)  # type: ignore[arg-type]


def parse_control_message(raw: str) -> Optional[dict[str, Any]]:
    """Validate and normalize a JSON `control` DataChannel message."""
    if not raw or len(raw) > MAX_CLIPBOARD_BYTES + 1024:
        return None
    try:
        msg = json.loads(raw)
    except json.JSONDecodeError:
        return None
    if not isinstance(msg, dict):
        return None
    msg_type = msg.get("type")
    if not isinstance(msg_type, str):
        return None

    if msg_type == "mouse_btn":
        btn = msg.get("button")
        pressed = msg.get("pressed")
        if isinstance(btn, int) and btn in WEB_BUTTON_TO_EVDEV and isinstance(pressed, bool):
            return {"type": "mouse_btn", "button": btn, "pressed": pressed}
    elif msg_type == "wheel":
        dx = msg.get("dx", 0.0)
        dy = msg.get("dy", 0.0)
        if isinstance(dx, (int, float)) and isinstance(dy, (int, float)):
            return {
                "type": "wheel",
                "dx": max(-2000.0, min(2000.0, float(dx))),
                "dy": max(-2000.0, min(2000.0, float(dy))),
            }
    elif msg_type == "key":
        code = msg.get("code")
        pressed = msg.get("pressed")
        if isinstance(code, str) and isinstance(pressed, bool):
            evdev_code = WEB_CODE_TO_EVDEV.get(code)
            if evdev_code is not None:
                return {"type": "key", "code": code, "evdev": evdev_code, "pressed": pressed}
    elif msg_type == "release_all":
        return {"type": "release_all"}
    elif msg_type == "clipboard":
        text = msg.get("text")
        if isinstance(text, str):
            encoded = text.encode("utf-8")
            if len(encoded) > MAX_CLIPBOARD_BYTES:
                text = encoded[:MAX_CLIPBOARD_BYTES].decode("utf-8", errors="ignore")
            return {"type": "clipboard", "text": text}
    elif msg_type == "mode":
        mode = msg.get("mode")
        if mode in ("screen", "kvm"):
            return {"type": "mode", "mode": mode}
    elif msg_type == "quality":
        preset = msg.get("preset")
        if preset in QUALITY_PRESETS:
            return {"type": "quality", "preset": preset}
    elif msg_type == "keyframe":
        return {"type": "keyframe"}
    elif msg_type == "ping":
        ts = msg.get("ts")
        if isinstance(ts, (int, float)):
            return {"type": "ping", "ts": float(ts)}

    return None


# W3C KeyboardEvent.code -> Linux evdev keycode (layout-independent)
WEB_CODE_TO_EVDEV: dict[str, int] = {
    "Escape": 1,
    "Digit1": 2,
    "Digit2": 3,
    "Digit3": 4,
    "Digit4": 5,
    "Digit5": 6,
    "Digit6": 7,
    "Digit7": 8,
    "Digit8": 9,
    "Digit9": 10,
    "Digit0": 11,
    "Minus": 12,
    "Equal": 13,
    "Backspace": 14,
    "Tab": 15,
    "KeyQ": 16,
    "KeyW": 17,
    "KeyE": 18,
    "KeyR": 19,
    "KeyT": 20,
    "KeyY": 21,
    "KeyU": 22,
    "KeyI": 23,
    "KeyO": 24,
    "KeyP": 25,
    "BracketLeft": 26,
    "BracketRight": 27,
    "Enter": 28,
    "ControlLeft": 29,
    "KeyA": 30,
    "KeyS": 31,
    "KeyD": 32,
    "KeyF": 33,
    "KeyG": 34,
    "KeyH": 35,
    "KeyJ": 36,
    "KeyK": 37,
    "KeyL": 38,
    "Semicolon": 39,
    "Quote": 40,
    "Backquote": 41,
    "ShiftLeft": 42,
    "Backslash": 43,
    "KeyZ": 44,
    "KeyX": 45,
    "KeyC": 46,
    "KeyV": 47,
    "KeyB": 48,
    "KeyN": 49,
    "KeyM": 50,
    "Comma": 51,
    "Period": 52,
    "Slash": 53,
    "ShiftRight": 54,
    "NumpadMultiply": 55,
    "AltLeft": 56,
    "Space": 57,
    "CapsLock": 58,
    "F1": 59,
    "F2": 60,
    "F3": 61,
    "F4": 62,
    "F5": 63,
    "F6": 64,
    "F7": 65,
    "F8": 66,
    "F9": 67,
    "F10": 68,
    "NumLock": 69,
    "ScrollLock": 70,
    "Numpad7": 71,
    "Numpad8": 72,
    "Numpad9": 73,
    "NumpadSubtract": 74,
    "Numpad4": 75,
    "Numpad5": 76,
    "Numpad6": 77,
    "NumpadAdd": 78,
    "Numpad1": 79,
    "Numpad2": 80,
    "Numpad3": 81,
    "Numpad0": 82,
    "NumpadDecimal": 83,
    "IntlBackslash": 86,
    "F11": 87,
    "F12": 88,
    "NumpadEnter": 96,
    "ControlRight": 97,
    "NumpadDivide": 98,
    "PrintScreen": 99,
    "AltRight": 100,
    "Home": 102,
    "ArrowUp": 103,
    "PageUp": 104,
    "ArrowLeft": 105,
    "ArrowRight": 106,
    "End": 107,
    "ArrowDown": 108,
    "PageDown": 109,
    "Insert": 110,
    "Delete": 111,
    "AudioVolumeMute": 113,
    "AudioVolumeDown": 114,
    "AudioVolumeUp": 115,
    "Pause": 119,
    "NumpadComma": 121,
    "MetaLeft": 125,
    "MetaRight": 126,
    "ContextMenu": 127,
    "MediaTrackNext": 163,
    "MediaPlayPause": 164,
    "MediaTrackPrevious": 165,
    "MediaStop": 166,
    "F13": 183,
    "F14": 184,
    "F15": 185,
    "F16": 186,
    "F17": 187,
    "F18": 188,
    "F19": 189,
    "F20": 190,
    "F21": 191,
    "F22": 192,
    "F23": 193,
    "F24": 194,
}


def evdev_to_win_scancode(code: int) -> Optional[Tuple[int, bool]]:
    """Convert Linux evdev keycode to `(win_set1_scancode, extended)` for SendInput."""
    if 1 <= code <= 68 or 70 <= code <= 83:
        return (code, False)
    mapping: dict[int, Tuple[int, bool]] = {
        69: (0x45, True),   # NumLock
        86: (0x56, False),  # IntlBackslash / 102ND
        87: (0x57, False),  # F11
        88: (0x58, False),  # F12
        96: (0x1C, True),   # NumpadEnter
        97: (0x1D, True),   # ControlRight
        98: (0x35, True),   # NumpadDivide
        99: (0x37, True),   # PrintScreen
        100: (0x38, True),  # AltRight
        102: (0x47, True),  # Home
        103: (0x48, True),  # ArrowUp
        104: (0x49, True),  # PageUp
        105: (0x4B, True),  # ArrowLeft
        106: (0x4D, True),  # ArrowRight
        107: (0x4F, True),  # End
        108: (0x50, True),  # ArrowDown
        109: (0x51, True),  # PageDown
        110: (0x52, True),  # Insert
        111: (0x53, True),  # Delete
        113: (0x20, True),  # Mute
        114: (0x2E, True),  # VolumeDown
        115: (0x30, True),  # VolumeUp
        119: (0x45, False), # Pause
        121: (0x7E, False), # NumpadComma
        125: (0x5B, True),  # MetaLeft
        126: (0x5C, True),  # MetaRight
        127: (0x5D, True),  # ContextMenu
        163: (0x19, True),  # NextTrack
        164: (0x22, True),  # PlayPause
        165: (0x10, True),  # PrevTrack
        166: (0x24, True),  # Stop
        183: (0x64, False), # F13
        184: (0x65, False), # F14
        185: (0x66, False), # F15
        186: (0x67, False), # F16
        187: (0x68, False), # F17
        188: (0x69, False), # F18
        189: (0x6A, False), # F19
        190: (0x6B, False), # F20
        191: (0x6C, False), # F21
        192: (0x6D, False), # F22
        193: (0x6E, False), # F23
        194: (0x76, False), # F24
    }
    return mapping.get(code)
