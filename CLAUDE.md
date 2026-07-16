# DevSys / AIT.dev

> 本文件是项目的长期上下文。改动架构或方向时同步更新它。

## 一句话

一个 SSH 客户端起步的**自包含桌面 app**,终极愿景是**一台透明、去中心化、人与 AI 共治的科学引擎** —— 把一群人的算力、数据、智能汇成一张自持、可复现、会复利的网,造福学术界(把不透明的中心化人治,变成透明的去中心化共治)。

北极星文档(愿景全貌):https://claude.ai/code/artifact/d275b038-38cd-4608-ad90-13ce801b28d7

## 核心心智(反复用到)

- **没有"本地/远程"之分,只有"节点"**:一台机器、一份数据、一个人、一个 agent,都是织物上一个可达的节点。算力**按位置**寻址、数据**按内容**寻址、agent **按能力**寻址,但共用同一张织物 / 登记 / 策略。
- **可达性统一交给 tailnet**:穿内网,数据和算力是同一个答案(WireGuard mesh,零公网暴露)。不要自己去凿公网穿透。
- **配置即代码 · 自持**:团队 = 一份共享 config(如 `team.yaml`)+ 每人登记自己的设备。协调器轻到几乎没有。
- **可复现仓库 = 信任原子**:研究产出 = 论文 + 完全可复现的代码/配置仓库。内容寻址;重跑=保真验证①;论文=价值评审②(裁判 agent)。
- **责任为门**:AI 能做能评但担不了责;发表 = 人署名担责。担责人核查的每处修改全程可追踪,并用于校准 agent(担责即校准)。
- **三层信任**:私有团队 → 联邦(学术去中心化的甜点)→ 公开公地。别一步跳到公地。

## 当前架构(两个可交付物,共用一套前端)

- **自包含桌面 app(主线,Tauri v2)**:本地存拓扑/凭据,Rust 原生 SSH 直连内网,无需服务端。
- **web 门户(遗留,仍可用,勿删)**:FastAPI + oauth2-proxy + Caddy + relay,团队/浏览器访问 —— 是"远程/团队"模式的雏形。
- **一套 React 前端喂两个目标**,靠抽象层切换:
  - `frontend/src/transport/`:终端 IO —— web 版包 WebSocket(到门户),tauri 版包 invoke(到 Rust)。`Terminal.tsx` 主体零改动。
  - `frontend/src/data/`:拓扑/凭据数据源 —— web 版走门户 REST,tauri 版走 Rust 命令。
  - 用 `isTauri`(`"__TAURI_INTERNALS__" in window`)选实现。

## 技术选型

- **Tauri v2** + `withGlobalTauri`(前端用 `window.__TAURI__.core.invoke` / `.event.listen`,不引 @tauri-apps npm 包)。
- **russh 0.45** 原生 SSH(直连 + ProxyJump direct-tcpip 通道 + 密码/私钥 + PTY)。
- **iota_stronghold 2.x** 加密保险库存凭据(主密码 = 登录密码 → Argon2 → 加密快照)。**已弃用 keychain**(`creds.rs` 是废弃占位)。
- **tailscaled sidecar**(规划中,阶段3):内嵌 tailnet 直连。
- 前端:React 18 + Vite + TS,手写 view 状态机(非 react-router),CIBOL 设计 token(`frontend/src/styles/tokens.css`),节点网 logo(`assets/logo/`)。

## 目录

```
frontend/        React 前端(transport/ data/ screens/ components/ styles/)
src-tauri/       Tauri v2 + Rust:lib.rs(命令)ssh.rs store.rs vault.rs sshconfig.rs
backend/         FastAPI 门户(devsys_portal),web/团队路径
deploy/ scripts/ deploy.sh   门户部署编排(Caddy/oauth2/relay/tailnet)
config.yaml      门户拓扑(gitignore;.env 放机密)
assets/logo/     节点网 logo(light/dark)
```

## 构建 / 运行

- 桌面 app:仓库根 `npm run tauri dev`(前端固定端口 **1420**;首次编 russh/stronghold 较慢)。
- 前端单独:`cd frontend && npm run build`(vite/esbuild,不做类型检查)。类型检查 `npx tsc --noEmit`(注:`src/upload.ts` 有既存的 Node26/TS5.5 `Uint8Array` lib 报错,与业务无关)。
- 门户部署:`./deploy.sh`(render → 构建前端 → 推 relay → 推 gateway)。默认 SSH 目标 `turing`,大流量断连时用 tailnet `100.125.82.91`。

## 关键坑(务必守住)

- **依赖必须优化编译**:`src-tauri/Cargo.toml` 有 `[profile.dev.package."*"] opt-level = 3`。否则 argon2/stronghold 纯 Rust 加密在 debug 下奇慢(解锁一次 ~35s,表现为"卡死")。删了就会复现。
- **重活命令必须 `async`**:Tauri 同步命令在 UI 主线程跑会冻结。`vault_unlock`/`save_credential` 已是 async。
- **凭据只写不读**:前端永不拿明文;`has_secret` 用 servers.json 标记位(启动不读保险库,避免弹窗/慢)。保险库只在登录解锁 + 连接时读。
- **门户不可重生成 oauth2-proxy.cfg**:门户端拿不到 .env,重生成会丢 client_secret/cookie_secret → 认证瘫痪。只原地改单行。
- **运行时真源在门户可写目录**(`~/gateway/data`、`~/gateway/oauth2`),绝不放 /etc。
- WKWebView 不支持元素级 requestFullscreen → 用 Tauri 窗口 API。

## 路线图(呼应北极星)

- **v0(接得上)✅ 已落地**:登录页去掉"本地/远程"切换,回归纯本地解锁(密码=保险库钥匙);服务器列表**按来源合并**(`Server.source = mine | team:<名>`,分组渲染、团队机只读);app 内「连接团队」入口(Team 屏占位 + 侧栏入口)。
- **v1(配得起)**:团队配置(建团队/邀成员/按人授权)→ 把门户 admin 泛化成 `team.yaml`。数据面复用原生 SSH。**贡献侧**:机器加 `shared_to: [team:<名>]`,把拓扑 + 授权声明提交进 team.yaml(凭据不出本机),队友那侧落成只读节点 —— 与 v0 的消费侧合成闭环。见下「共享算力模型」。
- **v2(长得开)**:自助贡献机器 + tailnet 织物 + 规格/负载登记 → 动态算力网;调度接 **SkyPilot**(见下);数据成网(内容寻址 git-annex/DVC over tailnet);持久 agent 托管 + 沙箱(见下);裁判 agent 评审 = 去中心化同行评阅;出版/评审层接 **DeSci**(见下「DeSci 用法」)。

## 主页形态(方向,v1.5→v2,反复用到)

主页 = **驾驶舱**,两半 + 一条缝合它们的核心交互:

- **织物图 = 地图/导航**(左):现有 `TeamGraph`(力导向织物、节点=人/机/将来的 agent、org=无名包络、无特权中心=P2P、悬停出身份与授权、主色环标「我」)。回答「在哪、够得着谁」。这张图**很重要、大有可为**,是主页一等公民,不是装饰。
- **工作区 = 工作面(网页版 tmux)**(右):多 pane、可持久的活会话。回答「正在干什么」。落在现有 `tmux.py`(会话封装)+ `ssh.rs` PTY + `Terminal.tsx`(xterm)上,不从零造。
- **缝合 = 点节点→开 pane**:图是**启动器/导航器**不是只读画;在地图上点一个节点 → 工作区开出它的终端 pane。地图选位置,工作面出终端。
- **北极星落地**:v2 持久 agent 上后,**agent 也是节点** —— 地图上是一个点、工作区里是一个 pane;于是人和 AI **共用同一张地图、同一个工作面**共事。「人与 AI 共治」第一次有了具体 UI 形态(一张织物上一起干活,不是两个工具)。
- **时序**:v1 收口后才动。先留住这个方向。

## 共享算力模型(v1→v2 定稿,反复用到)

一整条链的设计结论,别再重推:

- **共享的是「拓扑」,不是「凭据」**。贡献一台机 = 把"怎么到达它"给团队;密码/私钥永不出本机(铁律)。
- **身份到人,不用共享账号**。因为北极星信任原子 = 每处操作可追踪、用于校准 agent(担责即校准)。统一团队账号会断掉追踪链 → 禁用。
  - **终态**:Tailscale SSH + ACL —— 被共享机 `tailscale up --ssh` + 打 tag;policy 的 `ssh` 段声明 `src=group:团队, dst=tag:池, users=[各人账号], action=check/accept`;认证走 tailnet 身份(挂团队 SSO),零 authorized_keys、短期证书、离队即失效、自带 session recording(=现成的"可追踪")。等价开源自建 = 团队 SSH CA(step-ca)签短期证书,principal 内嵌身份。
  - **v1 过渡**:`shared_to` 把成员公钥从 team.yaml 同步进各自**独立账号**的 authorized_keys(身份到人、无共享号);sidecar 上了再无缝切 Tailscale SSH。
- **权限 = 档位**(机器主人选、也为此担责 = 责任为门;team.yaml 只记"这台对 team:X 开档几"):
  - **档0 纯跳板**(只借道、无 shell)· **档1 受限计算账号(默认)**(能登入跑计算、用配额内 GPU/CPU;无 sudo、进不了别人目录、占不满、可撤、可追踪)· **档2 完全信任**(有 sudo,仅核心成员)。
- **隔离 = 用容器兑现档位**(比裸机受限账号更干净:默认关死、天然限额、用完即焚、镜像即可复现环境)。**"档"=开多少权 与 "容器"=怎么关 正交**。
  - 信任队友 → 普通容器够;**不可信 / agent 生成的代码 → 必须强沙箱**(普通 docker 共享内核**不是**安全边界)。
  - **沙箱选型**:CPU coding agent → **Docker Sandboxes**(2026-03,microVM 硬隔离、跨平台、复杂度被 Docker 封装、原生认 Claude Code);**GPU + agent(我们核心)→ Docker Sandboxes 撑不起**(microVM/Firecracker 天生不支持 GPU 直通)→ 盯 **NVIDIA OpenShell**(GTC 2026 开源、GPU-native、原生 claude、声明式策略,最对味)/ Kata(microVM+GPU 直通)/ gVisor+nvproxy(拦 CUDA,安全性反而更好)/ Modal(托管)。**GPU 直通削弱隔离是必然折中**(驱动=共享攻击面);真直通要裸机+IOMMU。强沙箱**不自己造,接现成**。
- **SkyPilot = 可插的作业调度组件,不是底座**。它把机器抽象成"能 SSH 的盒子",恰好吃我们 tailnet 上登记好的算力池:登记成它的 **SSH Node Pool**(`~/.sky/ssh_node_pools.yaml` → `sky ssh up` → `sky launch --infra ssh/<pool>`)或对接 on-prem K8s,`sky launch` 就调度(比价/spot/队列/serve)。它的团队层(API server + Workspaces + User/Admin RBAC,v0.10 起)是**中心化**的、单团队够用 —— 但**去中心化联邦 + 每人贡献节点 + 责任追踪仍是我们的**,别让它的中心 API server 成为身份/治理真源(可共用同一 IdP 对齐身份)。其 Sandboxes 亦可选用。**松耦合接,别焊死**(同时留住直连 SSH / SkyPilot / 未来别的调度器)。

## 可达性与模型完备性(已知缺口清单,别误以为 model 已封口)

**可达性(reachability)—— 主体已覆盖**。核心原则:**内网 ≠ 需要跳板**;能出 443 的机就自己上 mesh 直连,跳板/桥只给"出不了公网 / 装不了 tailscale"的节点。
- tailnet 直连 P2P ✅ · 同 LAN `direct` ✅ · 公网 IP `direct` ✅ · NAT/CGNAT/对称 NAT 打洞失败→DERP 兜底 ✅(tailscale 扛,非我们的事)
- **多跳跳板链 ✅ 已实现**:jump 是"指向另一台机的名字",多跳=链式引用(target.jump→login、login.jump→edge)。`store::jump_chain` 走链(防环/防悬空),`ssh.rs::build_conn` 嵌套多层 direct-tcpip。校园"edge→登录节点→算力节点"两跳以上兑现。数据模型/前端未改。
- **校园无出网机**:靠校园里一台能出网的邻居当门——**subnet router**(`--advertise-routes` 广播网段,成员用内网 IP 直达,最省)或 **jump host**(SSH 层);想全内网自持则自建 **Headscale + 内网 DERP**。

**真·已知缺口(还没落,别假装完备)**:
1. **subnet router 未 first-class**:靠它打通的机现在算 `direct` 还是 `tailnet` 含糊,值得单独成一档或明确归属。
2. **门(jump/subnet router)本身未作一等节点**:它是共享拓扑里的关键基建,图里没显式建模/可视化。
3. **agent 节点**(按能力寻址)—— 北极星有,v2 建。
4. **数据节点**(内容寻址 git-annex/DVC over tailnet)—— 北极星"一份数据也是节点",现在模型只有人/机两种切面,**无数据切面** —— v2。
5. **认证演进**:现在 `authorized_keys` 下发(过渡);终态 Tailscale SSH / step-ca 短期证书。tier 抽象已就位,**切换路径未建**(刻意 v1→v2)。

## DeSci 用法(v2 出版/评审层,定稿别再重推)

调研过 DeSci(去中心化学术生态:区块链 + DAO + 代币激励 + IPFS/Arweave)。结论:**它补的是我们北极星的另一半——出版/评审/资助层**(我们现在造的是织物/算力/身份层),互补不重叠。**只用它的数据层,存储/身份/信任全用我们自己的。** 一句话心智:**DeSci Codex 当「公地出口格式」,tailnet+git 当「私有/联邦存储」,信任永远走具名担责 + 裁判 agent。**

- **① 抄格式不抄栈**:采用 **RO-Crate 研究对象**(`ro-crate-metadata.json`,JSON-LD,描述手稿/代码/数据/环境 + 校验和)当「可复现仓库=信任原子」的落地 schema。纯元数据文件,加到任何 git 仓库即可,**不需 IPFS/链**。别自己发明 schema(`github.com/desci-labs/nodes` 开源,有 nodes-lib/CLI)。
- **② 内容寻址落织物**:它用 IPFS CID,我们换 **git SHA + git-annex/DVC 哈希 over tailnet**。一样内容寻址/可验证/可版本,但**数据不出织物**(守「数据不出本机」「可达性交 tailnet」)。
- **③ 重跑=保真验证 复用算力共享**:研究对象带环境引用(Docker 镜像)→ 拉对象 → 在共享节点拉起环境跑一遍 → diff 输出。**就是 SkyPilot on tailnet SSH pool**,不新造;DeSci 给「验什么」,我们给「在哪验」。
- **④ 信任层换我们的(坚决不用 DeSci 这层)**:DeSci 把作者/评审可信度记成**链上声誉分**(AI/agent 也能挣声誉背书,因为它信数字);我们换成研究对象里嵌**签名担责记录**——具名担责人(tailnet SSO 身份)+ 裁判 agent 评审 + 被追踪用于校准 agent 的修改 diff。**分道点**:DeSci 用「声誉复利」替代守门人,我们用「具名担责 + 可追踪校准」替代守门人;它不校准 agent 所以不需追踪链,我们要,所以担责/追踪是铁律(呼应「责任为门·担责即校准」;匿名/纯声誉身份会断追踪链 → 禁用)。
- **在三层信任里的位置**:ring1 私有 = 对象存 tailnet+git-annex;ring2 联邦 = 同一 RO-Crate 对象在盟友 org 间同步(格式互通);ring3 公地 = 用 nodes-lib 注册 dPID/DOI、推 DeSci Codex/IPFS。**因为第一天就用 RO-Crate,通往公地零翻译**——DeSci 是我们的**公地网关**,不是内部存储(呼应「别一步跳到公地」)。
- **LabDAO**(P2P 算力/服务交易 Lab-Exchange):思路同源但绑生物信息+链上结算,**参考协议、不接底座**(我们走 tailnet+SkyPilot)。
- **现在不写代码**:这是 v2,当前重心仍是 v1 共享算力闭环。v2 第一刀 = 定义 RO-Crate 研究对象 schema + `examples/` 范例对象。

已完成(截至本轮):Tauri 阶段0–2(脚手架+传输抽象、本地拓扑/凭据、russh 原生 SSH 真连成功);Stronghold 保险库;登录门(密码=钥匙);SSH config 导入(读 ~/.ssh/config + 文件选择 + 带 IdentityFile 私钥导入);本地退出登录;**v0 三件事(纯本地解锁 / 按来源分组 / 连接团队入口)** + 清理废弃 creds.rs。
