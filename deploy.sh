#!/usr/bin/env bash
# devsys 一键编排器。在「你的本地机器」上运行（需能 SSH 到所有目标机）。
#
#   cp .env.example .env && vim .env      # 填 ALIDNS 密钥
#   vim config.yaml                       # 填拓扑
#   ./deploy.sh                           # 全量部署
#
# 也可分步： ./deploy.sh relay | control-plane | nodes | template
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$PWD"
STEP="${1:-all}"

# ── 依赖（只用 python3 标准库，无第三方包）──
command -v python3 >/dev/null || { echo "✗ 需要 python3"; exit 1; }
[ -f config.yaml ] || { echo "✗ 缺少 config.yaml"; exit 1; }
[ -f .env ] || { echo "✗ 缺少 .env（cp .env.example .env 并填 ALIDNS）"; exit 1; }

# ── 读配置 ──
eval "$(python3 scripts/parse_config.py config.yaml)"

# ── 密钥：DNS 凭据来自 .env（任意服务商）；PSK/DB 密码自动生成到 .secrets ──
set -a; . ./.env; [ -f .secrets ] && . ./.secrets; set +a
grep -qE '^[A-Za-z_][A-Za-z0-9_]*=.' .env || { echo "✗ .env 里没有任何 DNS 凭据（如 ALIDNS_* 或 CF_API_TOKEN）"; exit 1; }
if [ -z "${PROVISIONER_PSK:-}" ]; then PROVISIONER_PSK=$(openssl rand -hex 32); echo "PROVISIONER_PSK=$PROVISIONER_PSK" >> .secrets; fi
if [ -z "${POSTGRES_PASSWORD:-}" ]; then POSTGRES_PASSWORD=$(openssl rand -hex 16); echo "POSTGRES_PASSWORD=$POSTGRES_PASSWORD" >> .secrets; fi

log() { printf '\n\033[1;36m▶ %s\033[0m\n' "$*"; }

deploy_relay() {
  log "relay → $RELAY_SSH"
  scp -q -r roles/relay "$RELAY_SSH:~/"
  ssh "$RELAY_SSH" "sudo RELAY_USER=$(printf %q "$RELAY_USER") bash ~/relay/setup-relay.sh"
}

deploy_control_plane() {
  log "control-plane → $CP_SSH"
  # 探测控制面机器的 docker 组 gid，供内置置备器访问 docker.sock
  local dgid; dgid=$(ssh "$CP_SSH" 'getent group docker | cut -d: -f3' 2>/dev/null)
  local envf; envf=$(mktemp)
  cat > "$envf" <<EOF
DEV_DOMAIN=$DOMAIN
PUBLIC_PORT=$PUBLIC_PORT
ACME_EMAIL=$ACME_EMAIL
CONTAINER_PROXY=$CONTAINER_PROXY
DOCKER_GID=${dgid:-999}
POSTGRES_PASSWORD=$POSTGRES_PASSWORD
PROVISIONER_PSK=$PROVISIONER_PSK
CADDY_DNS_MODULE=$CADDY_DNS_MODULE
RELAY_HOST=$RELAY_HOST
RELAY_SSH_USER=$RELAY_USER
RELAY_SSH_PORT=$RELAY_PORT
EOF
  # 透传用户在 .env 里提供的 DNS 凭据（ALIDNS_* / CF_API_TOKEN / … 任意）
  grep -E '^[A-Za-z_][A-Za-z0-9_]*=' .env >> "$envf"
  scp -q -r roles/control-plane "$CP_SSH:~/"
  scp -q "$envf" "$CP_SSH:~/control-plane/.env"; rm -f "$envf"
  ssh "$CP_SSH" 'cd ~/control-plane && bash install.sh'
  log "登记控制面隧道公钥到 relay"
  ssh "$CP_SSH" 'sudo cat /root/.ssh/relay_key.pub' | ssh "$RELAY_SSH" "sudo RELAY_USER=$(printf %q "$RELAY_USER") bash ~/relay/authorize.sh"
}

deploy_nodes() {
  for entry in "${NODES[@]}"; do
    local name="${entry%%:*}" target="${entry##*:}"
    log "node '$name' → $target"
    local envf; envf=$(mktemp)
    cat > "$envf" <<EOF
DEV_DOMAIN=$DOMAIN
PUBLIC_PORT=$PUBLIC_PORT
PROVISIONER_PSK=$PROVISIONER_PSK
NODE_NAME=$name
CONTROL_PLANE_LAN=$CONTROL_PLANE_LAN
CONTAINER_PROXY=$CONTAINER_PROXY
EOF
    scp -q -r roles/node "$target:~/"
    scp -q "$envf" "$target:~/node/.env"; rm -f "$envf"
    ssh "$target" 'cd ~/node && bash install.sh'
  done
}

gen_machines_tfvars() {
  log "生成机器清单 machines.auto.tfvars"
  local list=""
  for n in "${NODE_NAMES[@]}"; do list+="\"$n\", "; done
  printf 'machines = [%s]\n' "${list%, }" > templates/gpu-container/machines.auto.tfvars
  cat templates/gpu-container/machines.auto.tfvars
}

case "$STEP" in
  check)          log "配置检查（不连远程）"
                  echo "对外        : https://$DOMAIN:$PUBLIC_PORT"
                  echo "relay       : $RELAY_USER@$RELAY_HOST:$RELAY_PORT  (部署 ssh: $RELAY_SSH)"
                  echo "control-plane: ssh $CP_SSH"
                  echo "nodes       :"
                  for e in "${NODES[@]}"; do echo "  - ${e%%:*}  (ssh ${e##*:})"; done
                  echo "DNS 服务商   : $CADDY_DNS_MODULE"
                  echo "密钥        : DNS 凭据已填(.env)；PSK/DB 密码已就绪(.secrets)" ;;
  relay)          deploy_relay ;;
  control-plane)  deploy_control_plane ;;
  nodes)          deploy_nodes ;;
  template)       gen_machines_tfvars ;;
  all)            deploy_relay; deploy_control_plane; deploy_nodes; gen_machines_tfvars
                  log "完成 ✅  接下来：浏览器开 https://$DOMAIN:$PUBLIC_PORT 建管理员，再 coder templates push（见 README）" ;;
  *) echo "用法: ./deploy.sh [all|relay|control-plane|nodes|template]"; exit 1 ;;
esac
