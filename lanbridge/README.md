# LanBridge

**100% free, zero-cloud, ultra-low-latency LAN Remote Desktop & Hybrid Software KVM.**

Control your **Zorin OS (Linux)** laptop from your **Windows desktop Chrome or Edge browser** over local Wi-Fi — with **zero client software installed** on Windows — and toggle **KVM-Only Mode** anytime to pause video encoding (`0%` GPU/CPU video overhead) while keeping mouse, keyboard, and clipboard instant.

---

## Architecture Highlights

- **Hardware-Accelerated WebRTC (`GStreamer` + `webrtcbin`):** Encoding, RTP packetization, DTLS/SRTP encryption, and ICE run in native C/GPU threads — never slow Python loops.
- **Strictly LAN-Only & Zero Cloud:** Disables all external STUN/TURN servers, binds to LAN interfaces, and filters out non-LAN / link-local (`169.254.x.x`) ICE candidates.
- **Hybrid Mode (`Screen + Control` ↔ `KVM Only`):**
  - **Screen + Control:** Hardware-encoded 60 fps H.264 video stream + full input/clipboard.
  - **KVM Only (`Ctrl+Alt+M` or toolbar button):** Locks/pauses the video capture & encoder bin (`READY`, `0%` GPU/CPU video load) while keeping both WebRTC DataChannels alive (`input-move` unordered binary + `control` reliable JSON).
- **Zero-Install Web Viewer:** Self-hosted on `https://<lan-ip>:8443` with Fullscreen API, Pointer Lock toggle, **Keyboard Lock API** (`Alt+Tab` and `Super` go to Zorin OS instead of Windows), live Stats HUD (`FPS`, `RTT ms`, `Bitrate`, `Encoder`), and quality presets.

---

## Quick Start: Zorin OS (Host) → Windows Chrome/Edge (Viewer)

### 1. One-Line Install on Zorin OS (Laptop)

```bash
curl -fsSL https://raw.githubusercontent.com/asufian97/DeskGlide/main/lanbridge/get.sh | bash
```

Start LanBridge anytime with **one word** (or open **LanBridge** from your Zorin app menu):

```bash
lanbridge
```

*(Need to check your GPU encoder or ports? Run `lanbridge-doctor`.)*

The terminal prints your local viewer URL and 6-digit pairing PIN:

```text
================================================================
 LanBridge Host Agent  (zorin-laptop · wayland)
================================================================
 Viewer URL   : https://192.168.10.180:8443
 Pairing PIN  : 482910
 H.264 Codec  : vah264enc (GPU hardware)
================================================================
```

### 2. Connect from Windows Desktop (Zero Install)

1. Open **Chrome** or **Edge** on Windows and visit the `https://<lan-ip>:8443` URL printed on Linux.
2. Click **Advanced → Proceed** once to trust your locally auto-generated ECDSA certificate.
3. Enter the **6-digit PIN** from the Linux terminal.
4. On your **first connection only**, Zorin OS prompts to allow screen sharing + remote input (`xdg-desktop-portal`). Approve it once — LanBridge saves the `restore_token` (`persist_mode=2`) so future launches and reboots connect without re-prompting.
5. Click **Fullscreen** in the floating top toolbar (hover top center of the screen) so `Alt+Tab` and `Super` control Zorin OS directly.

---

## Hybrid Mode Shortcuts & Toolbar

- **`Ctrl+Alt+M` (or click `KVM Only`):** Pauses screen capture & H.264 encoding (`0%` video overhead) when looking straight at your laptop screen.
- **Top-center hover:** Reveals the floating toolbar (`Screen + Control` / `KVM Only`, `FPS` / `RTT` / `Mbps` / `Encoder` HUD, Quality preset, `Pointer Lock`, `Fullscreen`, `Clipboard`, `Keyframe`, `Reconnect`).
- **`Esc` (hold in Fullscreen):** Exits browser Fullscreen / releases Pointer Lock.

---

## Optional: Autostart on Zorin OS (`systemd --user`)

*(Assumes the repo lives at `~/lanbridge`; adjust `WorkingDirectory` and `ExecStart` inside `packaging/lanbridge.service` if placed elsewhere.)*

```bash
mkdir -p ~/.config/systemd/user
cp packaging/lanbridge.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now lanbridge
```

---

## Optional Reverse Mode: Windows Host → Linux Browser

To control your Windows desktop from Linux Chrome/Chromium, install Python 3.11+ and GStreamer 1.x MSVC (full install), then run:

```powershell
.\install-windows.ps1
.\.venv\Scripts\python.exe -m lanbridge
```

*(Make sure your Windows Wi-Fi profile is set to **Private** and third-party firewalls allow inbound TCP `8443` + local UDP WebRTC.)*

---

## Running Unit Tests

```bash
.venv/bin/pip install -e ".[dev]"
.venv/bin/pytest -v
```
