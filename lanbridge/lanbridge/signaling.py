"""HTTPS Web Viewer server, 6-digit PIN auth gate, and LAN-only WSS signaling."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Awaitable, Callable, Optional

from aiohttp import web

from lanbridge.config import LanBridgeConfig
from lanbridge.security import (
    PinRateLimiter,
    create_session_token,
    is_private_lan_ip,
    verify_pin,
    verify_session_token,
)

COOKIE_NAME = "lanbridge_session"
WEB_DIR = Path(__file__).resolve().parent / "web"

SessionHandler = Callable[[web.WebSocketResponse, LanBridgeConfig], Awaitable[None]]


def _client_ip(request: web.Request) -> str:
    peer = request.remote or ""
    if not peer and request.transport is not None:
        info = request.transport.get_extra_info("peername")
        if isinstance(info, tuple) and info:
            peer = str(info[0])
    return peer


def _is_authenticated(request: web.Request, config: LanBridgeConfig) -> bool:
    token = request.cookies.get(COOKIE_NAME, "")
    return verify_session_token(config.session_secret, token)


@web.middleware
async def lan_only_and_headers_middleware(
    request: web.Request,
    handler: Callable[[web.Request], Awaitable[web.StreamResponse]],
) -> web.StreamResponse:
    peer_ip = _client_ip(request)
    if peer_ip and not is_private_lan_ip(peer_ip):
        raise web.HTTPForbidden(text="Access restricted strictly to private LAN clients.")

    resp = await handler(request)
    resp.headers.setdefault("X-Content-Type-Options", "nosniff")
    resp.headers.setdefault("X-Frame-Options", "DENY")
    resp.headers.setdefault("Referrer-Policy", "no-referrer")
    resp.headers.setdefault("Cache-Control", "no-store")
    resp.headers.setdefault(
        "Content-Security-Policy",
        "default-src 'self'; connect-src 'self' wss: https:; style-src 'self'; script-src 'self'; img-src 'self' data:;",
    )
    return resp


def create_app(
    config: LanBridgeConfig,
    session_handler: Optional[SessionHandler] = None,
    host_info: Optional[dict[str, Any]] = None,
    rate_limiter: Optional[PinRateLimiter] = None,
    web_dir: Optional[Path] = None,
) -> web.Application:
    """Build aiohttp Application serving Web Viewer, PIN auth API, and WSS /ws."""
    assets_dir = (web_dir or WEB_DIR).resolve()
    limiter = rate_limiter or PinRateLimiter()
    info = dict(host_info or {})

    app = web.Application(middlewares=[lan_only_and_headers_middleware])

    async def index(_request: web.Request) -> web.FileResponse:
        return web.FileResponse(assets_dir / "index.html")

    async def static_asset(request: web.Request) -> web.StreamResponse:
        rel = request.match_info.get("name", "")
        if not rel or "/" in rel or "\\" in rel or ".." in rel:
            raise web.HTTPNotFound()
        target = (assets_dir / rel).resolve()
        if target.parent != assets_dir or not target.is_file():
            raise web.HTTPNotFound()
        return web.FileResponse(target)

    async def api_status(request: web.Request) -> web.Response:
        authed = _is_authenticated(request, config)
        payload: dict[str, Any] = {"authenticated": authed}
        if authed:
            payload["host"] = info
        return web.json_response(payload)

    async def api_auth(request: web.Request) -> web.Response:
        peer_ip = _client_ip(request) or "unknown"
        retry = limiter.retry_after(peer_ip)
        if retry > 0:
            return web.json_response(
                {"ok": False, "error": f"Too many attempts. Retry in {retry}s.", "retry_after": retry},
                status=429,
                headers={"Retry-After": str(retry)},
            )

        try:
            body = await request.json()
        except Exception:
            return web.json_response({"ok": False, "error": "Invalid JSON body."}, status=400)

        pin = str(body.get("pin", "")) if isinstance(body, dict) else ""
        if not verify_pin(config.pin, pin):
            lockout = limiter.record_failure(peer_ip)
            if lockout > 0:
                return web.json_response(
                    {
                        "ok": False,
                        "error": f"Incorrect PIN. Locked for {lockout}s.",
                        "retry_after": lockout,
                    },
                    status=429,
                    headers={"Retry-After": str(lockout)},
                )
            return web.json_response({"ok": False, "error": "Incorrect 6-digit PIN."}, status=401)

        limiter.record_success(peer_ip)
        token = create_session_token(config.session_secret)
        resp = web.json_response({"ok": True, "host": info})
        resp.set_cookie(
            COOKIE_NAME,
            token,
            max_age=7 * 24 * 3600,
            httponly=True,
            secure=request.secure,
            samesite="Strict",
            path="/",
        )
        return resp

    async def api_logout(_request: web.Request) -> web.Response:
        resp = web.json_response({"ok": True})
        resp.del_cookie(COOKIE_NAME, path="/")
        return resp

    async def ws_endpoint(request: web.Request) -> web.StreamResponse:
        if not _is_authenticated(request, config):
            return web.json_response({"error": "Authentication required"}, status=401)

        ws = web.WebSocketResponse(heartbeat=15.0)
        await ws.prepare(request)

        if session_handler is not None:
            await session_handler(ws, config)
        else:
            from lanbridge.session import default_session_handler

            await default_session_handler(ws, config)

        if not ws.closed:
            await ws.close()
        return ws

    app.router.add_get("/", index)
    app.router.add_get("/static/{name}", static_asset)
    app.router.add_get("/api/status", api_status)
    app.router.add_post("/api/auth", api_auth)
    app.router.add_post("/api/logout", api_logout)
    app.router.add_get("/ws", ws_endpoint)
    return app


def build_ssl_context(cert_path: Path, key_path: Path) -> Any:
    """Create a TLS 1.2/1.3 SSLContext for the local HTTPS/WSS server."""
    import ssl

    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.minimum_version = ssl.TLSVersion.TLSv1_2
    ctx.load_cert_chain(certfile=str(cert_path), keyfile=str(key_path))
    return ctx
