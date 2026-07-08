#!/usr/bin/env bash
# 在网关机上运行（由 deploy.sh scp 整个 ~/gateway 后调用）。
# 装依赖、放 systemd、起门户/oauth2/隧道/Caddy。幂等，可反复跑。
set -euo pipefail
cd "$(dirname "$0")"
GW="$PWD"

echo "▶ /etc/devsys（环境）+ 运行时 servers 种子"
sudo mkdir -p /etc/devsys
sudo cp control-plane.env /etc/devsys/control-plane.env
# servers 真源迁到门户可写目录（~/gateway/data），首次种子、之后不覆盖——
# 保留管理员在线增删的改动（同 htpasswd 的运行时可写模式）。
mkdir -p data
[ -f data/servers.json ] || cp servers.json data/servers.json

echo "▶ oauth2-proxy 配置 + 登录模板"
mkdir -p oauth2/templates
cp oauth2-proxy.cfg oauth2/oauth2-proxy.cfg
[ -f sign_in.html ] && cp sign_in.html oauth2/templates/sign_in.html || true
[ -f error.html ] && cp error.html oauth2/templates/error.html || true
[ -f oauth2/htpasswd ] || touch oauth2/htpasswd    # 邮箱登录用户表（用 add-email-user.sh 维护）
# oauth2-proxy 拒绝空 htpasswd：没有有效用户时先摘除，避免崩溃（加了用户会自动重新启用）
if ! grep -qE "^[^#]+:" oauth2/htpasswd 2>/dev/null; then
  sed -i "/^htpasswd_file/d; /^display_htpasswd_form/d" oauth2/oauth2-proxy.cfg
  echo "  （htpasswd 暂无用户，未启用邮箱登录；bash add-email-user.sh <email> 添加后自动启用）"
fi

if [ ! -x oauth2/oauth2-proxy ]; then
  echo "▶ 下载 oauth2-proxy"
  V=7.15.3; A=$(uname -m); case "$A" in x86_64) A=amd64;; aarch64|arm64) A=arm64;; esac
  for U in \
    "https://ghproxy.net/https://github.com/oauth2-proxy/oauth2-proxy/releases/download/v${V}/oauth2-proxy-v${V}.linux-${A}.tar.gz" \
    "https://github.com/oauth2-proxy/oauth2-proxy/releases/download/v${V}/oauth2-proxy-v${V}.linux-${A}.tar.gz"; do
    if curl -fsSL "$U" -o /tmp/o2p.tgz; then
      tar -xzf /tmp/o2p.tgz -C /tmp
      find /tmp -maxdepth 2 -name oauth2-proxy -type f -exec cp {} oauth2/oauth2-proxy \;
      chmod +x oauth2/oauth2-proxy && break
    fi
  done
  [ -x oauth2/oauth2-proxy ] || { echo "✗ oauth2-proxy 下载失败，请手动放到 oauth2/oauth2-proxy"; exit 1; }
fi

echo "▶ Python 环境（uv）"
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
if ! command -v uv >/dev/null 2>&1; then
  python3 -m pip install --user -q uv -i https://pypi.tuna.tsinghua.edu.cn/simple \
    || curl -LsSf https://astral.sh/uv/install.sh | sh
  export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"
fi
( cd backend
  export UV_PYTHON_DOWNLOADS=never                                  # 用系统 python，不下载解释器（网关脆弱/国内网络）
  export UV_DEFAULT_INDEX="https://pypi.tuna.tsinghua.edu.cn/simple"
  uv sync --frozen 2>/dev/null || uv sync )                         # 按 uv.lock 建 .venv 装依赖

echo "▶ 反向隧道密钥"
sudo test -f /root/.ssh/relay_key || sudo ssh-keygen -t ed25519 -N '' -f /root/.ssh/relay_key -q
echo "  隧道公钥（需登记到 relay，deploy.sh 会自动做）："
sudo cat /root/.ssh/relay_key.pub | sed 's/^/    /'

echo "▶ 门户自助改密：允许运行用户免密重启 oauth2"
SC=$(command -v systemctl)
echo "$(whoami) ALL=(root) NOPASSWD: $SC restart devsys-oauth2" | sudo tee /etc/sudoers.d/devsys-portal >/dev/null
sudo chmod 440 /etc/sudoers.d/devsys-portal

echo "▶ systemd 服务"
sudo cp systemd/*.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now devsys-oauth2 devsys-portal devsys-tunnel

echo "▶ Caddy（xcaddy 编译 DNS 插件 + 起容器）"
export CADDY_DNS_MODULE="$(cat caddy-module.txt 2>/dev/null || echo github.com/caddy-dns/alidns)"
# docker 需 root：以 root 跑 compose，既能连 daemon socket，又能读 600 的 control-plane.env（env_file）
sudo bash -c "CADDY_DNS_MODULE='$CADDY_DNS_MODULE' docker compose up -d --build"

echo "✓ 网关安装完成： portal=$(systemctl is-active devsys-portal) oauth2=$(systemctl is-active devsys-oauth2) tunnel=$(systemctl is-active devsys-tunnel)"
