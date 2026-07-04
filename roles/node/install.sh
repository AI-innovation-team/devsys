#!/usr/bin/env bash
# 角色 node 的一键安装。幂等。在「每台内网开发机」上执行：
#   cp .env.example .env && vim .env      # 改 NODE_NAME，其余同控制面
#   bash install.sh
set -euo pipefail
cd "$(dirname "$0")"

[ -f .env ] || { echo "✗ 缺少 .env，先 cp .env.example .env 并填写"; exit 1; }
set -a; . ./.env; set +a
: "${DEV_DOMAIN:?}"; : "${PUBLIC_PORT:?}"; : "${PROVISIONER_PSK:?}"; : "${NODE_NAME:?}"

SUDO=""; [ "$(id -u)" -ne 0 ] && SUDO="sudo"
IMG="ghcr.io/coder/coder:latest"

echo "[1/3] 检查 Docker 与 nvidia runtime"
command -v docker >/dev/null || { echo "✗ 未装 Docker"; exit 1; }
if $SUDO docker info 2>/dev/null | grep -qi 'Runtimes:.*nvidia'; then
  echo "  ✓ 检测到 nvidia runtime"
else
  echo "  ⚠ 未检测到 nvidia runtime，GPU 直通可能不可用（需装 nvidia-container-toolkit）"
fi

echo "[2/3] 确保 Coder 镜像（多镜像源依次尝试，兼容国内网络）"
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

echo "[3/3] 启动置备器（tag: machine=${NODE_NAME}）"
$SUDO docker compose up -d

echo "✓ node '${NODE_NAME}' 已上线。到 Coder 后台 Deployment → Provisioners 应能看到它。"
