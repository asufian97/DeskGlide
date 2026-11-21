#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "=== LanBridge Linux Setup (Zorin OS / Ubuntu / Debian) ==="

sudo apt-get update
sudo apt-get install -y \
  python3 \
  python3-venv \
  python3-gi \
  python3-gi-cairo \
  python3-gst-1.0 \
  python3-evdev \
  gir1.2-gstreamer-1.0 \
  gir1.2-gst-plugins-base-1.0 \
  gir1.2-gst-plugins-bad-1.0 \
  gstreamer1.0-tools \
  gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad \
  gstreamer1.0-plugins-ugly \
  gstreamer1.0-nice \
  gstreamer1.0-pipewire \
  gstreamer1.0-vaapi \
  libgstreamer1.0-dev \
  xdg-desktop-portal \
  wl-clipboard \
  xclip \
  libxtst6

echo ""
echo "=== Creating virtualenv with --system-site-packages (preserves python3-gi/Gst) ==="
python3 -m venv --system-site-packages .venv
.venv/bin/pip install --upgrade pip
.venv/bin/pip install -e .

echo "=== Installing global commands: lanbridge & lanbridge-doctor ==="
sudo ln -sf "$SCRIPT_DIR/.venv/bin/lanbridge" /usr/local/bin/lanbridge
sudo ln -sf "$SCRIPT_DIR/.venv/bin/lanbridge-doctor" /usr/local/bin/lanbridge-doctor

mkdir -p "$HOME/.local/share/applications"
cat > "$HOME/.local/share/applications/lanbridge.desktop" <<'EOF'
[Desktop Entry]
Name=LanBridge
Comment=Zero-Cloud LAN Remote Desktop & Hybrid KVM Host
Exec=/usr/local/bin/lanbridge
Terminal=true
Type=Application
Categories=Utility;Network;
Keywords=kvm;remote;webrtc;wayland;
EOF
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q "$HOME/.local/share/applications" || true
fi

if command -v ufw >/dev/null 2>&1; then
  if sudo ufw status 2>/dev/null | grep -q "Status: active"; then
    echo "=== Allowing TCP port 8443 through active UFW firewall ==="
    sudo ufw allow 8443/tcp comment 'LanBridge HTTPS/WSS'
  fi
fi

echo ""
echo "=== Running LanBridge System Doctor ==="
lanbridge-doctor || true

echo ""
echo "================================================================"
echo " Setup complete!"
echo "   Start anytime : lanbridge"
echo "   Diagnostics   : lanbridge-doctor"
echo " Or launch 'LanBridge' from your Zorin app menu."
echo "================================================================"
