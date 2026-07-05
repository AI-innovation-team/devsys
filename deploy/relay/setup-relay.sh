#!/usr/bin/env bash
# 在公网中继上运行（sudo）。建隧道专用账号 + 开 GatewayPorts，仅用于端口转发。
# 中继只做纯 TCP 转发，很轻，不跑任何重服务。
set -euo pipefail
RELAY_USER="${RELAY_USER:-relay}"

if ! id "$RELAY_USER" &>/dev/null; then
  sudo useradd -m -s /usr/sbin/nologin "$RELAY_USER"
fi
sudo mkdir -p "/home/$RELAY_USER/.ssh"
sudo touch "/home/$RELAY_USER/.ssh/authorized_keys"
sudo chmod 700 "/home/$RELAY_USER/.ssh"
sudo chmod 600 "/home/$RELAY_USER/.ssh/authorized_keys"
sudo chown -R "$RELAY_USER:$RELAY_USER" "/home/$RELAY_USER/.ssh"

# 让 gateway 的 ssh -R 0.0.0.0:PORT 能对外绑定
if ! grep -qE '^\s*GatewayPorts\s+clientspecified' /etc/ssh/sshd_config; then
  echo 'GatewayPorts clientspecified' | sudo tee -a /etc/ssh/sshd_config >/dev/null
  (sudo systemctl reload sshd 2>/dev/null || sudo systemctl reload ssh 2>/dev/null || true)
fi
echo "✓ relay 就绪：账号 '$RELAY_USER'，GatewayPorts=clientspecified"
