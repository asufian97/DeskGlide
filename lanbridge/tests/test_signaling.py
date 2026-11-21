"""Integration tests for aiohttp PIN auth, lockout, cookie gate, and WSS signaling."""

from __future__ import annotations

import json
import ssl
from pathlib import Path

from aiohttp import WSMsgType
from aiohttp.test_utils import TestClient, TestServer

from lanbridge.config import LanBridgeConfig
from lanbridge.security import PinRateLimiter, ensure_tls_cert
from lanbridge.signaling import build_ssl_context, create_app


async def test_auth_gate_rate_limit_and_ws_signaling():
    cfg = LanBridgeConfig(
        port=8443,
        pin="654321",
        session_secret="test_secret_" + "a" * 32,
        quality="balanced",
    )
    limiter = PinRateLimiter(max_attempts=5, lockout_seconds=60)

    async def fake_session(ws, _conf):
        await ws.send_str(json.dumps({"type": "offer", "sdp": "v=0\r\n"}))
        async for msg in ws:
            if msg.type == WSMsgType.TEXT:
                data = json.loads(msg.data)
                if data.get("type") == "answer":
                    await ws.send_str(json.dumps({"type": "ack", "sdp": data["sdp"]}))
                    break

    app = create_app(cfg, session_handler=fake_session, host_info={"encoder": "vah264enc"}, rate_limiter=limiter)
    async with TestClient(TestServer(app)) as client:
        # 1. Root page serves HTML without external CDNs
        index_resp = await client.get("/")
        assert index_resp.status == 200
        html = await index_resp.text()
        assert "LanBridge" in html
        assert "https://" not in html and "http://" not in html

        # 2. Unauthenticated WS must be rejected with 401
        status_before = await (await client.get("/api/status")).json()
        assert status_before["authenticated"] is False

        ws_fail = await client.get("/ws")
        assert ws_fail.status == 401

        # 3. Wrong PIN triggers 401 and 5th attempt triggers 429 lockout
        for _ in range(4):
            r = await client.post("/api/auth", json={"pin": "000000"})
            assert r.status == 401
        r5 = await client.post("/api/auth", json={"pin": "000000"})
        assert r5.status == 429

        # Reset limiter and log in cleanly with valid PIN
        limiter.record_success("127.0.0.1")
        login = await client.post("/api/auth", json={"pin": "654321"})
        assert login.status == 200
        body = await login.json()
        assert body["ok"] is True

        status_after = await (await client.get("/api/status")).json()
        assert status_after["authenticated"] is True
        assert status_after["host"]["encoder"] == "vah264enc"

        # 4. Authenticated WSS /ws relays offer/answer
        async with client.ws_connect("/ws") as ws:
            offer = json.loads(await ws.receive_str())
            assert offer == {"type": "offer", "sdp": "v=0\r\n"}
            await ws.send_str(json.dumps({"type": "answer", "sdp": "v=0\r\na=sendrecv\r\n"}))
            ack = json.loads(await ws.receive_str())
            assert ack["type"] == "ack"


def test_ssl_context_loads_generated_cert(tmp_path: Path):
    cert_path, key_path, _ = ensure_tls_cert(tmp_path, ["192.168.10.180"])
    ctx = build_ssl_context(cert_path, key_path)
    assert isinstance(ctx, ssl.SSLContext)
