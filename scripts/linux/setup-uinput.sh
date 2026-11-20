#!/usr/bin/env bash
set -euo pipefail

# One-time setup script for DeskGlide on Zorin OS / Ubuntu / Linux.
# - Grants immediate non-root access to /dev/uinput via TAG+="uaccess" (no re-login needed for Client mode!)
# - Adds user to the `input` group for reading /dev/input/event* (Host mode).

if [[ $EUID -ne 0 ]]; then
    echo "Re-running with sudo..."
    exec sudo -E bash "$0" "$@"
fi

TARGET_USER="${SUDO_USER:-$(logname 2>/dev/null || echo "")}"
if [[ -z "$TARGET_USER" || "$TARGET_USER" == "root" ]]; then
    echo "Could not determine your non-root username automatically."
    echo "Run: sudo SUDO_USER=<your-username> bash $0"
    exit 1
fi

echo "==> Ensuring 'input' group exists and adding user '$TARGET_USER'..."
getent group input >/dev/null || groupadd --system input
usermod -aG input "$TARGET_USER"

echo "==> Loading 'uinput' kernel module and enabling on boot..."
modprobe uinput || true
cat <<'EOF' >/etc/modules-load.d/deskglide.conf
# Load uinput at boot so DeskGlide virtual mouse/keyboard work without sudo
uinput
EOF

echo "==> Installing udev rule /etc/udev/rules.d/70-deskglide.rules..."
rm -f /etc/udev/rules.d/99-deskglide.rules
cat <<'EOF' >/etc/udev/rules.d/70-deskglide.rules
# Allow active local desktop sessions (uaccess) and members of 'input' group
# to use /dev/uinput for DeskGlide without root.
KERNEL=="uinput", SUBSYSTEM=="misc", MODE="0660", GROUP="input", TAG+="uaccess", OPTIONS+="static_node=uinput"
EOF

echo "==> Reloading udev rules..."
udevadm control --reload-rules
udevadm trigger --subsystem-match=misc --sysname-match=uinput || true
udevadm trigger --subsystem-match=input || true

echo
echo "Done!"
echo "  * Client mode (/dev/uinput) is active immediately."
echo "  * For Host mode (/dev/input/event*), log out and back in ONCE if 'groups' does not yet list 'input'."
echo "  * If UFW firewall is enabled on Zorin OS, allow DeskGlide's UDP port:"
echo "      sudo ufw allow 24910/udp comment 'DeskGlide encrypted QUIC'"
