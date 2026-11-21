"""Persistent local configuration for LanBridge (XDG on Linux, AppData on Windows)."""

from __future__ import annotations

import json
import os
import secrets
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Optional

from lanbridge.protocol import QUALITY_PRESETS


def config_dir() -> Path:
    """Return platform-appropriate configuration directory for LanBridge."""
    if os.name == "nt":
        base = Path(os.environ.get("APPDATA") or Path.home() / "AppData" / "Roaming")
    else:
        base = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
    return base / "lanbridge"


def cert_dir(base_dir: Optional[Path] = None) -> Path:
    """Return directory storing auto-generated local TLS certificate and key."""
    return (base_dir or config_dir()) / "certs"


def default_config_path() -> Path:
    """Return default config JSON path."""
    return config_dir() / "config.json"


def generate_pin() -> str:
    """Generate a random 6-digit numeric pairing PIN (100000-999999)."""
    return f"{secrets.randbelow(900_000) + 100_000:06d}"


def generate_session_secret() -> str:
    """Generate a 32-byte hex secret used for HMAC-signed session tokens."""
    return secrets.token_hex(32)


@dataclass
class LanBridgeConfig:
    port: int = 8443
    pin: str = ""
    session_secret: str = ""
    quality: str = "balanced"
    fps: int = 60
    portal_restore_token: Optional[str] = None
    clipboard_sync: bool = True

    def ensure_defaults(self) -> bool:
        """Populate missing PIN / session secret or invalid quality. Returns True if modified."""
        dirty = False
        if not self.pin or len(self.pin) != 6 or not self.pin.isdigit():
            self.pin = generate_pin()
            dirty = True
        if not self.session_secret or len(self.session_secret) < 32:
            self.session_secret = generate_session_secret()
            dirty = True
        if self.quality not in QUALITY_PRESETS:
            self.quality = "balanced"
            dirty = True
        if not (1 <= int(self.port) <= 65535):
            self.port = 8443
            dirty = True
        if not (15 <= int(self.fps) <= 120):
            self.fps = 60
            dirty = True
        return dirty


def load_config(path: Optional[Path] = None) -> LanBridgeConfig:
    """Load configuration from disk, creating defaults if absent or malformed."""
    cfg_path = path or default_config_path()
    cfg = LanBridgeConfig()
    dirty = False

    if cfg_path.exists():
        try:
            raw = json.loads(cfg_path.read_text(encoding="utf-8"))
            if isinstance(raw, dict):
                known_fields = {f for f in LanBridgeConfig.__dataclass_fields__}
                filtered = {k: v for k, v in raw.items() if k in known_fields}
                cfg = LanBridgeConfig(**filtered)
        except Exception:
            dirty = True
    else:
        dirty = True

    if cfg.ensure_defaults():
        dirty = True

    if dirty:
        save_config(cfg, cfg_path)
    return cfg


def save_config(cfg: LanBridgeConfig, path: Optional[Path] = None) -> None:
    """Atomically persist configuration to disk with restrictive permissions on POSIX."""
    cfg_path = path or default_config_path()
    cfg_path.parent.mkdir(parents=True, exist_ok=True)
    tmp_path = cfg_path.with_suffix(".json.tmp")
    payload = json.dumps(asdict(cfg), indent=2) + "\n"
    tmp_path.write_text(payload, encoding="utf-8")
    if os.name == "posix":
        try:
            os.chmod(cfg_path.parent, 0o700)
            os.chmod(tmp_path, 0o600)
        except OSError:
            pass
    os.replace(tmp_path, cfg_path)
    if os.name == "posix":
        try:
            os.chmod(cfg_path, 0o600)
        except OSError:
            pass
