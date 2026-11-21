"""Tests for LanBridge wire protocol (9B pointer datagrams, control JSON, keycodes)."""

from __future__ import annotations

import json
import pytest

from lanbridge.protocol import (
    MAX_CLIPBOARD_BYTES,
    MOUSE_ABS,
    MOUSE_REL,
    QUALITY_PRESETS,
    WEB_CODE_TO_EVDEV,
    decode_mouse_packet,
    encode_mouse_abs,
    encode_mouse_rel,
    evdev_to_win_scancode,
    parse_control_message,
)


def test_binary_mouse_abs_roundtrip_and_clamp():
    pkt = encode_mouse_abs(32768, 65535)
    assert len(pkt) == 9
    decoded = decode_mouse_packet(pkt)
    assert decoded.kind == MOUSE_ABS
    assert decoded.x_norm == 32768
    assert decoded.y_norm == 65535
    assert decoded.dx == 0 and decoded.dy == 0

    clamped = decode_mouse_packet(encode_mouse_abs(-100, 999999))
    assert clamped.x_norm == 0
    assert clamped.y_norm == 65535


def test_binary_mouse_rel_roundtrip_and_clamp():
    pkt = encode_mouse_rel(-420, 1250)
    assert len(pkt) == 9
    decoded = decode_mouse_packet(pkt)
    assert decoded.kind == MOUSE_REL
    assert decoded.dx == -420
    assert decoded.dy == 1250

    clamped = decode_mouse_packet(encode_mouse_rel(-100000, 100000))
    assert clamped.dx == -32768
    assert clamped.dy == 32767


def test_binary_mouse_rejects_bad_length_and_kind():
    with pytest.raises(ValueError):
        decode_mouse_packet(b"\x01\x00\x00")
    with pytest.raises(ValueError):
        decode_mouse_packet(b"\x09" + b"\x00" * 8)


def test_control_message_parsing():
    key_msg = parse_control_message(json.dumps({"type": "key", "code": "MetaLeft", "pressed": True}))
    assert key_msg == {"type": "key", "code": "MetaLeft", "evdev": 125, "pressed": True}

    assert parse_control_message(json.dumps({"type": "key", "code": "UnknownCode", "pressed": True})) is None
    assert parse_control_message(json.dumps({"type": "mode", "mode": "kvm"})) == {"type": "mode", "mode": "kvm"}
    assert parse_control_message(json.dumps({"type": "mode", "mode": "invalid"})) is None

    for preset in QUALITY_PRESETS:
        assert parse_control_message(json.dumps({"type": "quality", "preset": preset})) == {
            "type": "quality",
            "preset": preset,
        }

    huge_clip = "a" * (MAX_CLIPBOARD_BYTES + 200)
    parsed_clip = parse_control_message(json.dumps({"type": "clipboard", "text": huge_clip}))
    assert parsed_clip is not None
    assert len(parsed_clip["text"].encode("utf-8")) <= MAX_CLIPBOARD_BYTES


def test_web_codes_and_windows_scancode_mapping():
    critical = [
        "Escape", "Tab", "Enter", "Space", "AltLeft", "AltRight",
        "ControlLeft", "ControlRight", "ShiftLeft", "ShiftRight",
        "MetaLeft", "MetaRight", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight",
        "F1", "F12", "Delete", "Backspace",
    ]
    for code in critical:
        ev = WEB_CODE_TO_EVDEV[code]
        wk = evdev_to_win_scancode(ev)
        assert wk is not None, f"Missing Windows scancode mapping for {code}"

    # Verify RightShift is non-extended 0x36 and RightAlt is extended 0x38
    assert evdev_to_win_scancode(WEB_CODE_TO_EVDEV["ShiftRight"]) == (0x36, False)
    assert evdev_to_win_scancode(WEB_CODE_TO_EVDEV["AltRight"]) == (0x38, True)
