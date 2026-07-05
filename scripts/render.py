#!/usr/bin/env python3
"""从 config.yaml + .env + .secrets 渲染出网关的具体配置文件到 build/gateway/。

deploy.sh 调用它，再把 build/gateway + backend + frontend/dist 推到网关机。
纯标准库。用法： python3 scripts/render.py [build_dir]
"""
import json
import os
import secrets as _secrets
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from devsysconf import load  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent

# ── 各 DNS 服务商在 Caddy tls{} 里的 dns 块（引用 .env 的凭据，Caddy 从容器环境读 {env.X}）──
DNS_BLOCKS = {
    "alidns": "dns alidns {\n  access_key_id {env.ALIDNS_ACCESS_KEY_ID}\n  access_key_secret {env.ALIDNS_ACCESS_KEY_SECRET}\n}",
    "cloudflare": "dns cloudflare {env.CF_API_TOKEN}",
    "dnspod": "dns dnspod {env.DNSPOD_TOKEN}",
}

CADDYFILE = """{
\tadmin off
\temail {$ACME_EMAIL}
\tauto_https disable_redirects
}
https://{$DEV_DOMAIN}:{$PUBLIC_PORT} {
\ttls {
__DNS_BLOCK__
\t}

\t# oauth2-proxy 自己的端点（登录页 / 回调）
\thandle /oauth2/* {
\t\treverse_proxy 127.0.0.1:4180
\t}

\t# 其余全部先过 GitHub 认证，未登录跳登录页
\thandle {
\t\tforward_auth 127.0.0.1:4180 {
\t\t\turi /oauth2/auth
\t\t\tcopy_headers X-Auth-Request-User X-Auth-Request-Email
\t\t\t@error status 401
\t\t\thandle_response @error {
\t\t\t\tredir * /oauth2/sign_in?rd={uri}
\t\t\t}
\t\t}
\t\treverse_proxy 127.0.0.1:8090
\t}
}
"""


def read_env(path: Path) -> dict:
    d = {}
    if path.exists():
        for line in path.read_text().splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                d[k.strip()] = v.strip()
    return d


def main(build_dir: str):
    cfg = load(str(ROOT / "config.yaml"))
    env = read_env(ROOT / ".env")
    secrets_env = read_env(ROOT / ".secrets")

    cookie = secrets_env.get("COOKIE_SECRET")
    if not cookie:
        cookie = _secrets.token_urlsafe(32)
        with open(ROOT / ".secrets", "a") as f:
            f.write(f"COOKIE_SECRET={cookie}\n")

    domain = cfg["domain"]
    port = str(cfg["public_port"])
    gw = cfg["gateway"]
    home = gw["home"].rstrip("/")
    relay = cfg["relay"]
    provider = cfg["dns"]["provider"]
    if provider not in DNS_BLOCKS:
        sys.exit(f"✗ 未内置 dns.provider={provider}，请在 scripts/render.py 的 DNS_BLOCKS 里补一个块")

    out = Path(build_dir) / "gateway"
    (out / "systemd").mkdir(parents=True, exist_ok=True)

    # ── Caddyfile（只注入 dns 块；域名/端口/邮箱走容器环境）──
    dns_block = "\n".join("\t\t" + ln for ln in DNS_BLOCKS[provider].splitlines())
    (out / "Caddyfile").write_text(CADDYFILE.replace("__DNS_BLOCK__", dns_block))
    (out / "caddy-module.txt").write_text(cfg["dns"]["module"] + "\n")  # xcaddy 构建用

    # ── control-plane.env（Caddy + 隧道读取；透传 .env 里的 DNS 凭据）──
    envlines = [
        f"DEV_DOMAIN={domain}",
        f"PUBLIC_PORT={port}",
        f"ACME_EMAIL={cfg['acme_email']}",
        f"CONTAINER_PROXY={cfg.get('container_proxy', '')}",
        f"RELAY_HOST={relay['host']}",
        f"RELAY_SSH_USER={relay['user']}",
        f"RELAY_SSH_PORT={relay.get('port', 22)}",
    ]
    for k, v in env.items():
        if not k.startswith("OAUTH_"):   # DNS 凭据等透传给 Caddy 容器
            envlines.append(f"{k}={v}")
    (out / "control-plane.env").write_text("\n".join(envlines) + "\n")

    # ── oauth2-proxy.cfg ──
    users = ", ".join(f'"{u}"' for u in cfg["oauth"].get("github_users", []))
    (out / "oauth2-proxy.cfg").write_text(f"""provider = "github"
client_id = "{env.get('OAUTH_CLIENT_ID', '')}"
client_secret = "{env.get('OAUTH_CLIENT_SECRET', '')}"
cookie_secret = "{cookie}"
http_address = "127.0.0.1:4180"
reverse_proxy = true
redirect_url = "https://{domain}:{port}/oauth2/callback"
whitelist_domains = ["{domain}:{port}"]
cookie_domains = ["{domain}"]
cookie_secure = true
email_domains = ["*"]
github_users = [ {users} ]
set_xauthrequest = true
custom_templates_dir = "{home}/gateway/oauth2/templates"
""")

    # ── servers.json ──
    servers = [{"name": s["name"], "host": s["host"], "port": int(s.get("port", 22)),
                **({"jump": s["jump"]} if s.get("jump") else {})} for s in cfg["servers"]]
    (out / "servers.json").write_text(json.dumps(servers, ensure_ascii=False, indent=2))

    # ── systemd units ──
    (out / "systemd" / "devsys-portal.service").write_text(f"""[Unit]
Description=devsys portal (FastAPI)
After=network-online.target
Wants=network-online.target

[Service]
User={gw['user']}
Environment=HOME={home}
Environment=DEVSYS_DATA={home}/gateway/data
Environment=DEVSYS_SERVERS=/etc/devsys/servers.json
Environment=DEVSYS_WEB={home}/gateway/web
Environment=DEVSYS_DOCS={home}/gateway/docs
WorkingDirectory={home}/gateway/backend
ExecStart={home}/gateway/backend/.venv/bin/python -m uvicorn devsys_portal.main:app --host 127.0.0.1 --port 8090
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
""")

    (out / "systemd" / "devsys-oauth2.service").write_text(f"""[Unit]
Description=devsys oauth2-proxy (GitHub auth)
After=network-online.target
Wants=network-online.target

[Service]
User={gw['user']}
ExecStart={home}/gateway/oauth2/oauth2-proxy --config={home}/gateway/oauth2/oauth2-proxy.cfg
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
""")

    (out / "systemd" / "devsys-tunnel.service").write_text("""[Unit]
Description=devsys reverse tunnel to public relay
After=network-online.target docker.service
Wants=network-online.target

[Service]
User=root
EnvironmentFile=/etc/devsys/control-plane.env
ExecStart=/usr/bin/ssh -NT \\
  -o ServerAliveInterval=30 -o ServerAliveCountMax=3 \\
  -o ExitOnForwardFailure=yes -o StrictHostKeyChecking=accept-new \\
  -o BatchMode=yes \\
  -i /root/.ssh/relay_key \\
  -p ${RELAY_SSH_PORT} \\
  -R 0.0.0.0:${PUBLIC_PORT}:localhost:${PUBLIC_PORT} \\
  ${RELAY_SSH_USER}@${RELAY_HOST}
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
""")

    print(f"✓ 渲染完成 → {out}")
    for p in sorted(out.rglob("*")):
        if p.is_file():
            print("  " + str(p.relative_to(Path(build_dir))))


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else str(ROOT / "build"))
