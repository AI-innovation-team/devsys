# devsys — 团队内网远程开发门户

一台**公网服务器**做入口（纯转发），一台稳定**内网机**做网关跑门户，团队成员用自己的
**GitHub 账号**（申请制）登录后，从浏览器直达各内网服务器：

- **Web SSH** 终端
- **VS Code**（浏览器版 code-server，每人独立实例）
- **工作区**：基于 `tmux` 的持久会话，关掉网页再回来仍可接入（进程/历史都在）
- **文档**：维护若干 `.md` 文件即成文档站
- **设置**：每人自设在各机上的 userid + 密钥/密码（Fernet 加密存网关），连接以本人身份进行

界面是暖色人文调的 CIBOL 设计系统，亮/暗双主题。

---

## 架构

```
用户浏览器
   │  https://<domain>:<public_port>
   ▼
[relay]  公网服务器：sshd GatewayPorts，纯 TCP 转发（很轻）
   │  反向 SSH 隧道（gateway 主动外连）
   ▼
[gateway] 内网机：Caddy(TLS) → oauth2-proxy(GitHub 申请制) → 门户(FastAPI)
   │  asyncssh（用户凭据，可选跳板）
   ▼
[servers] 各内网机：SSH / code-server / tmux
```

- **relay**：只有一个隧道专用账号 + `GatewayPorts`。不跑任何重服务。
- **gateway**：`Caddy`（Docker，DNS-01 签证书）+ `oauth2-proxy`（systemd）+ `门户`（systemd, uvicorn）+ 反向隧道（systemd）。同时是访问内网的 bastion。
- **门户**：前后端分离。后端纯 JSON/WS API，前端是构建好的 SPA，由后端托管。

## 仓库结构

```
config.yaml            拓扑（非机密）        .env(.example)  机密（OAuth/DNS 凭据）
deploy.sh              一键部署编排
scripts/
  devsysconf.py        stdlib YAML 加载器
  render.py            config → 网关具体配置（Caddyfile/oauth2/servers.json/systemd）
backend/               FastAPI 后端（模块化）
  devsys_portal/       config crypto storage servers auth ssh tmux vscode docs + routes/
  requirements.txt
frontend/              Vite + React + TS（CIBOL 设计系统）
  src/  ds/tokens · screens · icons · api
deploy/
  gateway/  caddy/Dockerfile · docker-compose.yml · install.sh
  relay/    setup-relay.sh · authorize.sh
  login/    sign_in.html（oauth2-proxy 自定义登录页）
docs/                  门户文档（*.md，部署时作为初始内容）
```

## 部署（一键）

前置：本地有 `python3` + `node`（构建前端）；能 SSH 到 relay 与 gateway；gateway 上有 `docker`。

```bash
cp .env.example .env      # 填 GitHub OAuth 的 client id/secret + DNS 凭据
vim config.yaml           # 填 domain / relay / gateway / servers / dns
./deploy.sh               # 渲染 + 构建前端 + 推 relay + 推 gateway
```

分步：`./deploy.sh render|build|relay|gateway|check`。

改配置后重新 `./deploy.sh gateway` 即可滚动更新（用户凭据 `data/` 不受影响）。

### 换 DNS 服务商 / OAuth

- DNS：`config.yaml` 的 `dns.provider` 内置 `alidns/cloudflare/dnspod`；凭据填 `.env`。
  其它服务商在 `scripts/render.py` 的 `DNS_BLOCKS` 加一个 Caddy `dns` 块即可。
- 登录白名单：`config.yaml` 的 `oauth.github_users`。

### 目标机准备（VS Code 需要）

各内网机装 `code-server`（`tmux` 一般已自带）。门户会以用户身份按需启动。

## 本地开发

```bash
# 后端
cd backend && pip install -r requirements.txt
DEVSYS_SERVERS=/path/servers.json uvicorn devsys_portal.main:app --port 8090
# 前端（代理到 8090）
cd frontend && npm install && npm run dev
```

## 安全要点

- 门户不持有 root、不共享凭据；每人用自己账号连接。凭据 Fernet 加密落盘于网关。
- 登录走 GitHub OAuth，`github_users` 白名单=申请制。
- `.env` / `.secrets` / 运行期 `data/` 已在 `.gitignore`，绝不入库。
