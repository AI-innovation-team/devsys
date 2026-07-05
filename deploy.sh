#!/usr/bin/env bash
# devsys 一键部署。在本地机器运行（需能 SSH 到 relay 与 gateway）。
#
#   cp .env.example .env && vim .env      # 填 OAuth + DNS 凭据
#   vim config.yaml                       # 填拓扑
#   ./deploy.sh                           # 全量：渲染 + 构建前端 + 推 relay + 推 gateway
#
# 分步： ./deploy.sh render | build | relay | gateway | check
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$PWD"
STEP="${1:-all}"

command -v python3 >/dev/null || { echo "✗ 需要 python3"; exit 1; }
[ -f config.yaml ] || { echo "✗ 缺少 config.yaml"; exit 1; }
[ -f .env ] || { echo "✗ 缺少 .env（cp .env.example .env 并填写）"; exit 1; }

# ── 读拓扑（stdlib YAML 加载器）──
eval "$(python3 - <<'PY'
import sys, shlex; sys.path.insert(0, "scripts")
from devsysconf import load
c = load("config.yaml")
def e(k, v): print(f"{k}={shlex.quote(str(v))}")
e("DOMAIN", c["domain"]); e("PUBLIC_PORT", c["public_port"])
r = c["relay"]; e("RELAY_SSH", r["ssh"]); e("RELAY_USER", r["user"])
g = c["gateway"]; e("GW_SSH", g["ssh"]); e("GW_HOME", g["home"])
PY
)"

log() { printf '\n\033[1;36m▶ %s\033[0m\n' "$*"; }

do_render() {
  log "渲染网关配置 → build/gateway"
  python3 scripts/render.py build
}

do_build() {
  log "构建前端 → frontend/dist"
  ( cd frontend && { [ -d node_modules ] || npm ci --registry=https://registry.npmmirror.com --no-audit --no-fund; } && npm run build )
}

do_relay() {
  log "relay → $RELAY_SSH（隧道账号 + GatewayPorts）"
  scp -q -r deploy/relay "$RELAY_SSH:~/"
  ssh "$RELAY_SSH" "sudo RELAY_USER=$(printf %q "$RELAY_USER") bash ~/relay/setup-relay.sh"
}

do_gateway() {
  log "gateway → $GW_SSH（后端 + 前端 + 配置 + 服务）"
  ssh "$GW_SSH" 'mkdir -p ~/gateway ~/gateway/systemd ~/gateway/web ~/gateway/backend'
  # 后端（只发源码 + uv 清单，不发本地 .venv）
  scp -q -r backend/devsys_portal backend/pyproject.toml backend/uv.lock "$GW_SSH:~/gateway/backend/"
  scp -q -r frontend/dist/. "$GW_SSH:~/gateway/web/"
  ssh "$GW_SSH" 'test -d ~/gateway/docs' || scp -q -r docs "$GW_SSH:~/gateway/docs"
  # 渲染出的配置
  scp -q build/gateway/Caddyfile build/gateway/control-plane.env build/gateway/oauth2-proxy.cfg \
         build/gateway/servers.json build/gateway/caddy-module.txt "$GW_SSH:~/gateway/"
  scp -q build/gateway/systemd/*.service "$GW_SSH:~/gateway/systemd/"
  # 部署静态件
  scp -q -r deploy/gateway/caddy "$GW_SSH:~/gateway/"
  scp -q deploy/gateway/docker-compose.yml deploy/gateway/install.sh "$GW_SSH:~/gateway/"
  scp -q deploy/login/sign_in.html "$GW_SSH:~/gateway/sign_in.html"
  # 安装
  ssh "$GW_SSH" 'cd ~/gateway && bash install.sh'
  # 登记隧道公钥到 relay
  log "登记隧道公钥 → relay"
  ssh "$GW_SSH" 'sudo cat /root/.ssh/relay_key.pub' \
    | ssh "$RELAY_SSH" "sudo RELAY_USER=$(printf %q "$RELAY_USER") bash ~/relay/authorize.sh"
}

case "$STEP" in
  check)   log "配置检查（不连远程）"
           echo "对外    : https://$DOMAIN:$PUBLIC_PORT"
           echo "relay   : $RELAY_USER@… (ssh $RELAY_SSH)"
           echo "gateway : ssh $GW_SSH  (home $GW_HOME)"
           python3 scripts/render.py build >/dev/null && echo "渲染    : OK（build/gateway）" ;;
  render)  do_render ;;
  build)   do_build ;;
  relay)   do_relay ;;
  gateway) do_render; do_gateway ;;
  all)     do_render; do_build; do_relay; do_gateway
           log "完成 ✅  浏览器打开 https://$DOMAIN:$PUBLIC_PORT" ;;
  *) echo "用法: ./deploy.sh [all|render|build|relay|gateway|check]"; exit 1 ;;
esac
