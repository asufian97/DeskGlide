"""Combined xdg-desktop-portal (RemoteDesktop + ScreenCast) backend for Linux Wayland."""

from __future__ import annotations

import os
import secrets
import threading
from typing import Any, Callable, Optional

from lanbridge.capture.base import CaptureBackend, CaptureDescriptor

PORTAL_BUS_NAME = "org.freedesktop.portal.Desktop"
PORTAL_OBJECT_PATH = "/org/freedesktop/portal/desktop"
PORTAL_REQUEST_IFACE = "org.freedesktop.portal.Request"
PORTAL_SESSION_IFACE = "org.freedesktop.portal.Session"
REMOTEDESKTOP_IFACE = "org.freedesktop.portal.RemoteDesktop"
SCREENCAST_IFACE = "org.freedesktop.portal.ScreenCast"


def _load_gio():
    import gi

    gi.require_version("Gio", "2.0")
    gi.require_version("GLib", "2.0")
    from gi.repository import Gio, GLib

    return Gio, GLib


class LinuxPortalBackend(CaptureBackend):
    """Uses combined RemoteDesktop + ScreenCast portal with persistent restore token."""

    def __init__(
        self,
        restore_token: Optional[str] = None,
        on_restore_token: Optional[Callable[[str], None]] = None,
    ) -> None:
        self._restore_token = restore_token
        self._on_restore_token = on_restore_token
        self._bus: Any = None
        self._session_handle: Optional[str] = None
        self._stream_node_id: Optional[int] = None
        self._pw_fd: Optional[int] = None
        self._width: int = 1920
        self._height: int = 1080
        self._lock = threading.Lock()

    @property
    def session_handle(self) -> Optional[str]:
        return self._session_handle

    @property
    def stream_node_id(self) -> Optional[int]:
        return self._stream_node_id

    @property
    def stream_size(self) -> tuple[int, int]:
        return (self._width, self._height)

    def _next_token(self, prefix: str) -> str:
        return f"lanbridge_{prefix}_{secrets.token_hex(6)}"

    def _call_portal(
        self,
        iface: str,
        method: str,
        params: Any,
        token: str,
        timeout_sec: int = 120,
    ) -> dict[str, Any]:
        Gio, GLib = _load_gio()
        assert self._bus is not None
        sender_name = self._bus.get_unique_name().lstrip(":").replace(".", "_")
        expected_req_path = f"/org/freedesktop/portal/desktop/request/{sender_name}/{token}"

        result_holder: dict[str, Any] = {}
        done = threading.Event()
        sub_id: Optional[int] = None

        def on_signal(_conn, _sender, object_path, _iface, _signal, parameters, *_args):
            if object_path != expected_req_path:
                return
            code, results = parameters.unpack()
            result_holder["code"] = int(code)
            result_holder["results"] = results or {}
            done.set()

        sub_id = self._bus.signal_subscribe(
            PORTAL_BUS_NAME,
            PORTAL_REQUEST_IFACE,
            "Response",
            None,
            None,
            Gio.DBusSignalFlags.NONE,
            on_signal,
        )
        try:
            reply = self._bus.call_sync(
                PORTAL_BUS_NAME,
                PORTAL_OBJECT_PATH,
                iface,
                method,
                params,
                GLib.VariantType("(o)"),
                Gio.DBusCallFlags.NONE,
                10_000,
                None,
            )
            req_path = reply.unpack()[0]
            if req_path != expected_req_path:
                expected_req_path = req_path

            ctx = GLib.MainContext.default()
            deadline_iters = timeout_sec * 50
            for _ in range(deadline_iters):
                if done.is_set():
                    break
                while ctx.pending():
                    ctx.iteration(False)
                if not done.is_set():
                    done.wait(0.02)

            if not done.is_set():
                raise TimeoutError(f"Portal call {iface}.{method} timed out")

            code = result_holder.get("code", 2)
            if code != 0:
                raise RuntimeError(f"Portal {iface}.{method} denied or failed (response={code})")
            return result_holder.get("results", {})
        finally:
            if sub_id is not None:
                try:
                    self._bus.signal_unsubscribe(sub_id)
                except Exception:
                    pass

    def start(self) -> CaptureDescriptor:
        Gio, GLib = _load_gio()
        self._bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)

        # 1. Create RemoteDesktop session (supports both ScreenCast + Input on Wayland)
        create_tok = self._next_token("create")
        sess_tok = self._next_token("session")
        create_res = self._call_portal(
            REMOTEDESKTOP_IFACE,
            "CreateSession",
            GLib.Variant(
                "(a{sv})",
                (
                    {
                        "handle_token": GLib.Variant("s", create_tok),
                        "session_handle_token": GLib.Variant("s", sess_tok),
                    },
                ),
            ),
            create_tok,
        )
        self._session_handle = create_res.get("session_handle")
        if not self._session_handle:
            raise RuntimeError("Portal did not return a session_handle")

        # 2. SelectDevices (keyboard=1 | pointer=2, persist_mode=2 with saved token if available)
        dev_tok = self._next_token("dev")
        dev_opts: dict[str, Any] = {
            "handle_token": GLib.Variant("s", dev_tok),
            "types": GLib.Variant("u", 1 | 2),
            "persist_mode": GLib.Variant("u", 2),
        }
        if self._restore_token:
            dev_opts["restore_token"] = GLib.Variant("s", self._restore_token)

        self._call_portal(
            REMOTEDESKTOP_IFACE,
            "SelectDevices",
            GLib.Variant("(oa{sv})", (self._session_handle, dev_opts)),
            dev_tok,
        )

        # 3. SelectSources on ScreenCast for the SAME session handle
        src_tok = self._next_token("src")
        self._call_portal(
            SCREENCAST_IFACE,
            "SelectSources",
            GLib.Variant(
                "(oa{sv})",
                (
                    self._session_handle,
                    {
                        "handle_token": GLib.Variant("s", src_tok),
                        "types": GLib.Variant("u", 1),        # Monitor capture
                        "multiple": GLib.Variant("b", False),
                        "cursor_mode": GLib.Variant("u", 2),  # Embedded cursor
                    },
                ),
            ),
            src_tok,
        )

        # 4. Start combined session (prompts once if no valid restore_token)
        start_tok = self._next_token("start")
        start_res = self._call_portal(
            REMOTEDESKTOP_IFACE,
            "Start",
            GLib.Variant(
                "(osa{sv})",
                (
                    self._session_handle,
                    "",
                    {"handle_token": GLib.Variant("s", start_tok)},
                ),
            ),
            start_tok,
        )

        new_token = start_res.get("restore_token")
        if isinstance(new_token, str) and new_token and new_token != self._restore_token:
            self._restore_token = new_token
            if self._on_restore_token is not None:
                self._on_restore_token(new_token)

        streams = start_res.get("streams") or []
        if not streams:
            raise RuntimeError("Portal ScreenCast returned no streams")

        node_id, props = streams[0]
        self._stream_node_id = int(node_id)
        if isinstance(props, dict) and "size" in props:
            size = props["size"]
            if isinstance(size, (tuple, list)) and len(size) == 2:
                self._width = max(1, int(size[0]))
                self._height = max(1, int(size[1]))

        # 5. Open PipeWire remote FD for GStreamer pipewiresrc
        out_val, fd_list = self._bus.call_with_unix_fd_list_sync(
            PORTAL_BUS_NAME,
            PORTAL_OBJECT_PATH,
            SCREENCAST_IFACE,
            "OpenPipeWireRemote",
            GLib.Variant("(oa{sv})", (self._session_handle, {})),
            GLib.VariantType("(h)"),
            Gio.DBusCallFlags.NONE,
            10_000,
            None,
            None,
        )
        fd_idx = out_val.unpack()[0]
        raw_fd = fd_list.get(fd_idx)
        self._pw_fd = os.dup(raw_fd)

        return CaptureDescriptor(
            backend="linux_portal",
            display_server="wayland",
            width=self._width,
            height=self._height,
            gst_source_name="pipewiresrc",
            pipewire_fd=self._pw_fd,
            pipewire_node_id=self._stream_node_id,
            extra_props={"do-timestamp": True, "keepalive-time": 1000},
        )

    # -- Direct RemoteDesktop D-Bus input methods used by LinuxPortalInputBackend --

    def notify_pointer_motion(self, dx: float, dy: float) -> None:
        if not self._bus or not self._session_handle:
            return
        Gio, GLib = _load_gio()
        with self._lock:
            self._bus.call_sync(
                PORTAL_BUS_NAME,
                PORTAL_OBJECT_PATH,
                REMOTEDESKTOP_IFACE,
                "NotifyPointerMotion",
                GLib.Variant("(oa{sv}dd)", (self._session_handle, {}, float(dx), float(dy))),
                None,
                Gio.DBusCallFlags.NONE,
                1000,
                None,
            )

    def notify_pointer_motion_absolute(self, x_norm: int, y_norm: int) -> None:
        if not self._bus or not self._session_handle or self._stream_node_id is None:
            return
        Gio, GLib = _load_gio()
        x = (max(0, min(65535, x_norm)) / 65535.0) * float(self._width)
        y = (max(0, min(65535, y_norm)) / 65535.0) * float(self._height)
        with self._lock:
            self._bus.call_sync(
                PORTAL_BUS_NAME,
                PORTAL_OBJECT_PATH,
                REMOTEDESKTOP_IFACE,
                "NotifyPointerMotionAbsolute",
                GLib.Variant(
                    "(oa{sv}udd)",
                    (self._session_handle, {}, int(self._stream_node_id), x, y),
                ),
                None,
                Gio.DBusCallFlags.NONE,
                1000,
                None,
            )

    def notify_pointer_button(self, evdev_button: int, pressed: bool) -> None:
        if not self._bus or not self._session_handle:
            return
        Gio, GLib = _load_gio()
        with self._lock:
            self._bus.call_sync(
                PORTAL_BUS_NAME,
                PORTAL_OBJECT_PATH,
                REMOTEDESKTOP_IFACE,
                "NotifyPointerButton",
                GLib.Variant(
                    "(oa{sv}iu)",
                    (self._session_handle, {}, int(evdev_button), 1 if pressed else 0),
                ),
                None,
                Gio.DBusCallFlags.NONE,
                1000,
                None,
            )

    def notify_pointer_axis_discrete(self, axis: int, steps: int) -> None:
        if not self._bus or not self._session_handle or steps == 0:
            return
        Gio, GLib = _load_gio()
        with self._lock:
            self._bus.call_sync(
                PORTAL_BUS_NAME,
                PORTAL_OBJECT_PATH,
                REMOTEDESKTOP_IFACE,
                "NotifyPointerAxisDiscrete",
                GLib.Variant(
                    "(oa{sv}ui)",
                    (self._session_handle, {}, int(axis), int(steps)),
                ),
                None,
                Gio.DBusCallFlags.NONE,
                1000,
                None,
            )

    def notify_keyboard_keycode(self, evdev_keycode: int, pressed: bool) -> None:
        if not self._bus or not self._session_handle:
            return
        Gio, GLib = _load_gio()
        with self._lock:
            self._bus.call_sync(
                PORTAL_BUS_NAME,
                PORTAL_OBJECT_PATH,
                REMOTEDESKTOP_IFACE,
                "NotifyKeyboardKeycode",
                GLib.Variant(
                    "(oa{sv}iu)",
                    (self._session_handle, {}, int(evdev_keycode), 1 if pressed else 0),
                ),
                None,
                Gio.DBusCallFlags.NONE,
                1000,
                None,
            )

    def close(self) -> None:
        if self._pw_fd is not None:
            try:
                os.close(self._pw_fd)
            except OSError:
                pass
            self._pw_fd = None

        if self._bus and self._session_handle:
            try:
                Gio, _ = _load_gio()
                self._bus.call_sync(
                    PORTAL_BUS_NAME,
                    self._session_handle,
                    PORTAL_SESSION_IFACE,
                    "Close",
                    None,
                    None,
                    Gio.DBusCallFlags.NONE,
                    1000,
                    None,
                )
            except Exception:
                pass
            self._session_handle = None
