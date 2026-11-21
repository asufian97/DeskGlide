"""WebRTC host session coordinator tying Capture, Media Pipeline, Input, and Clipboard."""

from __future__ import annotations

import asyncio
import json
from typing import Any, Optional

from aiohttp import WSMsgType, web

from lanbridge.capture import CaptureBackend, CaptureDescriptor, create_capture_backend
from lanbridge.config import LanBridgeConfig, save_config
from lanbridge.input import HeldInputTracker, create_input_backend
from lanbridge.input.clipboard import ClipboardBridge
from lanbridge.media import EncoderSpec, LanBridgePipeline, probe_best_encoder
from lanbridge.protocol import MOUSE_ABS, decode_mouse_packet, parse_control_message


class LanBridgeSession:
    """Owns one active Web Viewer connection's capture, WebRTC pipeline, and input."""

    def __init__(
        self,
        ws: web.WebSocketResponse,
        config: LanBridgeConfig,
        encoder: EncoderSpec,
    ) -> None:
        self._ws = ws
        self._config = config
        self._encoder = encoder
        self._loop = asyncio.get_running_loop()
        self._capture: Optional[CaptureBackend] = None
        self._descriptor: Optional[CaptureDescriptor] = None
        self._input: Optional[HeldInputTracker] = None
        self._pipeline: Optional[LanBridgePipeline] = None
        self._clipboard: Optional[ClipboardBridge] = None
        self._clip_task: Optional[asyncio.Task[None]] = None
        self._mode = "screen"
        self._quality = config.quality
        self._closed = False

    def _on_restore_token(self, token: str) -> None:
        self._config.portal_restore_token = token
        try:
            save_config(self._config)
        except Exception:
            pass

    def _ws_send(self, payload: dict[str, Any]) -> None:
        if self._closed or self._ws.closed:
            return
        self._loop.create_task(self._send_json_safe(payload))

    async def _send_json_safe(self, payload: dict[str, Any]) -> None:
        if not self._closed and not self._ws.closed:
            try:
                await self._ws.send_str(json.dumps(payload))
            except Exception:
                pass

    def _on_offer_sdp(self, sdp: str) -> None:
        self._ws_send({"type": "offer", "sdp": sdp})

    def _on_ice_candidate(self, sdp_mline_index: int, candidate: str) -> None:
        self._ws_send(
            {
                "type": "candidate",
                "sdpMLineIndex": sdp_mline_index,
                "candidate": candidate,
            }
        )

    def _on_ready_channels(self) -> None:
        if self._pipeline is None or self._descriptor is None:
            return
        self._pipeline.send_control(
            {
                "type": "meta",
                "encoder": self._encoder.element,
                "hardware": self._encoder.hardware,
                "display_server": self._descriptor.display_server,
                "width": self._descriptor.width,
                "height": self._descriptor.height,
                "mode": self._mode,
                "quality": self._quality,
                "clipboard_sync": self._config.clipboard_sync,
            }
        )

    def _on_binary_input(self, raw: bytes) -> None:
        if self._input is None:
            return
        try:
            pkt = decode_mouse_packet(raw)
        except ValueError:
            return
        if pkt.kind == MOUSE_ABS:
            self._input.move_abs(pkt.x_norm, pkt.y_norm)
        else:
            self._input.move_rel(pkt.dx, pkt.dy)

    def _on_control_message(self, raw: str) -> None:
        msg = parse_control_message(raw)
        if msg is None:
            return
        mtype = msg["type"]

        if mtype == "mouse_btn" and self._input is not None:
            self._input.mouse_button(msg["button"], msg["pressed"])
        elif mtype == "wheel" and self._input is not None:
            self._input.scroll(msg["dx"], msg["dy"])
        elif mtype == "key" and self._input is not None:
            self._input.key_event(msg["evdev"], msg["pressed"])
        elif mtype == "release_all" and self._input is not None:
            self._input.release_all()
        elif mtype == "mode" and self._pipeline is not None:
            self._mode = msg["mode"]
            self._pipeline.set_mode(self._mode)
            self._pipeline.send_control({"type": "mode_ack", "mode": self._mode})
        elif mtype == "quality" and self._pipeline is not None:
            self._quality = msg["preset"]
            self._pipeline.set_quality(self._quality)
            self._pipeline.send_control({"type": "quality_ack", "preset": self._quality})
        elif mtype == "keyframe" and self._pipeline is not None:
            self._pipeline.force_keyframe()
        elif mtype == "clipboard" and self._clipboard is not None and self._config.clipboard_sync:
            self._clipboard.set_remote_text(msg["text"])
        elif mtype == "ping" and self._pipeline is not None:
            self._pipeline.send_control({"type": "pong", "ts": msg["ts"]})

    def _on_pipeline_error(self, message: str) -> None:
        self._ws_send({"type": "error", "message": message})

    async def _clipboard_loop(self) -> None:
        while not self._closed:
            await asyncio.sleep(0.75)
            if self._closed or self._clipboard is None or self._pipeline is None:
                continue
            if not self._config.clipboard_sync:
                continue
            text = await asyncio.to_thread(self._clipboard.poll_local_change)
            if text:
                self._pipeline.send_control({"type": "clipboard", "text": text})

    async def run(self) -> None:
        """Start capture/input/pipeline and process WSS signaling until closed."""
        try:
            self._capture = create_capture_backend(
                restore_token=self._config.portal_restore_token,
                on_restore_token=self._on_restore_token,
            )
            self._descriptor = await asyncio.to_thread(self._capture.start)
            self._input = create_input_backend(self._capture, self._descriptor)
            self._clipboard = ClipboardBridge(self._descriptor.display_server)

            self._pipeline = LanBridgePipeline(
                descriptor=self._descriptor,
                encoder=self._encoder,
                loop=self._loop,
                on_offer_sdp=self._on_offer_sdp,
                on_ice_candidate=self._on_ice_candidate,
                on_binary_input=self._on_binary_input,
                on_control_message=self._on_control_message,
                on_ready_channels=self._on_ready_channels,
                on_error=self._on_pipeline_error,
                quality=self._quality,
                fps=self._config.fps,
            )
            self._pipeline.start()
            self._clip_task = asyncio.create_task(self._clipboard_loop())

            async for ws_msg in self._ws:
                if ws_msg.type != WSMsgType.TEXT:
                    if ws_msg.type in (WSMsgType.CLOSE, WSMsgType.ERROR):
                        break
                    continue
                try:
                    data = json.loads(ws_msg.data)
                except json.JSONDecodeError:
                    continue
                if not isinstance(data, dict):
                    continue
                dtype = data.get("type")
                if dtype == "answer" and isinstance(data.get("sdp"), str):
                    self._pipeline.set_remote_answer(data["sdp"])
                elif (
                    dtype == "candidate"
                    and isinstance(data.get("candidate"), str)
                    and isinstance(data.get("sdpMLineIndex"), int)
                ):
                    self._pipeline.add_remote_ice_candidate(
                        data["sdpMLineIndex"], data["candidate"]
                    )
        except Exception as exc:
            await self._send_json_safe({"type": "error", "message": str(exc)})
        finally:
            await self.close()

    async def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        if self._clip_task is not None:
            self._clip_task.cancel()
            try:
                await self._clip_task
            except BaseException:
                pass
        if self._input is not None:
            try:
                self._input.close()
            except Exception:
                pass
        if self._pipeline is not None:
            try:
                self._pipeline.close()
            except Exception:
                pass
        if self._capture is not None:
            try:
                self._capture.close()
            except Exception:
                pass


async def default_session_handler(
    ws: web.WebSocketResponse,
    config: LanBridgeConfig,
    encoder: Optional[EncoderSpec] = None,
) -> None:
    """Default WSS handler spinning up a live GStreamer + Capture + Input session."""
    chosen = encoder or probe_best_encoder(dry_run=True)
    if chosen is None:
        await ws.send_str(
            json.dumps(
                {
                    "type": "error",
                    "message": "No working GStreamer H.264 encoder found. Run doctor.py to check plugins.",
                }
            )
        )
        return
    session = LanBridgeSession(ws, config, chosen)
    await session.run()
