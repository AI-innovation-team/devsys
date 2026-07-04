#!/usr/bin/env bash
# 在中继上运行（sudo）。从 stdin 读 gateway 的隧道公钥，登记到隧道账号。
set -euo pipefail
RELAY_USER="${RELAY_USER:-relay}"
AK="/home/$RELAY_USER/.ssh/authorized_keys"
KEY="$(cat)"
[ -n "$KEY" ] || { echo "✗ 未收到公钥"; exit 1; }
if ! sudo grep -qF "$KEY" "$AK" 2>/dev/null; then
  echo "$KEY" | sudo tee -a "$AK" >/dev/null
fi
sudo chown "$RELAY_USER:$RELAY_USER" "$AK"
sudo chmod 600 "$AK"
echo "✓ 已登记隧道公钥到 '$RELAY_USER'"
