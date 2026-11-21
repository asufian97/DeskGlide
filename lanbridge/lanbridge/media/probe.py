"""Hardware H.264 GStreamer encoder discovery and priority fallback probing."""

from __future__ import annotations

import sys
from dataclasses import dataclass, field
from typing import Any, Iterable, Optional

LINUX_ENCODER_PRIORITY = ("vah264enc", "vaapih264enc", "nvh264enc", "x264enc")
WINDOWS_ENCODER_PRIORITY = ("d3d11h264enc", "mfh264enc", "nvh264enc", "x264enc")


@dataclass(frozen=True)
class EncoderSpec:
    """Configuration for a selected GStreamer H.264 encoder plugin."""

    element: str
    bitrate_prop: Optional[str] = "bitrate"
    bitrate_unit: str = "kbps"  # "kbps" | "bps"
    hardware: bool = True
    props: dict[str, Any] = field(default_factory=dict)


_KNOWN_ENCODER_SPECS: dict[str, EncoderSpec] = {
    "vah264enc": EncoderSpec(
        element="vah264enc",
        bitrate_prop="bitrate",
        bitrate_unit="kbps",
        hardware=True,
        props={"rate-control": "cbr", "target-usage": 6},
    ),
    "vaapih264enc": EncoderSpec(
        element="vaapih264enc",
        bitrate_prop="bitrate",
        bitrate_unit="kbps",
        hardware=True,
        props={"rate-control": "cbr", "tune": "low-power"},
    ),
    "d3d11h264enc": EncoderSpec(
        element="d3d11h264enc",
        bitrate_prop="bitrate",
        bitrate_unit="kbps",
        hardware=True,
        props={"rate-control": "cbr", "low-latency": True},
    ),
    "mfh264enc": EncoderSpec(
        element="mfh264enc",
        bitrate_prop="bitrate",
        bitrate_unit="kbps",
        hardware=True,
        props={"low-latency": True, "rc-mode": "cbr"},
    ),
    "nvh264enc": EncoderSpec(
        element="nvh264enc",
        bitrate_prop="bitrate",
        bitrate_unit="kbps",
        hardware=True,
        props={"zerolatency": True, "rc-mode": "cbr", "preset": "low-latency-hq"},
    ),
    "x264enc": EncoderSpec(
        element="x264enc",
        bitrate_prop="bitrate",
        bitrate_unit="kbps",
        hardware=False,
        props={"tune": "zerolatency", "speed-preset": "ultrafast", "key-int-max": 60},
    ),
}


def encoder_priority_for_platform(platform_name: Optional[str] = None) -> tuple[str, ...]:
    """Return ordered H.264 encoder priority list for the target platform."""
    plat = (platform_name or sys.platform).lower()
    if plat.startswith("win"):
        return WINDOWS_ENCODER_PRIORITY
    return LINUX_ENCODER_PRIORITY


def choose_encoder_from_available(
    available_plugins: Iterable[str],
    platform_name: Optional[str] = None,
) -> Optional[EncoderSpec]:
    """Select the highest-priority H.264 encoder present in `available_plugins`."""
    avail = set(available_plugins)
    for name in encoder_priority_for_platform(platform_name):
        if name in avail:
            return _KNOWN_ENCODER_SPECS[name]
    return None


def encoder_bitrate_value(spec: EncoderSpec, bitrate_kbps: int) -> int:
    """Convert target kbps into the integer value expected by `spec`."""
    kbps = max(500, int(bitrate_kbps))
    return kbps * 1000 if spec.bitrate_unit == "bps" else kbps


def apply_safe_properties(gst_element: Any, props: dict[str, Any]) -> None:
    """Set only properties that actually exist on `gst_element` (safe across GStreamer versions)."""
    try:
        supported = {p.name for p in gst_element.list_properties()}
    except Exception:
        supported = set()

    for key, val in props.items():
        if supported and key not in supported:
            continue
        try:
            gst_element.set_property(key, val)
        except Exception:
            pass


def _dry_run_encoder(Gst: Any, spec: EncoderSpec) -> bool:
    """Test 1-frame encode pipeline to confirm hardware driver actually works."""
    pipeline = None
    try:
        pipeline = Gst.Pipeline.new("lanbridge-probe")
        src = Gst.ElementFactory.make("videotestsrc", None)
        caps_raw = Gst.ElementFactory.make("capsfilter", None)
        conv = Gst.ElementFactory.make("videoconvert", None)
        enc = Gst.ElementFactory.make(spec.element, None)
        parse = Gst.ElementFactory.make("h264parse", None)
        sink = Gst.ElementFactory.make("fakesink", None)
        if not all((src, caps_raw, conv, enc, parse, sink)):
            return False

        src.set_property("num-buffers", 1)
        src.set_property("is-live", False)
        caps_raw.set_property(
            "caps", Gst.Caps.from_string("video/x-raw,width=128,height=128,framerate=30/1")
        )
        apply_safe_properties(enc, spec.props)
        if spec.bitrate_prop:
            apply_safe_properties(enc, {spec.bitrate_prop: encoder_bitrate_value(spec, 4000)})

        for el in (src, caps_raw, conv, enc, parse, sink):
            pipeline.add(el)
        assert src.link(caps_raw)
        assert caps_raw.link(conv)
        assert conv.link(enc)
        assert enc.link(parse)
        assert parse.link(sink)

        ret = pipeline.set_state(Gst.State.PLAYING)
        if ret == Gst.StateChangeReturn.FAILURE:
            return False

        bus = pipeline.get_bus()
        msg = bus.timed_pop_filtered(
            3 * Gst.SECOND,
            Gst.MessageType.EOS | Gst.MessageType.ERROR,
        )
        return msg is not None and msg.type == Gst.MessageType.EOS
    except Exception:
        return False
    finally:
        if pipeline is not None:
            try:
                pipeline.set_state(Gst.State.NULL)
            except Exception:
                pass


def probe_best_encoder(
    dry_run: bool = True,
    platform_name: Optional[str] = None,
) -> Optional[EncoderSpec]:
    """Find the best working H.264 encoder plugin on this system via GStreamer."""
    try:
        import gi

        gi.require_version("Gst", "1.0")
        from gi.repository import Gst
    except Exception:
        return None

    Gst.init(None)
    for name in encoder_priority_for_platform(platform_name):
        if Gst.ElementFactory.find(name) is None:
            continue
        spec = _KNOWN_ENCODER_SPECS[name]
        if not dry_run or _dry_run_encoder(Gst, spec):
            return spec
    return None
