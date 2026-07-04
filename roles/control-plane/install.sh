#!/usr/bin/env bash
# 角色 control-plane 的一键安装。幂等，可重复运行。
# 在「任意一台被选为控制面的内网机器」上执行：
#   cp .env.example .env && vim .env      # 填好配置
#   bash install.sh
set -euo pipefail
cd "$(dirname "$0")"

[ -f .env ] || { echo "✗ 缺少 .env，先 cp .env.example .env 并填写"; exit 1; }
# 用 grep 提取需要的标量（不 source 整个文件：CADDY_DNS 带空格，source 会出错）
val() { grep -E "^$1=" .env | head -1 | cut -d= -f2- | sed 's/^"\(.*\)"$/\1/'; }
DEV_DOMAIN=$(val DEV_DOMAIN); PUBLIC_PORT=$(val PUBLIC_PORT)
PROVISIONER_PSK=$(val PROVISIONER_PSK); RELAY_HOST=$(val RELAY_HOST)
RELAY_SSH_USER=$(val RELAY_SSH_USER); RELAY_SSH_PORT=$(val RELAY_SSH_PORT)
for v in DEV_DOMAIN PUBLIC_PORT PROVISIONER_PSK RELAY_HOST RELAY_SSH_USER RELAY_SSH_PORT; do
  [ -n "$(eval echo \"\$$v\")" ] || { echo "✗ .env 缺少 $v"; exit 1; }
done

SUDO=""; [ "$(id -u)" -ne 0 ] && SUDO="sudo"
IMG="ghcr.io/coder/coder:latest"

echo "[1/5] 检查 Docker"
command -v docker >/dev/null || { echo "✗ 未装 Docker，请先安装"; exit 1; }

echo "[2/5] 确保 Coder 镜像（多镜像源依次尝试，兼容国内网络）"
if ! $SUDO docker image inspect "$IMG" >/dev/null 2>&1; then
  ok=""
  for src in \
    "ghcr.io/coder/coder:latest" \
    "ghcr.nju.edu.cn/coder/coder:latest" \
    "ghcr.dockerproxy.net/coder/coder:latest" \
    "ghcr.m.daocloud.io/coder/coder:latest"; do
    echo "  尝试 $src"
    if timeout 300 $SUDO docker pull "$src"; then
      [ "$src" = "$IMG" ] || $SUDO docker tag "$src" "$IMG"
      ok=1; break
    fi
  done
  [ -n "$ok" ] || { echo "✗ 所有镜像源都拉取失败"; exit 1; }
fi

echo "[3/5] 生成到公网中继的隧道密钥"
$SUDO mkdir -p /root/.ssh
$SUDO test -f /root/.ssh/relay_key || $SUDO ssh-keygen -t ed25519 -f /root/.ssh/relay_key -N '' -q
echo "  → 把下面这行公钥登记到公网中继（在 relay 上跑 roles/relay/authorize.sh）："
echo "  ---------------------------------------------------------------"
$SUDO cat /root/.ssh/relay_key.pub
echo "  ---------------------------------------------------------------"

echo "[4/5] 启动 Coder + Postgres + Caddy"
$SUDO docker compose up -d --build

echo "[5/5] 安装反向隧道 systemd 服务（用系统自带 ssh，免装 autossh）"
$SUDO mkdir -p /etc/devsys
$SUDO cp .env /etc/devsys/control-plane.env
$SUDO cp reverse-tunnel.service /etc/systemd/system/devsys-tunnel.service
$SUDO systemctl daemon-reload
$SUDO systemctl enable --now devsys-tunnel

echo "✓ 控制面已启动。若隧道未连上，通常是上面的公钥还没登记到 relay。"
echo "  访问：https://${DEV_DOMAIN}:${PUBLIC_PORT}"
