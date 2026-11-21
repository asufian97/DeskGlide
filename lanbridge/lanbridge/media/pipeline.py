"""GStreamer `webrtcbin` pipeline with DataChannels & instant Hybrid KVM-Only mode."""

from __future__ import annotations

import asyncio
import json
import threading
from typing import Any, Callable, Optional

from lanbridge.capture.base import CaptureDescriptor
from lanbridge.media.probe import (
    EncoderSpec,
    apply_safe_properties,
    encoder_bitrate_value,
)
from lanbridge.protocol import QUALITY_PRESETS
from lanbridge.security import filter_lan_ice_candidate


class HybridModeController:
    """Tracks and returns ordered state transitions for `screen` vs `kvm` mode."""

    def __init__(self, initial_mode: str = "screen") -> None:
        if initial_mode not in ("screen", "kvm"):
            raise ValueError(f"invalid mode: {initial_mode}")
        self.mode = initial_mode

    @property
    def video_drop(self) -> bool:
        return self.mode == "kvm"

    @property
    def video_locked(self) -> bool:
        return self.mode == "kvm"

    @property
    def video_state(self) -> str:
        return "READY" if self.mode == "kvm" else "PLAYING"

    def transition_to(self, target_mode: str) -> list[str]:
        if target_mode not in ("screen", "kvm"):
            raise ValueError(f"invalid mode: {target_mode}")
        if target_mode == self.mode:
            return []
        self.mode = target_mode
        if target_mode == "kvm":
            return ["valve_drop_true", "lock_video_bin", "video_bin_ready"]
        return ["unlock_video_bin", "video_bin_playing", "valve_drop_false", "force_keyframe"]


class GLibMainLoopThread:
    """Shared daemon thread executing GLib MainLoop for GStreamer signals & D-Bus."""

    def __init__(self) -> None:
        self._thread: Optional[threading.Thread] = None
        self._loop: Any = None
        self._ready = threading.Event()
        self._lock = threading.Lock()

    def ensure_started(self) -> None:
        with self._lock:
            if self._thread is not None and self._thread.is_alive():
                return
            import gi

            gi.require_version("GLib", "2.0")
            from gi.repository import GLib

            self._ready.clear()

            def runner() -> None:
                self._loop = GLib.MainLoop()
                self._ready.set()
                self._loop.run()

            self._thread = threading.Thread(
                target=runner,
                name="lanbridge-glib-loop",
                daemon=True,
            )
            self._thread.start()
        self._ready.wait(timeout=5.0)


_GLIB_THREAD = GLibMainLoopThread()


class LanBridgePipeline:
    """Manages one WebRTC peer connection's GStreamer video + DataChannel pipeline."""

    def __init__(
        self,
        descriptor: CaptureDescriptor,
        encoder: EncoderSpec,
        loop: asyncio.AbstractEventLoop,
        on_offer_sdp: Callable[[str], None],
        on_ice_candidate: Callable[[int, str], None],
        on_binary_input: Callable[[bytes], None],
        on_control_message: Callable[[str], None],
        on_ready_channels: Optional[Callable[[], None]] = None,
        on_error: Optional[Callable[[str], None]] = None,
        quality: str = "balanced",
        fps: int = 60,
    ) -> None:
        import gi

        gi.require_version("Gst", "1.0")
        gi.require_version("GstSdp", "1.0")
        gi.require_version("GstWebRTC", "1.0")
        gi.require_version("GstVideo", "1.0")
        gi.require_version("GLib", "2.0")
        from gi.repository import GLib, Gst, GstSdp, GstVideo, GstWebRTC

        self._GLib = GLib
        self._Gst = Gst
        self._GstSdp = GstSdp
        self._GstVideo = GstVideo
        self._GstWebRTC = GstWebRTC
        Gst.init(None)
        _GLIB_THREAD.ensure_started()

        self._descriptor = descriptor
        self._encoder_spec = encoder
        self._loop = loop
        self._on_offer_sdp = on_offer_sdp
        self._on_ice_candidate = on_ice_candidate
        self._on_binary_input = on_binary_input
        self._on_control_message = on_control_message
        self._on_ready_channels = on_ready_channels
        self._on_error = on_error
        self._quality = quality if quality in QUALITY_PRESETS else "balanced"
        self._fps = max(15, min(120, int(fps)))
        self._hybrid = HybridModeController("screen")

        self._pipeline: Any = None
        self._video_bin: Any = None
        self._video_valve: Any = None
        self._encoder: Any = None
        self._webrtc: Any = None
        self._control_dc: Any = None
        self._move_dc: Any = None
        self._closed = False

        self._build_pipeline()

    def _run_on_glib(self, fn: Callable[[], bool]) -> None:
        self._GLib.idle_add(fn)

    def _emit_async(self, fn: Callable[..., Any], *args: Any) -> None:
        if not self._closed:
            self._loop.call_soon_threadsafe(fn, *args)

    def _build_pipeline(self) -> None:
        Gst = self._Gst
        pipe = Gst.Pipeline.new("lanbridge-session")
        video_bin = Gst.Bin.new("video_bin")

        src = Gst.ElementFactory.make(self._descriptor.gst_source_name, "capture_src")
        conv = Gst.ElementFactory.make("videoconvert", "conv")
        rate = Gst.ElementFactory.make("videorate", "rate")
        raw_caps = Gst.ElementFactory.make("capsfilter", "raw_caps")
        q_enc = Gst.ElementFactory.make("queue", "enc_queue")
        enc = Gst.ElementFactory.make(self._encoder_spec.element, "video_enc")
        parse = Gst.ElementFactory.make("h264parse", "h264_parse")
        pay = Gst.ElementFactory.make("rtph264pay", "rtp_pay")
        valve = Gst.ElementFactory.make("valve", "video_valve")
        webrtc = Gst.ElementFactory.make("webrtcbin", "webrtc")

        if not all((src, conv, rate, raw_caps, q_enc, enc, parse, pay, valve, webrtc)):
            raise RuntimeError(
                f"Failed creating GStreamer pipeline elements for {self._descriptor.gst_source_name} / {self._encoder_spec.element}"
            )

        # Configure capture source
        if self._descriptor.pipewire_fd is not None:
            apply_safe_properties(src, {"fd": int(self._descriptor.pipewire_fd)})
        if self._descriptor.pipewire_node_id is not None:
            apply_safe_properties(src, {"path": str(self._descriptor.pipewire_node_id)})
        apply_safe_properties(src, self._descriptor.extra_props)

        # Low-latency frame pacing & queue tuning
        apply_safe_properties(rate, {"drop-only": True})
        raw_caps.set_property(
            "caps",
            Gst.Caps.from_string(f"video/x-raw,framerate={self._fps}/1"),
        )
        apply_safe_properties(q_enc, {"leaky": 2, "max-size-buffers": 2, "max-size-bytes": 0, "max-size-time": 0})

        # Hardware / zerolatency H.264 encoder tuning
        apply_safe_properties(enc, self._encoder_spec.props)
        if self._encoder_spec.bitrate_prop:
            kbps = QUALITY_PRESETS[self._quality]
            apply_safe_properties(
                enc,
                {self._encoder_spec.bitrate_prop: encoder_bitrate_value(self._encoder_spec, kbps)},
            )

        apply_safe_properties(parse, {"config-interval": -1})
        apply_safe_properties(pay, {"config-interval": -1, "aggregate-mode": "zero-latency", "pt": 96})
        apply_safe_properties(valve, {"drop": False})
        apply_safe_properties(webrtc, {"bundle-policy": "max-bundle"})

        # Add elements into video_bin (which can be paused independently in KVM-Only mode)
        for el in (src, conv, rate, raw_caps, q_enc, enc, parse, pay):
            video_bin.add(el)

        assert src.link(conv)
        assert conv.link(rate)
        assert rate.link(raw_caps)
        assert raw_caps.link(q_enc)
        assert q_enc.link(enc)
        assert enc.link(parse)
        assert parse.link(pay)

        pay_src_pad = pay.get_static_pad("src")
        ghost_pad = Gst.GhostPad.new("src", pay_src_pad)
        ghost_pad.set_active(True)
        video_bin.add_pad(ghost_pad)

        pipe.add(video_bin)
        pipe.add(valve)
        pipe.add(webrtc)

        assert video_bin.link(valve)
        rtp_caps = Gst.Caps.from_string(
            "application/x-rtp,media=video,encoding-name=H264,payload=96,clock-rate=90000"
        )
        assert valve.link_filtered(webrtc, rtp_caps)

        # LAN-only ICE: disable STUN/TURN cleanly
        try:
            webrtc.set_property("stun-server", "")
        except Exception:
            pass

        webrtc.connect("on-negotiation-needed", self._on_negotiation_needed)
        webrtc.connect("on-ice-candidate", self._on_ice_candidate_cb)

        bus = pipe.get_bus()
        bus.add_signal_watch()
        bus.connect("message::error", self._on_bus_error)

        self._pipeline = pipe
        self._video_bin = video_bin
        self._video_valve = valve
        self._encoder = enc
        self._webrtc = webrtc

    def start(self) -> None:
        """Start GStreamer pipeline and trigger WebRTC DataChannel + SDP offer creation."""
        def _start_cb() -> bool:
            if self._closed or self._pipeline is None:
                return False
            self._create_datachannels()
            self._pipeline.set_state(self._Gst.State.PLAYING)
            return False

        self._run_on_glib(_start_cb)

    def _create_datachannels(self) -> None:
        if self._webrtc is None:
            return
        Gst = self._Gst

        move_opts = Gst.Structure.new_from_string("options,ordered=false,max-retransmits=0")
        move_dc = self._webrtc.emit("create-data-channel", "input-move", move_opts)
        if move_dc is not None:
            move_dc.connect("on-message-data", self._on_dc_binary_cb)
            move_dc.connect("on-open", self._on_dc_open_cb)
            self._move_dc = move_dc

        ctrl_opts = Gst.Structure.new_from_string("options,ordered=true")
        ctrl_dc = self._webrtc.emit("create-data-channel", "control", ctrl_opts)
        if ctrl_dc is not None:
            ctrl_dc.connect("on-message-string", self._on_dc_string_cb)
            ctrl_dc.connect("on-open", self._on_dc_open_cb)
            self._control_dc = ctrl_dc

    def _on_dc_open_cb(self, _dc: Any) -> None:
        if self._on_ready_channels is not None:
            self._emit_async(self._on_ready_channels)

    def _on_dc_binary_cb(self, _dc: Any, data: Any) -> None:
        try:
            raw = bytes(data.get_data()) if hasattr(data, "get_data") else bytes(data)
            self._emit_async(self._on_binary_input, raw)
        except Exception:
            pass

    def _on_dc_string_cb(self, _dc: Any, text: str) -> None:
        self._emit_async(self._on_control_message, text)

    def _on_negotiation_needed(self, element: Any) -> None:
        promise = self._Gst.Promise.new_with_change_func(self._on_offer_created, element, None)
        element.emit("create-offer", None, promise)

    def _on_offer_created(self, promise: Any, element: Any, _user_data: Any) -> None:
        reply = promise.get_reply()
        if reply is None:
            return
        offer = reply.get_value("offer")
        if offer is None:
            return
        element.emit("set-local-description", offer, self._Gst.Promise.new())
        sdp_text = offer.sdp.as_text()
        self._emit_async(self._on_offer_sdp, sdp_text)

    def _on_ice_candidate_cb(self, _element: Any, mline_index: int, candidate: str) -> None:
        if filter_lan_ice_candidate(candidate):
            self._emit_async(self._on_ice_candidate, int(mline_index), candidate)

    def _on_bus_error(self, _bus: Any, msg: Any) -> None:
        err, debug = msg.parse_error()
        if self._on_error is not None:
            self._emit_async(self._on_error, f"{err.message} ({debug})")

    def set_remote_answer(self, sdp_text: str) -> None:
        def _apply() -> bool:
            if self._closed or self._webrtc is None:
                return False
            res, sdp_msg = self._GstSdp.SDPMessage.new_from_text(sdp_text)
            if res != self._GstSdp.SDPResult.OK:
                return False
            answer = self._GstWebRTC.WebRTCSessionDescription.new(
                self._GstWebRTC.WebRTCSDPType.ANSWER, sdp_msg
            )
            self._webrtc.emit("set-remote-description", answer, self._Gst.Promise.new())
            return False

        self._run_on_glib(_apply)

    def add_remote_ice_candidate(self, sdp_mline_index: int, candidate: str) -> None:
        if not filter_lan_ice_candidate(candidate):
            return

        def _add() -> bool:
            if self._closed or self._webrtc is None:
                return False
            self._webrtc.emit("add-ice-candidate", int(sdp_mline_index), candidate)
            return False

        self._run_on_glib(_add)

    def send_control(self, payload: dict[str, Any]) -> None:
        """Send a reliable JSON message to the remote Web Viewer over `control` DataChannel."""
        text = json.dumps(payload)

        def _send() -> bool:
            if self._closed or self._control_dc is None:
                return False
            try:
                self._control_dc.emit("send-string", text)
            except Exception:
                pass
            return False

        self._run_on_glib(_send)

    def set_mode(self, mode: str) -> list[str]:
        """Switch between `screen` (Mirror + Control) and `kvm` (0% GPU/CPU video pause)."""
        steps = self._hybrid.transition_to(mode)
        if not steps:
            return steps

        def _apply() -> bool:
            if self._closed or self._video_bin is None or self._video_valve is None:
                return False
            if mode == "kvm":
                self._video_valve.set_property("drop", True)
                self._video_bin.set_locked_state(True)
                self._video_bin.set_state(self._Gst.State.READY)
            else:
                self._video_bin.set_locked_state(False)
                self._video_bin.sync_state_with_parent()
                self._video_valve.set_property("drop", False)
                self._emit_force_keyframe_locked()
            return False

        self._run_on_glib(_apply)
        return steps

    def set_quality(self, preset: str) -> None:
        """Dynamically adjust encoder bitrate (`crisp`, `balanced`, `low`)."""
        if preset not in QUALITY_PRESETS:
            return
        self._quality = preset
        kbps = QUALITY_PRESETS[preset]

        def _apply() -> bool:
            if self._closed or self._encoder is None or not self._encoder_spec.bitrate_prop:
                return False
            val = encoder_bitrate_value(self._encoder_spec, kbps)
            apply_safe_properties(self._encoder, {self._encoder_spec.bitrate_prop: val})
            return False

        self._run_on_glib(_apply)

    def force_keyframe(self) -> None:
        """Request an immediate IDR/keyframe from the H.264 encoder."""
        self._run_on_glib(lambda: (self._emit_force_keyframe_locked(), False)[1])

    def _emit_force_keyframe_locked(self) -> None:
        if self._closed or self._encoder is None:
            return
        try:
            event = self._GstVideo.video_event_new_downstream_force_key_unit(
                self._Gst.CLOCK_TIME_NONE,
                self._Gst.CLOCK_TIME_NONE,
                self._Gst.CLOCK_TIME_NONE,
                True,
                0,
            )
            pad = self._encoder.get_static_pad("sink")
            if pad is not None:
                pad.send_event(event)
        except Exception:
            pass

    def close(self) -> None:
        """Stop GStreamer pipeline cleanly."""
        if self._closed:
            return
        self._closed = True
        done = threading.Event()

        def _stop() -> bool:
            if self._pipeline is not None:
                try:
                    self._pipeline.set_state(self._Gst.State.NULL)
                except Exception:
                    pass
                self._pipeline = None
            done.set()
            return False

        try:
            self._run_on_glib(_stop)
            done.wait(timeout=3.0)
        except Exception:
            pass
