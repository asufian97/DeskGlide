"""Linux `evdev`/`uinput` fallback input backend."""

from __future__ import annotations

from lanbridge.input.base import InputBackend
from lanbridge.protocol import WEB_BUTTON_TO_EVDEV, WEB_CODE_TO_EVDEV


class LinuxUinputBackend(InputBackend):
    """Injects input via kernel `/dev/uinput` virtual keyboard & mouse devices."""

    def __init__(self, width: int = 1920, height: int = 1080) -> None:
        from evdev import AbsInfo, UInput, ecodes

        self._ecodes = ecodes
        self._width = max(1, width)
        self._height = max(1, height)
        self._scroll_v_acc = 0.0
        self._scroll_h_acc = 0.0

        kbd_caps = {ecodes.EV_KEY: sorted(set(WEB_CODE_TO_EVDEV.values()))}
        self._kbd = UInput(kbd_caps, name="lanbridge-virtual-keyboard")

        mouse_caps = {
            ecodes.EV_KEY: list(WEB_BUTTON_TO_EVDEV.values()),
            ecodes.EV_REL: [
                ecodes.REL_X,
                ecodes.REL_Y,
                ecodes.REL_WHEEL,
                ecodes.REL_HWHEEL,
            ],
            ecodes.EV_ABS: [
                (ecodes.ABS_X, AbsInfo(value=0, min=0, max=65535, fuzz=0, flat=0, resolution=0)),
                (ecodes.ABS_Y, AbsInfo(value=0, min=0, max=65535, fuzz=0, flat=0, resolution=0)),
            ],
        }
        self._mouse = UInput(mouse_caps, name="lanbridge-virtual-mouse")

    def move_abs(self, x_norm: int, y_norm: int) -> None:
        e = self._ecodes
        self._mouse.write(e.EV_ABS, e.ABS_X, max(0, min(65535, int(x_norm))))
        self._mouse.write(e.EV_ABS, e.ABS_Y, max(0, min(65535, int(y_norm))))
        self._mouse.syn()

    def move_rel(self, dx: int, dy: int) -> None:
        e = self._ecodes
        if dx:
            self._mouse.write(e.EV_REL, e.REL_X, int(dx))
        if dy:
            self._mouse.write(e.EV_REL, e.REL_Y, int(dy))
        if dx or dy:
            self._mouse.syn()

    def mouse_button(self, web_button: int, pressed: bool) -> None:
        ev_btn = WEB_BUTTON_TO_EVDEV.get(web_button)
        if ev_btn is None:
            return
        e = self._ecodes
        self._mouse.write(e.EV_KEY, ev_btn, 1 if pressed else 0)
        self._mouse.syn()

    def scroll(self, dx: float, dy: float) -> None:
        e = self._ecodes
        sent = False
        # Linux REL_WHEEL: +1 up, -1 down (opposite browser WheelEvent deltaY)
        self._scroll_v_acc += -dy / 100.0
        v_steps = int(self._scroll_v_acc)
        if v_steps != 0:
            self._scroll_v_acc -= v_steps
            self._mouse.write(e.EV_REL, e.REL_WHEEL, v_steps)
            sent = True

        self._scroll_h_acc += dx / 100.0
        h_steps = int(self._scroll_h_acc)
        if h_steps != 0:
            self._scroll_h_acc -= h_steps
            self._mouse.write(e.EV_REL, e.REL_HWHEEL, h_steps)
            sent = True

        if sent:
            self._mouse.syn()

    def key_event(self, evdev_code: int, pressed: bool) -> None:
        e = self._ecodes
        self._kbd.write(e.EV_KEY, int(evdev_code), 1 if pressed else 0)
        self._kbd.syn()

    def close(self) -> None:
        for dev in (self._kbd, self._mouse):
            try:
                dev.close()
            except Exception:
                pass
