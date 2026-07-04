# devsys — 公网 + 内网的团队远程开发平台

一台**公网服务器**做入口，若干**内网服务器**提供算力；团队成员在浏览器登录后，选一台内网机器就能拿到那台机器上的 **VS Code + 终端**（每人独立容器、GPU 直通、可挂载主机数据）。

**设计原则：不绑定具体机器/域名。** 一切站点相关的值（公网地址、域名、机器清单）都在 `.env` 里，代码按「角色」组织，换机器/换域名只改配置。

## 三个角色

| 角色 | 跑在哪 | 职责 | 目录 |
|---|---|---|---|
| **relay** | 任意公网服务器 | 纯 TCP 转发（sshd 反向隧道），极轻，无 Docker | [roles/relay](roles/relay) |
| **control-plane** | 一台稳定的内网机 | Coder Server + Postgres + Caddy(TLS) + 反向隧道 | [roles/control-plane](roles/control-plane) |
| **node** | 每台内网开发机 | Coder 置备器 + 用户容器 | [roles/node](roles/node) |

```
用户浏览器
  │ https://<DEV_DOMAIN>:<PUBLIC_PORT>     (DNS 指向公网服务器)
  ▼
[relay] 公网服务器  —— 只转发 TCP，无状态、可随时替换
  │ 反向隧道（内网主动拨出）
  ▼
[control-plane] 一台内网机  —— Coder + Postgres + Caddy
  │ 置备（经中继）
  ▼
[node] 各内网机  —— 本机 Docker 起用户容器（GPU 直通 + /data 挂载）
```

内网机器**只需能出站**访问公网服务器，不用公网 IP、不用开入站端口。

## 配置项（一次填好，多处复用）

| 变量 | 含义 | 当前示例 |
|---|---|---|
| `DEV_DOMAIN` | 对外访问域名 | `coder.psyagent.top` |
| `PUBLIC_PORT` | 对外端口（备案后可用 443） | `8443` |
| `RELAY_HOST` | 公网服务器地址 | `39.97.7.248` |
| `PROVISIONER_PSK` | 控制面与所有 node 共享密钥 | `openssl rand -hex 32` |
| `NODE_NAME` | 每台内网机的唯一名字 | `lecun` / `fodor` / … |
| `dns.module` / `dns.directive` | 证书 DNS-01 服务商（可换） | 阿里云 `alidns` |
| DNS 凭据（`.env`） | 上面 directive 引用的密钥 | 阿里云 RAM AccessKey |

---

## 一键部署（推荐）

在**你的本地机器**上（需能 SSH 到所有目标机，用 `~/.ssh/config` 别名最方便）：

```bash
cp .env.example .env      # 填 ALIDNS_ACCESS_KEY_ID / SECRET
vim config.yaml           # 填拓扑：域名、relay、control_plane、nodes
./deploy.sh               # 自动 SSH 到各机器按角色部署
```

- `config.yaml`：拓扑（非机密，可提交）。加机器就在 `nodes` 加一行。
- `.env`：只填阿里云 DNS 密钥；PSK 和数据库密码由 `deploy.sh` 自动生成并存到 `.secrets`（复跑稳定）。
- 分步执行：`./deploy.sh relay | control-plane | nodes | template`。

`deploy.sh` 干的事 = 下面「手动部署」每一步的自动化：拷角色目录 → 生成该机 `.env` → 跑 `install.sh` → 登记隧道公钥 → 生成机器清单。

**部署后两步收尾**（需要人工，因涉及登录/建号）：
1. 浏览器开 `https://<domain>:<public_port>` → 建管理员 → Users 里给成员建号（= 申请制）。
2. `coder templates push gpu-container -d templates/gpu-container`。

DNS：把 `domain` 的 A 记录指到 relay 公网 IP（一次）。

---

## 手动部署（理解内部 / 逐机排查）

**通用模式：每个角色都是「把该角色目录拷到目标机器 → 填 `.env` → 跑脚本」。** 换任意公网/内网机器都是同一套动作，不改代码。

### 0. DNS（一次）
把 `DEV_DOMAIN` 的 A 记录指向公网服务器 IP（如 `coder.psyagent.top → 39.97.7.248`）。

### 1. relay（任意公网服务器）
```bash
scp -r roles/relay <公网机>:~/relay && ssh <公网机>
sudo bash relay/setup-relay.sh          # 建 relay 账号 + 开 GatewayPorts
```
安全组放行 `PUBLIC_PORT`。详见 [roles/relay/README.md](roles/relay/README.md)。

### 2. control-plane（任意一台稳定内网机）
前提：装好 Docker。
```bash
scp -r roles/control-plane <内网机>:~/control-plane && ssh <内网机>
cd control-plane
cp .env.example .env && vim .env         # 填域名/端口/PSK/ALIDNS/RELAY_*
bash install.sh                          # 拉镜像+起服务+生成隧道密钥+装隧道
```
`install.sh` 会打印一行**隧道公钥**。把它登记到公网中继：
```bash
echo '<上一步打印的公钥>' | ssh <公网机> 'sudo bash ~/relay/authorize.sh'
```
然后浏览器打开 `https://<DEV_DOMAIN>:<PUBLIC_PORT>` → 创建管理员 → 在 Users 里**手动给成员建号**（密码模式无自助注册 = 天然申请制）。

### 3. node（每台内网开发机）
前提：Docker + nvidia-container-toolkit，`nvidia` 已注册为 docker runtime。
```bash
scp -r roles/node <内网机>:~/node && ssh <内网机>
cd node
cp .env.example .env && vim .env         # 改 NODE_NAME；其余同控制面
bash install.sh
```
每台机器做一遍。在 Coder 后台 Deployment → Provisioners 应看到它们上线。

### 4. 推送模板
```bash
vim templates/gpu-container/machines.auto.tfvars   # 机器清单，须和各 NODE_NAME 对应
coder login https://<DEV_DOMAIN>:<PUBLIC_PORT>
coder templates push gpu-container -d templates/gpu-container
```

### 用户使用
登录 → Create Workspace → 选模板 + 选机器 → 得到浏览器 VS Code / 终端 / `coder ssh`。

---

## 泛化操作

- **加一台内网机**：在新机器部署 `roles/node`（填 `NODE_NAME`）→ 把名字加进 `machines.auto.tfvars` → 重新 `coder templates push`。不改任何代码。
- **换公网服务器**：新机器跑 `setup-relay.sh` → 改控制面 `.env` 的 `RELAY_HOST` → 重启 `devsys-tunnel` → DNS 指向新 IP。relay 无状态，无需迁移数据。
- **换控制面机器**：迁移 Postgres 卷 + `.env` 到新机器起 compose 即可。

## 备注 / 待办
- 换 DNS 服务商（Cloudflare / DNSPod / …）：改 `config.yaml` 的 `dns.module` + `dns.directive`，再在 `.env` 填对应凭据。代码不动。
- 备案通过后改用 443：改 `.env` 的 `PUBLIC_PORT`、放行安全组、DNS 不变。
- 开发者文档站（Backstage/MkDocs）：跑通后再叠一层，可复用同一条中继。
