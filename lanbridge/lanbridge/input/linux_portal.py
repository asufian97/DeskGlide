"""Wayland RemoteDesktop portal input backend delegating to LinuxPortalBackend."""

from __future__ import annotations

from lanbridge.capture.linux_portal import LinuxPortalBackend
from lanbridge.input.base import InputBackend
from lanbridge.protocol import WEB_BUTTON_TO_EVDEV


class LinuxPortalInputBackend(InputBackend):
    """Injects pointer and keyboard events over an authorized xdg-desktop-portal session."""

    def __init__(self, portal: LinuxPortalBackend) -> None:
        self._portal = portal
        self._scroll_v_acc = 0.0
        self._scroll_h_acc = 0.0

    def move_abs(self, x_norm: int, y_norm: int) -> None:
        self._portal.notify_pointer_motion_absolute(x_norm, y_norm)

    def move_rel(self, dx: int, dy: int) -> None:
        self._portal.notify_pointer_motion(float(dx), float(dy))

    def mouse_button(self, web_button: int, pressed: bool) -> None:
        ev_btn = WEB_BUTTON_TO_EVDEV.get(web_button)
        if ev_btn is not None:
            self._portal.notify_pointer_button(ev_btn, pressed)

    def scroll(self, dx: float, dy: float) -> None:
        # Browser WheelEvent: deltaY > 0 is down -> portal discrete vertical steps are +down / -up
        self._scroll_v_acc += dy / 100.0
        v_steps = int(self._scroll_v_acc)
        if v_steps != 0:
            self._scroll_v_acc -= v_steps
            self._portal.notify_pointer_axis_discrete(0, v_steps)

        self._scroll_h_acc += dx / 100.0
        h_steps = int(self._scroll_h_acc)
        if h_steps != 0:
            self._scroll_h_acc -= h_steps
            self._portal.notify_pointer_axis_discrete(1, h_steps)

    def key_event(self, evdev_code: int, pressed: bool) -> None:
        self._portal.notify_keyboard_keycode(evdev_code, pressed)

    def close(self) -> None:
        # Portal lifetime is owned by capture backend
        return None
