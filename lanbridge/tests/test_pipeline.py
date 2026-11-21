"""Tests for encoder fallback priority, bitrate units, and Hybrid KVM-Only state machine."""

from __future__ import annotations

from lanbridge.media.pipeline import HybridModeController
from lanbridge.media.probe import (
    EncoderSpec,
    choose_encoder_from_available,
    encoder_bitrate_value,
)


def test_linux_encoder_fallback_order():
    all_linux = {"vah264enc", "vaapih264enc", "nvh264enc", "x264enc"}
    assert choose_encoder_from_available(all_linux, "linux").element == "vah264enc"  # type: ignore[union-attr]

    all_linux.remove("vah264enc")
    assert choose_encoder_from_available(all_linux, "linux").element == "vaapih264enc"  # type: ignore[union-attr]

    all_linux.remove("vaapih264enc")
    assert choose_encoder_from_available(all_linux, "linux").element == "nvh264enc"  # type: ignore[union-attr]

    all_linux.remove("nvh264enc")
    sw = choose_encoder_from_available(all_linux, "linux")
    assert sw is not None and sw.element == "x264enc" and not sw.hardware

    assert choose_encoder_from_available(set(), "linux") is None


def test_windows_encoder_fallback_order():
    all_win = {"d3d11h264enc", "mfh264enc", "nvh264enc", "x264enc"}
    assert choose_encoder_from_available(all_win, "win32").element == "d3d11h264enc"  # type: ignore[union-attr]

    all_win.remove("d3d11h264enc")
    assert choose_encoder_from_available(all_win, "win32").element == "mfh264enc"  # type: ignore[union-attr]

    all_win.remove("mfh264enc")
    assert choose_encoder_from_available(all_win, "win32").element == "nvh264enc"  # type: ignore[union-attr]


def test_encoder_bitrate_conversion():
    kbps_spec = EncoderSpec(element="vah264enc", bitrate_unit="kbps")
    bps_spec = EncoderSpec(element="custom", bitrate_unit="bps")
    assert encoder_bitrate_value(kbps_spec, 15000) == 15000
    assert encoder_bitrate_value(bps_spec, 10000) == 10_000_000


def test_hybrid_mode_state_machine():
    ctl = HybridModeController("screen")
    assert not ctl.video_drop
    assert not ctl.video_locked
    assert ctl.video_state == "PLAYING"

    steps_to_kvm = ctl.transition_to("kvm")
    assert steps_to_kvm == ["valve_drop_true", "lock_video_bin", "video_bin_ready"]
    assert ctl.video_drop
    assert ctl.video_locked
    assert ctl.video_state == "READY"

    # Idempotent repeat returns empty steps
    assert ctl.transition_to("kvm") == []

    steps_to_screen = ctl.transition_to("screen")
    assert steps_to_screen == ["unlock_video_bin", "video_bin_playing", "valve_drop_false", "force_keyframe"]
    assert not ctl.video_drop
    assert not ctl.video_locked
    assert ctl.video_state == "PLAYING"


def test_doctor_and_cli_modules_importable():
    import lanbridge.doctor as doc
    import lanbridge.main as cli

    args = cli.parse_args(["--port", "9443", "--quality", "crisp"])
    assert args.port == 9443
    assert args.quality == "crisp"
    assert callable(doc.run_diagnostics)
