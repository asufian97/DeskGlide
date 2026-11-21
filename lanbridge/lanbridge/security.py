"""Local TLS certificate generation, LAN-only IP/ICE enforcement, PIN auth, and session tokens."""

from __future__ import annotations

import datetime as dt
import hashlib
import hmac
import ipaddress
import os
import secrets
import socket
import threading
import time
from pathlib import Path
from typing import Callable, Iterable, Optional

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import ExtendedKeyUsageOID, NameOID

_RFC1918_NETWORKS = (
    ipaddress.IPv4Network("10.0.0.0/8"),
    ipaddress.IPv4Network("172.16.0.0/12"),
    ipaddress.IPv4Network("192.168.0.0/16"),
)


def is_private_lan_ip(ip_str: str) -> bool:
    """Return True strictly for RFC 1918 private LAN IPv4 addresses and loopback (::1 / 127.0.0.1)."""
    if not ip_str:
        return False
    clean = ip_str.split("%", 1)[0].strip()
    try:
        ip = ipaddress.ip_address(clean)
    except ValueError:
        return False

    if isinstance(ip, ipaddress.IPv6Address) and ip.ipv4_mapped is not None:
        ip = ip.ipv4_mapped

    if ip.is_link_local or ip.is_multicast or ip.is_unspecified:
        return False

    if isinstance(ip, ipaddress.IPv6Address):
        return ip.is_loopback or ip.is_private  # Allows ::1 and RFC 4193 fd00::/8 (excludes fe80::/10 link-local)

    if ip.is_loopback:
        return True

    return any(ip in net for net in _RFC1918_NETWORKS)


def detect_lan_ipv4s() -> list[str]:
    """Detect local RFC 1918 IPv4 addresses, prioritizing the default outbound interface IP."""
    candidates: list[str] = []

    for target in (("192.168.0.1", 80), ("10.0.0.1", 80), ("1.1.1.1", 80)):
        sock: Optional[socket.socket] = None
        try:
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.connect(target)
            ip = sock.getsockname()[0]
            if is_private_lan_ip(ip) and not ip.startswith("127.") and ip not in candidates:
                candidates.append(ip)
        except OSError:
            pass
        finally:
            if sock is not None:
                sock.close()

    try:
        for info in socket.getaddrinfo(socket.gethostname(), None, socket.AF_INET):
            ip = info[4][0]
            if is_private_lan_ip(ip) and not ip.startswith("127.") and ip not in candidates:
                candidates.append(ip)
    except OSError:
        pass

    return candidates


def filter_lan_ice_candidate(candidate_sdp: str) -> bool:
    """Allow only LAN host ICE candidates (private IPs, loopback, or .local mDNS)."""
    if not candidate_sdp:
        return False
    raw = candidate_sdp.strip()
    if raw.startswith("a="):
        raw = raw[2:]
    if not raw.startswith("candidate:"):
        return False

    parts = raw.split()
    # candidate:<foundation> <component> <proto> <priority> <ip> <port> typ <type> ...
    if len(parts) < 8 or parts[6] != "typ":
        return False

    host = parts[4].strip()
    cand_type = parts[7].strip().lower()
    if cand_type != "host":
        return False

    if host.lower().endswith(".local"):
        return True

    return is_private_lan_ip(host)


def format_cert_fingerprint(cert: x509.Certificate) -> str:
    """Return colon-delimited uppercase SHA-256 certificate fingerprint."""
    digest = cert.fingerprint(hashes.SHA256())
    return ":".join(f"{b:02X}" for b in digest)


def _existing_cert_valid(cert: x509.Certificate, required_ips: Iterable[str]) -> bool:
    now = dt.datetime.now(dt.timezone.utc)
    not_after = cert.not_valid_after_utc
    if not_after - now < dt.timedelta(days=7):
        return False

    try:
        san = cert.extensions.get_extension_for_class(x509.SubjectAlternativeName).value
    except x509.ExtensionNotFound:
        return False

    present_ips = {str(ip) for ip in san.get_values_for_type(x509.IPAddress)}
    present_dns = {d.lower() for d in san.get_values_for_type(x509.DNSName)}
    if "localhost" not in present_dns or "127.0.0.1" not in present_ips:
        return False

    for req_ip in required_ips:
        if req_ip not in present_ips:
            return False
    return True


def ensure_tls_cert(cert_dir: Path, lan_ips: Optional[list[str]] = None) -> tuple[Path, Path, str]:
    """Ensure a valid local self-signed ECDSA P-256 TLS cert covers all current LAN IPs."""
    cert_dir.mkdir(parents=True, exist_ok=True)
    if os.name == "posix":
        try:
            os.chmod(cert_dir, 0o700)
        except OSError:
            pass

    cert_path = cert_dir / "cert.pem"
    key_path = cert_dir / "key.pem"
    ips = [ip for ip in (lan_ips if lan_ips is not None else detect_lan_ipv4s()) if is_private_lan_ip(ip)]

    if cert_path.exists() and key_path.exists():
        try:
            cert = x509.load_pem_x509_certificate(cert_path.read_bytes())
            if _existing_cert_valid(cert, ips):
                return cert_path, key_path, format_cert_fingerprint(cert)
        except Exception:
            pass

    key = ec.generate_private_key(ec.SECP256R1())
    now = dt.datetime.now(dt.timezone.utc)
    hostname = socket.gethostname().strip() or "lanbridge"
    safe_label = "".join(c for c in hostname if c.isalnum() or c == "-").strip("-") or "lanbridge"

    subject = issuer = x509.Name(
        [
            x509.NameAttribute(NameOID.COMMON_NAME, "LanBridge Local Host"),
            x509.NameAttribute(NameOID.ORGANIZATION_NAME, "LanBridge"),
        ]
    )

    san_entries: list[x509.GeneralName] = [
        x509.DNSName("localhost"),
        x509.DNSName(f"{safe_label}.local"),
        x509.IPAddress(ipaddress.IPv4Address("127.0.0.1")),
        x509.IPAddress(ipaddress.IPv6Address("::1")),
    ]
    seen_ips = {"127.0.0.1", "::1"}
    for ip_str in ips:
        if ip_str not in seen_ips:
            seen_ips.add(ip_str)
            san_entries.append(x509.IPAddress(ipaddress.ip_address(ip_str)))

    cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - dt.timedelta(minutes=5))
        .not_valid_after(now + dt.timedelta(days=365))
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                key_encipherment=True,
                content_commitment=False,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=False,
                crl_sign=False,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(
            x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]),
            critical=False,
        )
        .add_extension(x509.SubjectAlternativeName(san_entries), critical=False)
        .sign(private_key=key, algorithm=hashes.SHA256())
    )

    key_pem = key.private_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PrivateFormat.PKCS8,
        encryption_algorithm=serialization.NoEncryption(),
    )
    cert_pem = cert.public_bytes(serialization.Encoding.PEM)

    tmp_key = key_path.with_suffix(".pem.tmp")
    tmp_key.write_bytes(key_pem)
    if os.name == "posix":
        try:
            os.chmod(tmp_key, 0o600)
        except OSError:
            pass
    os.replace(tmp_key, key_path)
    if os.name == "posix":
        try:
            os.chmod(key_path, 0o600)
        except OSError:
            pass

    cert_path.write_bytes(cert_pem)
    return cert_path, key_path, format_cert_fingerprint(cert)


def verify_pin(expected: str, provided: str) -> bool:
    """Constant-time verification of the 6-digit pairing PIN."""
    if not expected or not provided:
        return False
    return hmac.compare_digest(expected.strip().encode("utf-8"), provided.strip().encode("utf-8"))


class PinRateLimiter:
    """Per-IP brute-force protection for PIN authentication attempts."""

    def __init__(
        self,
        max_attempts: int = 5,
        lockout_seconds: int = 60,
        now_fn: Callable[[], float] = time.monotonic,
    ) -> None:
        self._max_attempts = max_attempts
        self._lockout_seconds = lockout_seconds
        self._now = now_fn
        self._lock = threading.Lock()
        self._failures: dict[str, list[float]] = {}
        self._locked_until: dict[str, float] = {}

    def retry_after(self, client_ip: str) -> int:
        """Return remaining lockout seconds for `client_ip`, or 0 if allowed."""
        now = self._now()
        with self._lock:
            until = self._locked_until.get(client_ip, 0.0)
            if now < until:
                return max(1, int(until - now + 0.999))
            if until and now >= until:
                self._locked_until.pop(client_ip, None)
                self._failures.pop(client_ip, None)
            return 0

    def record_failure(self, client_ip: str) -> int:
        """Record a failed PIN attempt and return lockout seconds if now locked."""
        now = self._now()
        with self._lock:
            cutoff = now - self._lockout_seconds
            recent = [t for t in self._failures.get(client_ip, []) if t >= cutoff]
            recent.append(now)
            self._failures[client_ip] = recent
            if len(recent) >= self._max_attempts:
                until = now + self._lockout_seconds
                self._locked_until[client_ip] = until
                return self._lockout_seconds
            return 0

    def record_success(self, client_ip: str) -> None:
        """Clear failure history after a successful authentication."""
        with self._lock:
            self._failures.pop(client_ip, None)
            self._locked_until.pop(client_ip, None)


def create_session_token(
    secret: str,
    ttl_seconds: int = 7 * 24 * 3600,
    now_epoch: Optional[int] = None,
) -> str:
    """Create an HMAC-SHA256 signed session token (`exp.nonce.sig`)."""
    now = int(time.time() if now_epoch is None else now_epoch)
    exp = now + ttl_seconds
    nonce = secrets.token_hex(16)
    payload = f"{exp}.{nonce}"
    sig = hmac.new(secret.encode("utf-8"), payload.encode("utf-8"), hashlib.sha256).hexdigest()
    return f"{payload}.{sig}"


def verify_session_token(
    secret: str,
    token: str,
    now_epoch: Optional[int] = None,
) -> bool:
    """Verify HMAC signature and expiry of a session token."""
    if not secret or not token:
        return False
    parts = token.strip().split(".")
    if len(parts) != 3:
        return False
    exp_str, nonce, sig = parts
    if not exp_str.isdigit() or not nonce or not sig:
        return False
    payload = f"{exp_str}.{nonce}"
    expected_sig = hmac.new(
        secret.encode("utf-8"), payload.encode("utf-8"), hashlib.sha256
    ).hexdigest()
    if not hmac.compare_digest(expected_sig, sig):
        return False
    now = int(time.time() if now_epoch is None else now_epoch)
    return now <= int(exp_str)
