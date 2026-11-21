"""Base interface and descriptors for modular screen capture backends."""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from typing import Any, Optional


@dataclass
class CaptureDescriptor:
    """Describes the active GStreamer screen capture source and dimensions."""

    backend: str          # "linux_portal", "linux_x11", "windows"
    display_server: str   # "wayland", "x11", "windows"
    width: int
    height: int
    gst_source_name: str  # "pipewiresrc", "ximagesrc", "d3d11screencapturesrc"
    pipewire_fd: Optional[int] = None
    pipewire_node_id: Optional[int] = None
    extra_props: dict[str, Any] = field(default_factory=dict)


class CaptureBackend(ABC):
    """Abstract base class for OS screen capture providers."""

    @abstractmethod
    def start(self) -> CaptureDescriptor:
        """Initialize the capture source and return its GStreamer descriptor."""

    @abstractmethod
    def close(self) -> None:
        """Release any open OS handles, D-Bus sessions, or file descriptors."""
