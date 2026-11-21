(function () {
  "use strict";

  const authView = document.getElementById("authView");
  const stageView = document.getElementById("stageView");
  const authForm = document.getElementById("authForm");
  const pinInput = document.getElementById("pinInput");
  const authError = document.getElementById("authError");

  const toolbar = document.getElementById("toolbar");
  const connPill = document.getElementById("connPill");
  const modeScreenBtn = document.getElementById("modeScreenBtn");
  const modeKvmBtn = document.getElementById("modeKvmBtn");
  const qualitySelect = document.getElementById("qualitySelect");
  const pointerLockBtn = document.getElementById("pointerLockBtn");
  const fullscreenBtn = document.getElementById("fullscreenBtn");
  const clipboardBtn = document.getElementById("clipboardBtn");
  const keyframeBtn = document.getElementById("keyframeBtn");
  const reconnectBtn = document.getElementById("reconnectBtn");
  const logoutBtn = document.getElementById("logoutBtn");

  const hudFps = document.getElementById("hudFps");
  const hudRtt = document.getElementById("hudRtt");
  const hudBitrate = document.getElementById("hudBitrate");
  const hudEncoder = document.getElementById("hudEncoder");

  const inputSurface = document.getElementById("inputSurface");
  const remoteVideo = document.getElementById("remoteVideo");
  const kvmOverlay = document.getElementById("kvmOverlay");
  const kvmPointerLockBtn = document.getElementById("kvmPointerLockBtn");
  const kvmResumeVideoBtn = document.getElementById("kvmResumeVideoBtn");
  const statusOverlay = document.getElementById("statusOverlay");
  const statusMessage = document.getElementById("statusMessage");

  let ws = null;
  let pc = null;
  let moveDc = null;
  let controlDc = null;
  let currentMode = "screen";
  let clipboardEnabled = true;
  let pinTimer = null;
  let statsTimer = null;
  let prevBytesReceived = 0;
  let prevStatsTs = 0;
  let lastDcRttMs = null;

  function pinToolbarBriefly(ms = 4000) {
    toolbar.classList.add("pinned");
    if (pinTimer) clearTimeout(pinTimer);
    pinTimer = setTimeout(() => toolbar.classList.remove("pinned"), ms);
  }

  function setStatus(label, cls, overlayText = null) {
    connPill.textContent = label;
    connPill.className = "pill " + cls;
    if (overlayText) {
      statusMessage.textContent = overlayText;
      statusOverlay.hidden = false;
    } else {
      statusOverlay.hidden = true;
    }
  }

  function isLanIceCandidate(candidateStr) {
    if (!candidateStr) return false;
    const raw = candidateStr.startsWith("a=") ? candidateStr.slice(2) : candidateStr;
    const parts = raw.trim().split(/\s+/);
    if (parts.length < 8 || parts[6] !== "typ") return false;
    if (parts[7].toLowerCase() !== "host") return false;
    const ip = parts[4];
    if (ip.endsWith(".local")) return true;
    if (ip === "127.0.0.1" || ip === "::1") return true;
    if (/^10\./.test(ip) || /^192\.168\./.test(ip)) return true;
    const m172 = /^172\.(\d+)\./.exec(ip);
    if (m172) {
      const oct = parseInt(m172[1], 10);
      return oct >= 16 && oct <= 31;
    }
    return false;
  }

  function sendControl(obj) {
    if (controlDc && controlDc.readyState === "open") {
      controlDc.send(JSON.stringify(obj));
    }
  }

  function sendMouseAbs(xNorm, yNorm) {
    if (!moveDc || moveDc.readyState !== "open") return;
    const buf = new ArrayBuffer(9);
    const dv = new DataView(buf);
    dv.setUint8(0, 1); // MOUSE_ABS
    dv.setInt16(1, 0);
    dv.setInt16(3, 0);
    dv.setUint16(5, Math.max(0, Math.min(65535, Math.round(xNorm))));
    dv.setUint16(7, Math.max(0, Math.min(65535, Math.round(yNorm))));
    moveDc.send(buf);
  }

  function sendMouseRel(dx, dy) {
    if (!moveDc || moveDc.readyState !== "open") return;
    const buf = new ArrayBuffer(9);
    const dv = new DataView(buf);
    dv.setUint8(0, 2); // MOUSE_REL
    dv.setInt16(1, Math.max(-32768, Math.min(32767, Math.round(dx))));
    dv.setInt16(3, Math.max(-32768, Math.min(32767, Math.round(dy))));
    dv.setUint16(5, 0);
    dv.setUint16(7, 0);
    moveDc.send(buf);
  }

  function mapClientToVideoNorm(clientX, clientY) {
    const rect = remoteVideo.getBoundingClientRect();
    const vw = remoteVideo.videoWidth || rect.width;
    const vh = remoteVideo.videoHeight || rect.height;
    if (!rect.width || !rect.height || !vw || !vh) return null;

    const videoAspect = vw / vh;
    const rectAspect = rect.width / rect.height;
    let renderW = rect.width;
    let renderH = rect.height;
    let offsetX = 0;
    let offsetY = 0;

    if (videoAspect > rectAspect) {
      renderH = rect.width / videoAspect;
      offsetY = (rect.height - renderH) / 2;
    } else {
      renderW = rect.height * videoAspect;
      offsetX = (rect.width - renderW) / 2;
    }

    const relX = (clientX - rect.left - offsetX) / renderW;
    const relY = (clientY - rect.top - offsetY) / renderH;
    const nx = Math.max(0, Math.min(1, relX)) * 65535;
    const ny = Math.max(0, Math.min(1, relY)) * 65535;
    return [nx, ny];
  }

  function applyModeUI(mode) {
    currentMode = mode;
    if (mode === "kvm") {
      modeKvmBtn.classList.add("active");
      modeScreenBtn.classList.remove("active");
      kvmOverlay.hidden = false;
      setStatus("KVM Only", "pill-kvm");
    } else {
      modeScreenBtn.classList.add("active");
      modeKvmBtn.classList.remove("active");
      kvmOverlay.hidden = true;
      setStatus("Live", "pill-live");
    }
    pinToolbarBriefly(2500);
  }

  function setMode(mode) {
    applyModeUI(mode);
    sendControl({ type: "mode", mode });
  }

  async function enterFullscreenWithKeyboardLock() {
    try {
      if (!document.fullscreenElement) {
        await document.documentElement.requestFullscreen();
      }
      if (navigator.keyboard && typeof navigator.keyboard.lock === "function") {
        await navigator.keyboard.lock([
          "AltLeft", "AltRight", "Tab", "MetaLeft", "MetaRight", "Escape"
        ]);
      }
      fullscreenBtn.textContent = "Exit Fullscreen";
      pinToolbarBriefly(2500);
    } catch (_) {
      // Browser declined fullscreen or Keyboard Lock unsupported
    }
  }

  async function toggleFullscreen() {
    if (document.fullscreenElement) {
      if (navigator.keyboard && typeof navigator.keyboard.unlock === "function") {
        navigator.keyboard.unlock();
      }
      await document.exitFullscreen();
    } else {
      await enterFullscreenWithKeyboardLock();
    }
  }

  document.addEventListener("fullscreenchange", () => {
    fullscreenBtn.textContent = document.fullscreenElement ? "Exit Fullscreen" : "Fullscreen";
    if (!document.fullscreenElement && navigator.keyboard && typeof navigator.keyboard.unlock === "function") {
      navigator.keyboard.unlock();
    }
  });

  function togglePointerLock() {
    if (document.pointerLockElement === inputSurface) {
      document.exitPointerLock();
    } else {
      inputSurface.requestPointerLock();
    }
  }

  document.addEventListener("pointerlockchange", () => {
    const locked = document.pointerLockElement === inputSurface;
    pointerLockBtn.classList.toggle("active-outline", locked);
    pointerLockBtn.textContent = locked ? "Unlock Mouse" : "Pointer Lock";
    if (!locked) {
      sendControl({ type: "release_all" });
    }
  });

  async function syncClipboardToRemote() {
    if (!clipboardEnabled || !navigator.clipboard || !navigator.clipboard.readText) return;
    try {
      const text = await navigator.clipboard.readText();
      if (text) sendControl({ type: "clipboard", text });
    } catch (_) {}
  }

  async function applyRemoteClipboard(text) {
    if (!clipboardEnabled || !navigator.clipboard || !navigator.clipboard.writeText) return;
    try {
      await navigator.clipboard.writeText(text);
    } catch (_) {}
  }

  function startStatsPolling() {
    if (statsTimer) clearInterval(statsTimer);
    prevBytesReceived = 0;
    prevStatsTs = performance.now();

    statsTimer = setInterval(async () => {
      if (controlDc && controlDc.readyState === "open") {
        sendControl({ type: "ping", ts: performance.now() });
      }
      if (!pc) return;
      try {
        const report = await pc.getStats();
        let fps = null;
        let bytes = null;
        let iceRttMs = null;

        report.forEach((stat) => {
          if (stat.type === "inbound-rtp" && stat.kind === "video") {
            if (typeof stat.framesPerSecond === "number") fps = stat.framesPerSecond;
            if (typeof stat.bytesReceived === "number") bytes = stat.bytesReceived;
          }
          if (stat.type === "candidate-pair" && stat.nominated && typeof stat.currentRoundTripTime === "number") {
            iceRttMs = Math.round(stat.currentRoundTripTime * 1000);
          }
        });

        const now = performance.now();
        const dt = (now - prevStatsTs) / 1000;
        if (bytes !== null && dt > 0.2) {
          const mbps = ((bytes - prevBytesReceived) * 8) / (dt * 1_000_000);
          prevBytesReceived = bytes;
          prevStatsTs = now;
          hudBitrate.textContent = currentMode === "kvm" ? "0.0 Mbps" : `${Math.max(0, mbps).toFixed(1)} Mbps`;
        }
        hudFps.textContent = currentMode === "kvm" ? "0 fps (KVM)" : (fps !== null ? `${Math.round(fps)} fps` : "-- fps");
        const rtt = lastDcRttMs !== null ? lastDcRttMs : iceRttMs;
        hudRtt.textContent = rtt !== null ? `${rtt} ms` : "-- ms";
      } catch (_) {}
    }, 1000);
  }

  function teardownPeer() {
    if (statsTimer) {
      clearInterval(statsTimer);
      statsTimer = null;
    }
    if (moveDc) { try { moveDc.close(); } catch (_) {} moveDc = null; }
    if (controlDc) { try { controlDc.close(); } catch (_) {} controlDc = null; }
    if (pc) { try { pc.close(); } catch (_) {} pc = null; }
    if (ws) { try { ws.close(); } catch (_) {} ws = null; }
    remoteVideo.srcObject = null;
  }

  function connectSession() {
    teardownPeer();
    setStatus("Connecting", "pill-muted", "Connecting over LAN WebRTC…");

    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    ws = new WebSocket(`${proto}//${location.host}/ws`);

    pc = new RTCPeerConnection({ iceServers: [] });

    pc.ontrack = (ev) => {
      if (ev.streams && ev.streams[0]) {
        remoteVideo.srcObject = ev.streams[0];
      } else {
        remoteVideo.srcObject = new MediaStream([ev.track]);
      }
      remoteVideo.play().catch(() => {});
      if (currentMode !== "kvm") {
        setStatus("Buffering", "pill-muted", "Waiting for first video frame…");
      }
      pinToolbarBriefly(3500);
    };

    remoteVideo.onloadeddata = () => {
      if (currentMode !== "kvm") {
        setStatus("Live", "pill-live");
      }
    };

    remoteVideo.onplaying = () => {
      if (currentMode !== "kvm") {
        setStatus("Live", "pill-live");
      }
    };

    pc.onicecandidate = (ev) => {
      if (!ev.candidate || !ev.candidate.candidate) return;
      if (!isLanIceCandidate(ev.candidate.candidate)) return;
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({
          type: "candidate",
          sdpMLineIndex: ev.candidate.sdpMLineIndex ?? 0,
          candidate: ev.candidate.candidate,
        }));
      }
    };

    pc.oniceconnectionstatechange = () => {
      if (!pc) return;
      const st = pc.iceConnectionState;
      if (st === "checking") {
        if (currentMode !== "kvm" && !remoteVideo.videoWidth) {
          setStatus("Connecting ICE", "pill-muted", "Connecting ICE over local Wi-Fi…");
        }
      } else if (st === "connected" || st === "completed") {
        sendControl({ type: "keyframe" });
        if (currentMode !== "kvm" && !remoteVideo.videoWidth) {
          setStatus("Waiting Frame", "pill-muted", "Connected over LAN — waiting for first H.264 keyframe…");
        }
      } else if (st === "failed") {
        setStatus("ICE Failed", "pill-err", "LAN WebRTC ICE failed — check UFW/firewall UDP between PCs and click Reconnect.");
        pinToolbarBriefly(8000);
      }
    };

    pc.onconnectionstatechange = () => {
      if (!pc) return;
      if (pc.connectionState === "failed" || pc.connectionState === "disconnected") {
        setStatus("Reconnecting", "pill-err", "WebRTC connection lost — click Reconnect.");
      }
    };

    pc.ondatachannel = (ev) => {
      const ch = ev.channel;
      if (ch.label === "input-move") {
        moveDc = ch;
        moveDc.binaryType = "arraybuffer";
      } else if (ch.label === "control") {
        controlDc = ch;
        controlDc.onopen = () => {
          startStatsPolling();
          sendControl({ type: "keyframe" });
          inputSurface.focus();
        };
        controlDc.onmessage = (msgEv) => {
          try {
            const data = JSON.parse(msgEv.data);
            if (data.type === "meta") {
              hudEncoder.textContent = data.hardware ? `${data.encoder} (HW)` : `${data.encoder} (CPU)`;
              if (data.quality) qualitySelect.value = data.quality;
              if (data.mode) applyModeUI(data.mode);
            } else if (data.type === "mode_ack") {
              applyModeUI(data.mode);
            } else if (data.type === "clipboard" && typeof data.text === "string") {
              applyRemoteClipboard(data.text);
            } else if (data.type === "pong" && typeof data.ts === "number") {
              lastDcRttMs = Math.max(0, Math.round(performance.now() - data.ts));
            }
          } catch (_) {}
        };
      }
    };

    ws.onmessage = async (ev) => {
      let msg;
      try {
        msg = JSON.parse(ev.data);
      } catch (_) {
        return;
      }
      if (msg.type === "offer" && pc) {
        await pc.setRemoteDescription({ type: "offer", sdp: msg.sdp });
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        ws.send(JSON.stringify({ type: "answer", sdp: answer.sdp }));
      } else if (msg.type === "candidate" && pc && isLanIceCandidate(msg.candidate)) {
        try {
          await pc.addIceCandidate({
            sdpMLineIndex: msg.sdpMLineIndex,
            candidate: msg.candidate,
          });
        } catch (_) {}
      } else if (msg.type === "error") {
        setStatus("Error", "pill-err", `Host error: ${msg.message}`);
        pinToolbarBriefly(6000);
      }
    };

    ws.onclose = (ev) => {
      if (ev.code === 1008 || ev.code === 4401) {
        showAuth();
      } else {
        setStatus("Disconnected", "pill-err", "Signaling connection closed.");
      }
    };
  }

  // Pointer & Keyboard event capture
  inputSurface.addEventListener("mousemove", (ev) => {
    if (document.pointerLockElement === inputSurface) {
      if (ev.movementX || ev.movementY) {
        sendMouseRel(ev.movementX, ev.movementY);
      }
      return;
    }
    if (currentMode === "kvm") {
      // Relative fallback if pointer isn't locked on KVM pad
      if (ev.movementX || ev.movementY) {
        sendMouseRel(ev.movementX, ev.movementY);
      }
      return;
    }
    const coords = mapClientToVideoNorm(ev.clientX, ev.clientY);
    if (coords) sendMouseAbs(coords[0], coords[1]);
  });

  inputSurface.addEventListener("mousedown", (ev) => {
    ev.preventDefault();
    inputSurface.focus();
    sendControl({ type: "mouse_btn", button: ev.button, pressed: true });
  });

  inputSurface.addEventListener("mouseup", (ev) => {
    ev.preventDefault();
    sendControl({ type: "mouse_btn", button: ev.button, pressed: false });
  });

  inputSurface.addEventListener("contextmenu", (ev) => ev.preventDefault());

  inputSurface.addEventListener("wheel", (ev) => {
    ev.preventDefault();
    sendControl({ type: "wheel", dx: ev.deltaX, dy: ev.deltaY });
  }, { passive: false });

  window.addEventListener("keydown", (ev) => {
    if (stageView.hidden) return;
    if (ev.ctrlKey && ev.altKey && ev.code === "KeyM") {
      ev.preventDefault();
      setMode(currentMode === "screen" ? "kvm" : "screen");
      return;
    }
    if (ev.repeat) return;
    if (window.LanBridgeKeymap.isSupportedCode(ev.code)) {
      ev.preventDefault();
      sendControl({ type: "key", code: ev.code, pressed: true });
    }
  });

  window.addEventListener("keyup", (ev) => {
    if (stageView.hidden) return;
    if (window.LanBridgeKeymap.isSupportedCode(ev.code)) {
      ev.preventDefault();
      sendControl({ type: "key", code: ev.code, pressed: false });
    }
  });

  window.addEventListener("blur", () => {
    if (!stageView.hidden) sendControl({ type: "release_all" });
  });
  window.addEventListener("focus", () => {
    if (!stageView.hidden) syncClipboardToRemote();
  });
  document.addEventListener("visibilitychange", () => {
    if (document.hidden && !stageView.hidden) sendControl({ type: "release_all" });
  });

  // UI Buttons
  modeScreenBtn.addEventListener("click", () => setMode("screen"));
  modeKvmBtn.addEventListener("click", () => setMode("kvm"));
  kvmResumeVideoBtn.addEventListener("click", () => setMode("screen"));
  kvmPointerLockBtn.addEventListener("click", togglePointerLock);
  pointerLockBtn.addEventListener("click", togglePointerLock);
  fullscreenBtn.addEventListener("click", toggleFullscreen);

  qualitySelect.addEventListener("change", () => {
    sendControl({ type: "quality", preset: qualitySelect.value });
  });

  clipboardBtn.addEventListener("click", () => {
    clipboardEnabled = !clipboardEnabled;
    clipboardBtn.textContent = clipboardEnabled ? "Clipboard: ON" : "Clipboard: OFF";
    clipboardBtn.classList.toggle("active-outline", clipboardEnabled);
  });

  keyframeBtn.addEventListener("click", () => sendControl({ type: "keyframe" }));
  reconnectBtn.addEventListener("click", connectSession);
  logoutBtn.addEventListener("click", async () => {
    teardownPeer();
    await fetch("/api/logout", { method: "POST" });
    showAuth();
  });

  function showAuth() {
    teardownPeer();
    stageView.hidden = true;
    authView.hidden = false;
    pinInput.value = "";
    pinInput.focus();
  }

  function showStage() {
    authView.hidden = true;
    stageView.hidden = false;
    pinToolbarBriefly(4500);
    connectSession();
  }

  authForm.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    authError.hidden = true;
    const pin = pinInput.value.trim();
    try {
      const resp = await fetch("/api/auth", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ pin }),
      });
      const data = await resp.json();
      if (resp.ok && data.ok) {
        showStage();
      } else {
        authError.textContent = data.error || "Authentication failed.";
        authError.hidden = false;
      }
    } catch (_) {
      authError.textContent = "Cannot reach Host server.";
      authError.hidden = false;
    }
  });

  async function init() {
    try {
      const resp = await fetch("/api/status");
      const data = await resp.json();
      if (data.authenticated) {
        showStage();
        return;
      }
    } catch (_) {}
    showAuth();
  }

  init();
})();
