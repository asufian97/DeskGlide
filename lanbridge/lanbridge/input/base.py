"""Base interface and stuck-key tracker for modular OS input backends."""

from __future__ import annotations

from abc import ABC, abstractmethod

from lanbridge.protocol import WEB_BUTTON_TO_EVDEV


class InputBackend(ABC):
    """Abstract base class for OS keyboard & mouse injection."""

    @abstractmethod
    def move_abs(self, x_norm: int, y_norm: int) -> None:
        """Move pointer to normalized screen coordinates (0..65535)."""

    @abstractmethod
    def move_rel(self, dx: int, dy: int) -> None:
        """Move pointer relatively by `(dx, dy)` pixels."""

    @abstractmethod
    def mouse_button(self, web_button: int, pressed: bool) -> None:
        """Press or release a mouse button (`0=left, 1=middle, 2=right, 3=back, 4=forward`)."""

    @abstractmethod
    def scroll(self, dx: float, dy: float) -> None:
        """Scroll pointer wheel (`dy > 0` scrolls down in browser WheelEvent)."""

    @abstractmethod
    def key_event(self, evdev_code: int, pressed: bool) -> None:
        """Press or release a keyboard key identified by canonical Linux evdev code."""

    @abstractmethod
    def close(self) -> None:
        """Release backend handles."""


class HeldInputTracker:
    """Tracks pressed keys/buttons to release every held input on blur or disconnect."""

    def __init__(self, backend: InputBackend) -> None:
        self._backend = backend
        self._held_keys: set[int] = set()
        self._held_buttons: set[int] = set()

    def move_abs(self, x_norm: int, y_norm: int) -> None:
        self._backend.move_abs(x_norm, y_norm)

    def move_rel(self, dx: int, dy: int) -> None:
        self._backend.move_rel(dx, dy)

    def mouse_button(self, web_button: int, pressed: bool) -> None:
        if web_button not in WEB_BUTTON_TO_EVDEV:
            return
        if pressed:
            self._held_buttons.add(web_button)
        else:
            self._held_buttons.discard(web_button)
        self._backend.mouse_button(web_button, pressed)

    def scroll(self, dx: float, dy: float) -> None:
        self._backend.scroll(dx, dy)

    def key_event(self, evdev_code: int, pressed: bool) -> None:
        if pressed:
            self._held_keys.add(evdev_code)
        else:
            self._held_keys.discard(evdev_code)
        self._backend.key_event(evdev_code, pressed)

    def release_all(self) -> None:
        """Release all currently pressed buttons and keys cleanly."""
        for btn in list(self._held_buttons):
            try:
                self._backend.mouse_button(btn, False)
            except Exception:
                pass
        self._held_buttons.clear()

        for code in sorted(self._held_keys, reverse=True):
            try:
                self._backend.key_event(code, False)
            except Exception:
                pass
        self._held_keys.clear()

    def close(self) -> None:
        self.release_all()
        self._backend.close()
