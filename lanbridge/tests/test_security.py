"""Tests for LanBridge local TLS certs, PIN rate limiting, session tokens, and LAN ICE filtering."""

from __future__ import annotations

from cryptography import x509

from lanbridge.security import (
    PinRateLimiter,
    create_session_token,
    ensure_tls_cert,
    filter_lan_ice_candidate,
    is_private_lan_ip,
    verify_pin,
    verify_session_token,
)


def test_private_lan_ip_strictness():
    assert is_private_lan_ip("192.168.10.180")
    assert is_private_lan_ip("10.0.5.42")
    assert is_private_lan_ip("172.16.0.1")
    assert is_private_lan_ip("172.31.255.254")
    assert is_private_lan_ip("127.0.0.1")
    assert is_private_lan_ip("::1")
    assert is_private_lan_ip("::ffff:192.168.10.116")

    # Reject link-local (169.254.x.x), public IPs, CGNAT, multicast
    assert not is_private_lan_ip("169.254.74.119")
    assert not is_private_lan_ip("8.8.8.8")
    assert not is_private_lan_ip("172.32.0.1")
    assert not is_private_lan_ip("100.64.0.1")
    assert not is_private_lan_ip("0.0.0.0")
    assert not is_private_lan_ip("224.0.0.251")


def test_ice_candidate_lan_only_filter():
    # Allowed host LAN candidates (including .local mDNS)
    assert filter_lan_ice_candidate(
        "candidate:1 1 UDP 2122260223 192.168.10.180 51234 typ host"
    )
    assert filter_lan_ice_candidate(
        "a=candidate:2 1 udp 2113937151 abcdef.local 49152 typ host generation 0"
    )

    # Rejected: external STUN/TURN, link-local 169.254.x.x, or public IP host
    assert not filter_lan_ice_candidate(
        "candidate:3 1 UDP 1686052607 203.0.113.5 55000 typ srflx raddr 192.168.10.180 rport 51234"
    )
    assert not filter_lan_ice_candidate(
        "candidate:4 1 UDP 41885439 198.51.100.2 3478 typ relay"
    )
    assert not filter_lan_ice_candidate(
        "candidate:5 1 UDP 2122260223 169.254.74.119 52000 typ host"
    )
    assert not filter_lan_ice_candidate(
        "candidate:6 1 UDP 2122260223 1.1.1.1 52000 typ host"
    )


def test_tls_cert_generation_and_san_refresh(tmp_path):
    cert_path, key_path, fp1 = ensure_tls_cert(tmp_path, ["192.168.10.180"])
    assert cert_path.exists() and key_path.exists()
    assert len(fp1.split(":")) == 32

    # Same IP reuses existing certificate
    _, _, fp2 = ensure_tls_cert(tmp_path, ["192.168.10.180"])
    assert fp2 == fp1

    # Changing LAN IP triggers automatic SAN regeneration
    cert_path2, _, fp3 = ensure_tls_cert(tmp_path, ["192.168.1.99"])
    assert fp3 != fp1
    cert = x509.load_pem_x509_certificate(cert_path2.read_bytes())
    san = cert.extensions.get_extension_for_class(x509.SubjectAlternativeName).value
    ips = {str(ip) for ip in san.get_values_for_type(x509.IPAddress)}
    assert "192.168.1.99" in ips
    assert "127.0.0.1" in ips


def test_pin_verification_rate_limiter_and_tokens():
    assert verify_pin("482910", "482910")
    assert not verify_pin("482910", "482911")

    now = [100.0]
    limiter = PinRateLimiter(max_attempts=5, lockout_seconds=60, now_fn=lambda: now[0])
    for i in range(4):
        assert limiter.record_failure("192.168.10.116") == 0
        assert limiter.retry_after("192.168.10.116") == 0

    assert limiter.record_failure("192.168.10.116") == 60
    assert limiter.retry_after("192.168.10.116") == 60

    now[0] += 61.0
    assert limiter.retry_after("192.168.10.116") == 0

    secret = "0123456789abcdef" * 4
    tok = create_session_token(secret, ttl_seconds=60, now_epoch=1000)
    assert verify_session_token(secret, tok, now_epoch=1050)
    assert not verify_session_token(secret, tok, now_epoch=1061)
    assert not verify_session_token(secret, tok[:-1] + ("0" if tok[-1] != "0" else "1"), now_epoch=1050)
