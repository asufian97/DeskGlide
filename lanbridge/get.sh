#!/usr/bin/env bash
set -euo pipefail

REPO_URL="https://github.com/asufian97/DeskGlide.git"
INSTALL_DIR="$HOME/DeskGlide"

echo "=== Installing LanBridge from GitHub ==="

if ! command -v git >/dev/null 2>&1; then
  sudo apt-get update
  sudo apt-get install -y git
fi

if [ -d "$INSTALL_DIR/.git" ]; then
  echo "Updating existing checkout in $INSTALL_DIR..."
  git -C "$INSTALL_DIR" pull --ff-only
else
  git clone --depth 1 "$REPO_URL" "$INSTALL_DIR"
fi

bash "$INSTALL_DIR/lanbridge/install-linux.sh"
