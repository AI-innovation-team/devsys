#!/usr/bin/env bash
# 在公网中继上运行：把一台控制面的隧道公钥登记进来，允许它建反向隧道。
# 用法：
#   sudo bash authorize.sh 'ssh-ed25519 AAAA... root@host'
#   或   control-plane 的公钥通过管道：  ... | sudo bash authorize.sh
set -euo pipefail
RELAY_USER="${RELAY_USER:-relay}"
KEY="${1:-$(cat)}"
[ -n "${KEY// }" ] || { echo "✗ 没有收到公钥"; exit 1; }

AK="/home/${RELAY_USER}/.ssh/authorized_keys"
install -d -m 700 -o "${RELAY_USER}" -g "${RELAY_USER}" "/home/${RELAY_USER}/.ssh"
touch "$AK"
grep -qxF "$KEY" "$AK" || echo "$KEY" >> "$AK"
chown "${RELAY_USER}:${RELAY_USER}" "$AK"; chmod 600 "$AK"
echo "✓ 已登记控制面公钥。"
