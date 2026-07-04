#!/usr/bin/env bash
# 角色：relay —— 跑在「任意公网服务器」上（当前是阿里云 ECS）。
# 作用：把它变成反向隧道入口。极轻量：不装 Docker，只用系统自带的 sshd。
#
# 用法（在公网服务器上以 root 执行）：  sudo bash setup-relay.sh
set -euo pipefail

RELAY_USER="${RELAY_USER:-relay}"

echo "[1/3] 创建隧道专用账号：${RELAY_USER}"
if ! id "${RELAY_USER}" >/dev/null 2>&1; then
  useradd -m -s /bin/bash "${RELAY_USER}"
fi
install -d -m 700 -o "${RELAY_USER}" -g "${RELAY_USER}" "/home/${RELAY_USER}/.ssh"
touch "/home/${RELAY_USER}/.ssh/authorized_keys"
chown "${RELAY_USER}:${RELAY_USER}" "/home/${RELAY_USER}/.ssh/authorized_keys"
chmod 600 "/home/${RELAY_USER}/.ssh/authorized_keys"

echo "[2/3] 开启 GatewayPorts（让反向隧道能绑定到公网网卡）"
if grep -qE '^\s*GatewayPorts' /etc/ssh/sshd_config; then
  sed -i 's/^\s*GatewayPorts.*/GatewayPorts yes/' /etc/ssh/sshd_config
else
  echo 'GatewayPorts yes' >> /etc/ssh/sshd_config
fi
systemctl reload ssh 2>/dev/null || systemctl reload sshd 2>/dev/null || service ssh reload

echo "[3/3] 完成。"
cat <<EOF

下一步：
  把「控制面机器」的隧道公钥（/root/.ssh/relay_key.pub）追加到本机：
      /home/${RELAY_USER}/.ssh/authorized_keys
  之后控制面机器的 devsys-tunnel 服务即可把 Coder 端口反向暴露到本机公网。

注意：本机安全组/防火墙需放行对外端口（PUBLIC_PORT，默认 8443）。
EOF
