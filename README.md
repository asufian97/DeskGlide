<div align="center">
  <img src="packaging/linux/deskglide.svg" width="88" height="88" alt="DeskGlide logo" />
  <h1>DeskGlide</h1>
  <p>
    <strong>Use one wireless keyboard and mouse across Windows 11 and Linux (Wayland &amp; X11) over encrypted local Wi-Fi.</strong>
  </p>
  <p>
    <a href="https://github.com/asufian97/DeskGlide/releases/latest">
      <img src="https://img.shields.io/badge/Download-Latest%20Release-2dd4bf?style=for-the-badge" alt="Download Latest Release" />
    </a>
    <img src="https://img.shields.io/badge/Windows%2011-Supported-0078D4?style=for-the-badge&logo=windows" alt="Windows 11" />
    <img src="https://img.shields.io/badge/Linux-Wayland%20%26%20X11-FCC624?style=for-the-badge&logo=linux&logoColor=black" alt="Linux Wayland & X11" />
    <img src="https://img.shields.io/badge/License-MIT%20%2F%20Apache--2.0-slate?style=for-the-badge" alt="MIT / Apache-2.0" />
  </p>
</div>

---

## Download (Pick Your OS — 1 File Only)

> **You do NOT need to download the whole repository or install Rust.** Just download the **single file** for your operating system from the table below:

| Your Computer | Download (1 Click) | How to Install |
|---|---|---|
| **Windows 11** (Installer — Recommended) | [**DeskGlide-Windows-Setup-x64.exe**](https://github.com/asufian97/DeskGlide/releases/latest/download/DeskGlide-Windows-Setup-x64.exe) | Run the installer — adds Start Menu shortcut & Windows Firewall rule automatically |
| **Windows 11** (Portable ZIP) | [**DeskGlide-Windows-Portable-x64.zip**](https://github.com/asufian97/DeskGlide/releases/latest/download/DeskGlide-Windows-Portable-x64.zip) | Unzip and double-click `deskglide-app.exe` |
| **Zorin OS / Ubuntu / Debian** (`.deb`) | [**DeskGlide-Linux-Debian-Zorin-amd64.deb**](https://github.com/asufian97/DeskGlide/releases/latest/download/DeskGlide-Linux-Debian-Zorin-amd64.deb) | Double-click to install (or see terminal command below) |
| **Other Linux** (Fedora / Arch / etc.) | [**DeskGlide-Linux-Portable-x86_64.tar.gz**](https://github.com/asufian97/DeskGlide/releases/latest/download/DeskGlide-Linux-Portable-x86_64.tar.gz) | Extract, run `sudo ./setup-uinput.sh` once, then run `./deskglide-app` |

**Install or upgrade on Zorin OS / Ubuntu / Debian** (after downloading the `.deb` above):

```bash
sudo apt install ./DeskGlide-Linux-Debian-Zorin-amd64.deb
```

Need an older version or checksums? See the [**All Releases**](https://github.com/asufian97/DeskGlide/releases) page.

---

## Quick Start (30 Seconds)

1. **On your main PC** (the one with your wireless keyboard & mouse):
   - Open **DeskGlide**, select **Host (Share my keyboard &amp; mouse)**, pick which side your second PC sits on (**Left** / **Right** / **Top** / **Bottom**), and click **Start Sharing**.
   - *(Using the portable Windows ZIP? Click **"Allow Windows Firewall (UDP 24910)"** once inside the app.)*
2. **On your second PC:**
   - Open **DeskGlide** and select **Client (Control this PC)**.
   - Your Host PC appears automatically under **PCs Found on Your Wi-Fi** — click **Pair &amp; Connect** and type the 8-character pairing code shown on the Host.
3. **Glide seamlessly:**
   - Move your mouse past the chosen monitor edge — or press **`Ctrl+Alt+S`** — to jump between **Windows 11** and **Zorin OS / Linux** in either direction (**Windows ↔ Linux**, **Windows ↔ Windows**, **Linux ↔ Linux**).

*(After pairing once, DeskGlide remembers your trusted device and auto-reconnects silently whenever both PCs are on the same Wi-Fi.)*

---

## Why DeskGlide?

- **Works natively on Zorin OS Wayland &amp; X11:** Uses Linux `evdev` and virtual `uinput` devices so cursor movement, shortcuts, and scrolling work identically on Wayland (GNOME/Mutter) and X11.
- **Wireless-friendly &amp; ultra-low latency:** High-polling mouse movement is coalesced (`5 ms` default) and sent over QUIC datagrams (zero TCP head-of-line stutter), while clicks flush pending movement first for pixel-exact precision.
- **Encrypted by default:** All keystrokes, clicks, mouse deltas, and clipboard text travel over **TLS 1.3 (QUIC)** with SHA-256 certificate pinning and channel-bound mutual pairing-code authentication.
- **No stuck keys ever:** Switching monitors, disconnecting, or closing the app automatically releases every held modifier (`Ctrl`, `Alt`, `Shift`, `Win`/`Super`) and mouse button.
- **Shared clipboard:** Copy text on Windows and paste directly on Linux (and vice versa).
- **Lightweight native Rust:** ~9 MB desktop app (`deskglide-app`) with system tray support and zero Electron/Chromium bloat.

---

## Security &amp; Privacy

- **100% local LAN/Wi-Fi** — zero cloud servers, accounts, or telemetry.
- **Encrypted end-to-end** — TLS 1.3 mandatory on every packet (`UDP 24910`).
- **MITM protection** — every Host auto-generates its own ECDSA P-256 certificate pinned by SHA-256 fingerprint plus RFC 5705 channel-bound HMAC authentication against your 8-character pairing code.

---

## Troubleshooting &amp; FAQ

- **Client cannot see or connect to Host:**
  Make sure **UDP port `24910`** is allowed on the Host PC (`Allow Windows Firewall (UDP 24910)` button on Windows, or `sudo ufw allow 24910/udp` on Linux) and both devices are on the same local Wi-Fi/LAN subnet (not Guest Wi-Fi isolation).
- **Windows UAC / Task Manager / Admin windows don't respond:**
  Windows security (UIPI) blocks non-elevated input tools from controlling elevated Admin windows. Run DeskGlide as Administrator if you need to control elevated windows.
- **Linux permissions when switching Zorin OS to Host mode:**
  Installing the `.deb` grants immediate Client access (`/dev/uinput` via `uaccess`). If using Linux as the **Host** right after your very first install, log out and back in once so your account picks up the `input` group (or click **"Enable Permissions (1-Click)"** in the app).

---

<details>
<summary><strong>CLI Usage (Headless / Power Users)</strong></summary>

DeskGlide also includes a standalone CLI binary (`deskglide` / `deskglide.exe`):

```bash
# Host (owns the physical keyboard & mouse)
deskglide server --edge right --hotkey Ctrl+Alt+S --coalesce-ms 5

# Client (auto-reconnects, prints instructions from server output)
deskglide client 192.168.1.50:24910 --secret XXXX-XXXX --fingerprint AA:BB:... --auto-reconnect

# Show this machine's TLS SHA-256 certificate fingerprint
deskglide fingerprint
```

*(Double-clicking `deskglide.exe` with no arguments automatically opens `deskglide-app.exe`.)*

</details>

<details>
<summary><strong>Build from Source (Windows 11 &amp; Zorin OS / Linux)</strong></summary>

### Windows 11
```powershell
cargo build --release --workspace
# Outputs:
#   .\target\release\deskglide-app.exe  (GUI + System Tray)
#   .\target\release\deskglide.exe      (CLI)

# Create portable Windows ZIP + compact Linux source ZIP in dist\
powershell -ExecutionPolicy Bypass -File scripts\windows\package-portable.ps1
```

### Zorin OS / Ubuntu / Debian
```bash
sudo apt update && sudo apt install -y build-essential curl pkg-config \
  libx11-dev libxkbcommon-dev libwayland-dev libgl-dev libdbus-1-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

chmod +x scripts/linux/setup-uinput.sh
sudo ./scripts/linux/setup-uinput.sh
sudo ufw allow 24910/udp comment 'DeskGlide encrypted QUIC'

cargo build --release --workspace

# Optional: build a double-clickable .deb installer
cargo install cargo-deb --locked
cargo deb -p deskglide-gui
```

</details>

<details>
<summary><strong>Publishing a GitHub Release (For Maintainers)</strong></summary>

1. Tag a release and push it to GitHub:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
2. [GitHub Actions](.github/workflows/release.yml) automatically builds and attaches only the 4 single-file OS downloads (`DeskGlide-Windows-Setup-x64.exe`, `DeskGlide-Windows-Portable-x64.zip`, `DeskGlide-Linux-Debian-Zorin-amd64.deb`, `DeskGlide-Linux-Portable-x86_64.tar.gz`) directly to your Release page.

</details>

---

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
