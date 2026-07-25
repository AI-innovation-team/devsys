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
- **登录 = GitHub(OAuth Device Flow),本地不验密码**(`ghauth.rs` + `GithubGate.tsx`):app 显示短码 → 浏览器 `github.com/login/device` 授权 → 回来即登录,自动列所属 org 选团队。**client_id 是应用级公开值**(非机密,device flow 专为公共客户端设计),烤进二进制(`BAKED_CLIENT_ID` / 构建期 `DEVSYS_GH_CLIENT_ID`),**用户零配置**。token 存 OS 钥匙串。
- **iota_stronghold 2.x** 加密保险库存凭据,但**钥匙改成 OS 钥匙串里的随机设备密钥**(`keychain.rs`,account `vault-master`;钥匙串不可用回退 0600 文件)→ app 启动 Rust `setup` 里 `vault.unlock_device()` **自动解锁,零密码提示**。旧「密码=钥匙」的库设备密钥开不动 → 报 `VAULT_LEGACY`,UI 顶栏给「迁移」按钮(销毁旧库重建,旧凭据作废)。Argon2 派生仍在(设备密钥当「密码」喂进原 `unlock`)。
- **tsnet sidecar(内嵌 tailnet)✅ 出站已接线**:`tsnet-helper`(Go,已构建)登录 tailnet + 开 SOCKS5(:1055)+ 可代理入站 22。**tsnet 是用户态网络、不建 TUN 网卡,流量必须显式从它走** —— `ssh.rs::connect_direct` 与 `probe_reach` 对 `transport=tailnet` 的入口、且 tsnet 在跑时,经 SOCKS 拨(tokio-socks);否则裸 TCP(系统级 Tailscale 由 OS 路由,直连即通)。**仍缺:团队"同一 tailnet"组网机制**(各自浏览器登录=各进自己的 tailnet,互相看不见;GitHub org ≠ tailnet org。落法:官方 tailnet 邀成员 / pre-auth key 带外分发(机密不进 git)/ Headscale 自建 —— v2)。入站依赖 app 常开。
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
- **凭据只写不读**:前端永不拿明文;`has_secret` 用 servers.json 标记位(启动不读保险库,避免弹窗/慢)。保险库只在启动自动解锁 + 连接时读。
- **身份锚统一走 `verified_identity`(GitHub 登录优先 → tailnet SSO 备选)**:create_team / share / unshare / tailnet_identity / local_identity 全部同源。别再各自摸 `tn.status().identity()` —— 锚不一致,同一个人会在织物图裂成两个节点(roster=GitHub login,成员档 name=slug_login(同一 login),对齐靠的就是锚同源;回归测试 `team::fold_github_aligns_slug_and_login_case`)。
- **单活跃团队不变量**:store 里只允许 `mine` + **当前**团队的 `team:*` 项。执行点必须齐**四个**:①激活/加载(`fold_team_into_store` 清光所有 `team:*` 再写当前)②登出(`gh_logout`)③无团队兜底(`prune_team_sources`)④**启动对齐**(App 进门后无条件跑一次:有 org 重激活 / 有 teamPath 重加载 / 都没有则兜底清)。教训:曾只在「teamPath 为空」时激活 → 老状态永远轮不到清理,旧团队(labnet/neuroai)在列表里装死。`has_secret` 按机器名跨团队保留。
- **共享机器必须连 members/ 一起 push**:贡献写在 `members/<我>.yaml`,而 `gitsync::push` 早期只 `git add team.yaml` → 用户在 app 里共享的机器**永远同步不到队友和控制面**(整条 git→reconcile→ACL 闭环白跑)。现已:push 范围含 `members/`+ 自动写 `.gitignore`(花名册缓存不进 git)+ 无 upstream 时自动 `-u origin HEAD`;Servers 屏共享/取消共享后**自动推送**并显示结果。回归测试 `gitsync::push_includes_member_files`。
- **草稿目录会挡 clone**:`gh_activate_org` 的本地草稿(无 .git)存在时,`git clone` 拒绝非空目录 → 必须先挪开草稿再 clone、失败再还原(已实现),否则永远接不上管理员后建的真仓库。
- **门户不可重生成 oauth2-proxy.cfg**:门户端拿不到 .env,重生成会丢 client_secret/cookie_secret → 认证瘫痪。只原地改单行。
- **运行时真源在门户可写目录**(`~/gateway/data`、`~/gateway/oauth2`),绝不放 /etc。
- WKWebView 不支持元素级 requestFullscreen → 用 Tauri 窗口 API。

## 路线图(呼应北极星)

- **v0(接得上)✅ 已落地**:登录页去掉"本地/远程"切换(当时是纯本地密码解锁,**后已改为 GitHub 登录 + 设备密钥自动解锁**,见「技术选型」);服务器列表**按来源合并**(`Server.source = mine | team:<名>`,分组渲染、团队机只读);app 内「连接团队」入口(Team 屏占位 + 侧栏入口)。
- **v1(配得起)**:团队配置(建团队/邀成员/按人授权)→ 把门户 admin 泛化成 `team.yaml`。数据面复用原生 SSH。**贡献侧**:机器加 `shared_to: [team:<名>]`,把拓扑 + 授权声明提交进 team.yaml(凭据不出本机),队友那侧落成只读节点 —— 与 v0 的消费侧合成闭环。见下「共享算力模型」。
- **v2(长得开)**:**一人一容器兑现档位 ✅ 已落地**(见下「共享算力模型」);自助贡献机器 + tailnet 织物 + 规格/负载登记 → 动态算力网;调度接 **SkyPilot**(见下);数据成网(内容寻址 git-annex/DVC over tailnet);持久 agent 托管 + 沙箱(见下);裁判 agent 评审 = 去中心化同行评阅;出版/评审层接 **DeSci**(见下「DeSci 用法」)。

## 主页形态(✅ 已落地,反复用到)

**地图与工作面拆成两个侧栏视图**(不是并排驾驶舱 —— 用户嫌挤,拆开了):

- **主页 = 织物图,全宽**(`Home.tsx` → `TeamGraph`,`.cockpit.solo`):**以我为中心的力导向织物** —— 「我」固定圆心(无团队也合成中心节点),其余按关系分环(我的机器→门→队友→队友的机器,`RING` 常量;径向只是温和偏好,不是同心圆刻度盘)。节点大小随画布/密度自适应;悬停出身份与授权、主色环标「我」、门标菱形、机器→门有可达路径边。**状态直接画在点上**:已连接=常亮光环(`ssh_active` + `ssh://active` 事件推送,`Sessions` 存了 server 名)· 可达=呼吸脉冲 · 不可达=灰(`probe_reach`:并行 TCP 摸跳板链**入口**,Home 每 25s 一轮;tailnet 入口在内嵌 tsnet 运行时经其 SOCKS 摸,与拨号同一规则)。**吃统一织物 `Fabric`**(name 即 `ssh_open` 连接键)。回答「我在哪、够得着谁、现在谁能用」。
- **工作区 = 独立侧栏视图(网页版 tmux)**(`Workspace.tsx`,view=`workspaces`):标签 + 可分屏(2-up),每 pane 一条独立会话。回答「正在干什么」。**常驻挂载在 `App.tsx`(`.ws-host`),切到别的视图只 CSS 隐藏、绝不卸载**——否则会关掉所有活会话。
  - **铁律:非活动 pane 也保持挂载、只 CSS 隐藏**(`.tpane.hidden`)。
  - `TermView.tsx` = 可内嵌终端核心(xterm+传输),满屏 `Terminal.tsx` 与 pane 共用。带 `ResizeObserver`:切标签/分屏不触发 window.resize;**隐藏时尺寸 0 要跳过 refit**,否则 xterm 算出畸形行列。
- **本机也是节点(`localpty.rs` + 保留名 `~local`)**:织物图恒有「本机」节点(toFabric 注入,连到我、恒可达),点开 = **本地 PTY**(portable-pty 起 `$SHELL -l`),不走 SSH/不依赖 sshd。事件/写入/resize 与远程会话同构(`Sessions.register` 共用),前端传输层零改动;`ws` 名同样跑 tmux attach-or-create(本地也持久)。`ssh_open` 见 `~local` 即走本地分支,不查 store/保险库。
- **缝合 = 点节点/点 SSH → 切到工作区开 pane**:主页 canvas click(自己做命中测试,不依赖 hover)与 Servers 屏「SSH」按钮**都汇到 `App.openInWorkspace`**(openPane + setView("workspaces"))。tauri 下 SSH 一律进工作区 pane;web 仍走全屏 `Terminal`。连不上时说清原因(未入 store / 无凭据),不静默失败。
- **持久化 ✅**:每 pane 带稳定 `ws` 名,后端 `ssh.rs::open` 跑 `tmux new-session -A -s <ws>`(attach-or-create;`tmux_session_name` 白名单 `[A-Za-z0-9_-]` 是注入防线)。关 app / 断网,远端的活照跑;布局(开了哪些 pane)落 `localStorage devsys.wsp`,启动重开同名 ws → 重连即接回。无 tmux 的机回退登录 shell(仍可连,只是那次不持久)。
- **北极星落地**:v2 持久 agent 上后,**agent 也是节点** —— 地图上一个点、工作区里一个 pane;人和 AI **共用同一张地图、同一个工作面**。「人与 AI 共治」第一次有了具体 UI 形态。
- **剩余(Phase E 未做)**:可拖分隔条、>2 网格平铺、拖节点开 pane、agent/数据节点。

## 共享算力模型(v1→v2 定稿,反复用到)

一整条链的设计结论,别再重推:

- **共享的是「拓扑」,不是「凭据」**。贡献一台机 = 把"怎么到达它"给团队;密码/私钥永不出本机(铁律)。
- **身份到人,不用共享账号**。因为北极星信任原子 = 每处操作可追踪、用于校准 agent(担责即校准)。统一团队账号会断掉追踪链 → 禁用。
  - **终态**:Tailscale SSH + ACL —— 被共享机 `tailscale up --ssh` + 打 tag;policy 的 `ssh` 段声明 `src=group:团队, dst=tag:池, users=[各人账号], action=check/accept`;认证走 tailnet 身份(挂团队 SSO),零 authorized_keys、短期证书、离队即失效、自带 session recording(=现成的"可追踪")。等价开源自建 = 团队 SSH CA(step-ca)签短期证书,principal 内嵌身份。
  - **v1 过渡**:`shared_to` 把成员公钥从 team.yaml 同步进各自**独立账号**的 authorized_keys(身份到人、无共享号);sidecar 上了再无缝切 Tailscale SSH。
- **权限 = 档位**(机器主人选、也为此担责 = 责任为门;team.yaml 只记"这台对 team:X 开档几"):
  - **档0 纯跳板**(只借道、无 shell)· **档1 受限计算账号(默认)**(能登入跑计算、用配额内 GPU/CPU;无 sudo、进不了别人目录、占不满、可撤、可追踪)· **档2 完全信任**(有 sudo,仅核心成员)。
- **隔离 = 用容器兑现档位 ✅ 已落地(v2 第一刀)**。**"档"=开多少权 与 "容器"=怎么关 正交** —— `grants` 一字不改,只是兑现从 `useradd` 换成 `docker run`。落点:`team.rs::Sharing{isolation,image,limit,data}`(machine/device 上的 `sharing:` 块,缺省不写进 YAML)+ `provision.rs::plan_container` + 共享面板的「这些档位怎么兑现」。
  - **一人一容器,不是「共用一个容器内分权限」(定论)**。决定性理由:**档位只能逐人容器兑现** —— 容器的特权/GPU 是**容器级**的,共用容器给不了"张三档2、李四档1";per-user 容器才能逐人设能力。加分:per-user cgroup 天然配额(一人占不满饿死别人)、爆炸半径小(焚/坏自己的不波及别人)、身份到人=容器到人(担责链清晰)。成本低:**镜像层共享(环境存一份)**,空闲容器≈免费、闲时可 stop 到零占用。
  - **档位 → 容器能力映射(已实现)**:档0=不给容器,只建 nologin 账号 + `command="/bin/false",restrict,port-forwarding` 的 key(**档0 从前根本不建账号 → ProxyJump 认证不了,"纯跳板"是空话;现已修**);档1=普通容器(`--security-opt no-new-privileges --pids-limit` + 限额 + 只挂自己 volume);档2=`--privileged -v /:/host` + 容器内 sudo(等价宿主 root,仅核心)。**两种模式下宿主一律无 sudo**,档2 的 sudo 只在容器里。
  - **门房 = 登录 shell,不是 ForceCommand(实现决策)**:`usermod -s /usr/local/bin/devsys-enter`,脚本 `docker exec -u $u` 进本人容器。**不依赖 sshd 配置** —— ForceCommand 要 `Include sshd_config.d` 才生效,缺了就**静默退化成宿主 shell**(=档1 拿到裸机 bash,安全洞)。门房必须透传 `-c "$2"`,否则工作区的 `tmux new-session -A` 起不到容器里、持久化断掉。sshd 段只做**尽力而为的加固**(挡到宿主的端口转发),装不上就明说。
  - **主人的共享上限 = 父 cgroup 资源池(与容器数无关)**:`sharing.limit: {cpus, mem, gpus}` → provision 建 `devsys-shared.slice`(`CPUQuota`/`MemoryMax`),**所有借用容器 `--cgroup-parent` 挂它下面** → 加起来永不超父上限,开多少容器都突破不了。主人随时可改/设 0(收回)—— 责任为门、可撤。**GPU 不受父池约束**(设备直通不是可分配资源),每个容器都拿到声明的那组卡,UI/warnings 里必须说清。
  - **数据分层(已实现)**:①借用者自己的活 = **独立 named volume** `devsys-<user>-home` 挂 `/home/<user>`(焚容器也留);②宿主/别人的目录 = **默认完全不可见**(档1 不挂任何宿主 FS);③团队共享数据集 = **只有主人点名的才只读挂进来**(`sharing.data: [{host, as, mode: ro}]`)。**数据不出本机**:计算在数据旁跑 —— 正是"把计算带到数据旁"。
  - 信任队友 → 普通容器够;**不可信 / agent 生成的代码 → 必须强沙箱**(普通 docker 共享内核**不是**安全边界)。终态 = 每容器挂一个 tailscale 节点(容器=织物上一个节点,呼应"agent/数据也是节点"),那时端口转发的限制自然消失。
  - **✅ 第三档:一人一容器 · 免 root(`isolation: rootless`)** —— 前两档都要**被共享机的 root**,可校园里绝大多数人对实验室 GPU 服务器**只有一个普通账号**;「自助贡献机器」若非管理员不可,就只服务得了机器主人。换法:**队友不登宿主账号,直连他自己容器里的 sshd**(rootless podman 优先/回退 rootless docker,每人一个高位端口 `port_base`+排位,默认 2200)。于是 useradd / 改 sshd / 写 /etc 一件都不做。**门槛从「我是这台机的管理员」降到「我在这台机上有个账号」。**
    - **端口两端必须同一个纯函数算**(`TeamView::rootless_ports`,按成员名排序取位):一边开 2201、一边连 2202 的话**两头都不报错、只是永远连不上**。消费侧在 `fold_team_into_store` 里按同一函数改写 port+username;下发脚本给容器打 `--label devsys.port=<N>`,端口漂移(有人离队排位前移)自动重建。
    - **rootless 反而更安全**:容器 root 经 userns 映射到贡献者本人,**档 2 也拿不到宿主 root**。
    - 代价(都在 warnings 里明说,不假装):①**做不到档 0「借道」**(建不了宿主账号);②**没有父 cgroup 池**(建 systemd slice 要 root)→ 限额只到逐容器,N 人 N 倍;③rootless 默认没委派 CPU/内存 cgroup 控制器 → 带限额起不来时**退回不限额并打印管理员一次性修法**(`user@.service.d/delegate.conf`);④端口绑 0.0.0.0(拿不到 tailnet 网卡定向绑);⑤Linux 上要 `loginctl enable-linger`,否则登出即杀容器。
    - `provision_apply` 对 rootless **不套 `sudo -n`**(脚本自己也拒绝 uid 0)。
  - **平台边界:卡住 Linux 的不是 docker,是宿主账号那套**(`useradd`/`sudoers`/systemd slice)。所以 **①② 只支持 Linux(脚本开头 `uname -s` 就挡住并指路),③ 免 root 在 macOS 上照样能跑** —— 它只要 docker/podman,容器里永远是 Linux。macOS 分支的差异已在脚本里分好:不需要 linger(Docker Desktop 那个 VM 托管容器,不跟 SSH 会话走,但**跟桌面登录走** —— 没人登录时容器是停的),且**忽略 GPU 设置**(容器在虚拟机里,没有直通)。**Windows 仍不支持**:它的 SSH 默认 shell 不是 sh,`sh -s <<EOF` 根本喂不进去 —— 让 Windows 用户共享 **WSL2 里的那套 Linux**。回归测试 `only_rootless_works_beyond_linux`。
  - **✅ 共享前探测被共享机能力**(`probe_host_caps`):一次 SSH 问全 os/distro/docker/dockerd/podman/systemd/免密sudo/nvidia-smi,面板上三档不可用的直接灰掉+说清缺什么+给该发行版的安装命令。**给命令不替他装** —— 不静默改别人的机器。教训:曾是「选容器→共享→推 git→队友都看见了→最后下发才报没装 docker」。
  - **未做**:userns-remap(daemon 级开关,会掀翻宿主已有容器 → 不静默改别人的机器,只在 warnings 里提);每人子配额(现在每容器都吃父池上限,靠 cgroup 层级兜底);换镜像/换挂载需 `docker rm -f` 重建(脚本只 `docker update` 限额)。
  - **沙箱选型**(强沙箱**不自己造,接现成**;分成 CPU-only 与 GPU 两档,别混):
    - **CPU-only agent → 首选 CubeSandbox**(腾讯云,Apache-2.0,Rust + RustVMM/KVM,每沙箱独立内核的 microVM,冷启 <60ms、开销 <5MB、单机数千实例,**E2B SDK 兼容**)。选它而不是 Docker Sandboxes 的理由:**开源可自持**,能直接跑在我们自己的贡献节点上(符合「自持」);E2B 兼容白捡 code-interpreter 生态。**Docker Sandboxes** 仍列备选(跨平台、复杂度被 Docker 封装)。
    - **GPU + agent(我们核心)→ 上面两个都撑不起**:microVM/Firecracker/KVM 天生不支持 GPU 直通,CubeSandbox 的 README/roadmap 里 GPU/CUDA/直通一个字都没有。→ 盯 **NVIDIA OpenShell**(GTC 2026 开源、GPU-native、原生 claude、声明式策略,最对味)/ Kata(microVM+GPU 直通)/ gVisor+nvproxy(拦 CUDA,安全性反而更好)/ Modal(托管)。**GPU 直通削弱隔离是必然折中**(驱动=共享攻击面);真直通要裸机+IOMMU。
    - CubeSandbox 的两条注意:①它**面向短命沙箱**(ephemeral + auto-pause/resume),我们要的是工作区里**常驻**的持久 agent —— roadmap 的「跨机暂停与恢复」方向对但语义不是;②它自带整套控制面(CubeAPI/CubeMaster/Cubelet/CubeVS/CubeEgress),**和 SkyPilot 同一个坑:别让它的中心 API server 成为身份/治理真源** —— 它只是某台贡献节点上的一种 runtime,ACL/身份/担责链还是我们的。要 x86_64/ARM64 Linux + KVM(云 VM 需嵌套虚拟化)。**现在不接,v2 真上 agent 节点时再接。**
- **SkyPilot = 可插的作业调度组件,不是底座**。它把机器抽象成"能 SSH 的盒子",恰好吃我们 tailnet 上登记好的算力池:登记成它的 **SSH Node Pool**(`~/.sky/ssh_node_pools.yaml` → `sky ssh up` → `sky launch --infra ssh/<pool>`)或对接 on-prem K8s,`sky launch` 就调度(比价/spot/队列/serve)。它的团队层(API server + Workspaces + User/Admin RBAC,v0.10 起)是**中心化**的、单团队够用 —— 但**去中心化联邦 + 每人贡献节点 + 责任追踪仍是我们的**,别让它的中心 API server 成为身份/治理真源(可共用同一 IdP 对齐身份)。其 Sandboxes 亦可选用。**松耦合接,别焊死**(同时留住直连 SSH / SkyPilot / 未来别的调度器)。

## 可达性与模型完备性(已知缺口清单,别误以为 model 已封口)

**可达性(reachability)—— 主体已覆盖**。核心原则:**内网 ≠ 需要跳板**;能出 443 的机就自己上 mesh 直连,跳板/桥只给"出不了公网 / 装不了 tailscale"的节点。
- tailnet 直连 P2P ✅ · 同 LAN `direct` ✅ · 公网 IP `direct` ✅ · NAT/CGNAT/对称 NAT 打洞失败→DERP 兜底 ✅(tailscale 扛,非我们的事)
- **多跳跳板链 ✅ 已实现**:jump 是"指向另一台机的名字",多跳=链式引用(target.jump→login、login.jump→edge)。`store::jump_chain` 走链(防环/防悬空),`ssh.rs::build_conn` 嵌套多层 direct-tcpip。校园"edge→登录节点→算力节点"两跳以上兑现。数据模型/前端未改。
- **校园无出网机**:靠校园里一台能出网的邻居当门——**subnet router**(`--advertise-routes` 广播网段,成员用内网 IP 直达,最省)或 **jump host**(SSH 层);想全内网自持则自建 **Headscale + 内网 DERP**。

**团队自持组网 ✅ 已真机跑通(Headscale,2026-07-25)**:阿里云 `39.97.7.248` 跑 **Headscale v0.29.2** 控制面(443+内置 Let's Encrypt TLS-ALPN-01,避开 nginx 的 80;官方 DERP 不自建,零数据转发;SQLite;user `neuroai`+pre-auth key),mac 内嵌 tsnet(`--control https://headscale.psyagent.top`)连上拿到 100.64.0.x、online。**一个团队一张网、不装系统 tailscale、控制面自持全部兑现**。细节+坑见 memory [[devsys-headscale-selfhost]]。**头号坑:系统代理(clash/mihomo/VPN)会把 tsnet→控制面的 HTTPS 截成 `fetch control key: EOF`** → Rust 给 helper 设 `NO_PROXY=<控制面host>` 直连(仅 control 非空时)。**✅ 成员入网已改 OIDC 并验过**:阿里云 443 = **Caddy**(TLS-ALPN-01 自动签证;`/dex/*`→Dex、`/*`→Headscale:8080)+ **Dex**(GitHub OAuth→OIDC 的桥,`orgs:` 限定组织即网络边界)。成员 app 无 authkey 连 → 自动弹浏览器 → GitHub 授权 → 入网,headscale **自动按 GitHub 身份建 user**(身份到人贯穿网络层),重启静默重连。坑:①阿里云拦未备案域名 80 端口 → HTTP-01 永远失败,只能 TLS-ALPN-01;②Dex 用 issuer 路径当路由前缀 → Caddy 要 `handle /dex/*` 不剥前缀;③**tsnet 状态目录必须按控制面分**(`tsnet_dir()`),否则官方/自建身份互相污染。pre-auth key 退化为「仅无人值守机器」。

**团队组网(定论,反复用到)**:一切路径的**地基 = 同一张 tailnet**(各自浏览器登录=各进自己的网,互相看不见 → 必须先砌这块砖)。**已定并落地 = 自建 Headscale + Dex/GitHub OIDC**(见上一段;曾考虑官方 Tailscale,因要完全自持而放弃)。team.yaml 加 `tailnet` 字段声明团队用哪张网(TeamRoot/TeamView 已带;Team 屏「团队网络」条把接入从设置深处提为一等基建)。**理想拓扑 = 不经过任何人**:能出 443 的设备各自 `tailscale up` 直接上 mesh、P2P 直连;**「成员经我连我的设备」是内网 fallback**,不是常态。fallback 三实现能力递减:①subnet router(网络层,整网段,**要系统级 tailscaled**,内嵌 tsnet 做不了内核转发)②jump host(SSH 层,已支持多跳链)③**tsnet 应用层门 = 现有积木组合**:我 app 开 ingress(本机 :22 上 tailnet)+ 对本机 provision 档0(纯跳板)+ 内网设备 team.yaml 里 `jump=我本机` → 队友经我 ProxyJump 进去(复用 SOCKS 出站 + direct-tcpip 多跳,**不改 Go helper**)。混合场景 = 直连 + 经门并存。v2 自动化:pre-auth key 经保险库带外分发 / Headscale 自持。

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

已完成(截至本轮):Tauri 阶段0–2(脚手架+传输抽象、本地拓扑/凭据、russh 原生 SSH 真连成功);Stronghold 保险库;登录门(密码=钥匙);SSH config 导入(读 ~/.ssh/config + 文件选择 + 带 IdentityFile 私钥导入);本地退出登录;**v0 三件事(纯本地解锁 / 按来源分组 / 连接团队入口)** + 清理废弃 creds.rs;**对齐模型**(角色降为纯名字、tier 只在 grants、guest→pub);**切面模型**(人共享自身设备 = 人本身也是算力,`member.device` → merge 折成 is_self 机器);**离线「我」**(`local_identity`:内建 tsnet → 系统 tailscale → OS 用户,仅显示用);**多跳跳板链**(`store::jump_chain` 走链防环 + `ssh.rs` 嵌套 direct-tcpip);**subnet router 一等化 + 门作一等节点**(`advertises` CIDR);**主页驾驶舱**(织物图 + 工作区,点节点开 pane);**主页/工作区拆成两个侧栏视图**(主页纯织物图全宽、工作区常驻独立视图,Servers SSH 也汇入工作区);**工作区 tmux 持久化**(`ssh.rs` attach-or-create + `tmux_session_name` 注入白名单 + 布局落 localStorage);**tailnet 一次登录长期保留**(`tailnet.json` 自启意向 + `setup` 静默重连);**登录改 GitHub device flow + 保险库设备密钥自动解锁**(`ghauth.rs`/`keychain.rs`/`GithubGate.tsx`,client_id 烤进二进制、用户零配置,旧密码库经顶栏「迁移」按钮重建);**org→团队闭环**(一个 org = 一个约定仓库 `<org>/ait-team` = 一份 team.yaml:登录选/切 org → 自动 clone/pull 或退化本地草稿 → 同步花名册 → 折进 store;`gh_init_team` 生成模板、`gh_push_team` API 建仓+推送(scope `read:org repo`)、切换组织/手动输 org/授权页入口;**人=org 实时拉、机器=约定仓库共享**,OAuth 花名册只是便利、git 权限才是硬依赖);**一致性收口**(单活跃团队不变量 + 草稿挡 clone 修复 + 身份锚统一 GitHub 优先 + Team 屏旧表单收进「高级」、花名册 token 输入删除改钥匙串自动);**v2 第一刀:一人一容器兑现档位**(`team.rs::Sharing` + `provision.rs::plan_container` + 共享面板「怎么兑现」+ 下发弹窗显示容器/池/数据集;顺带修两个既存 bug:档0 从不建账号→纯跳板认证不了、已共享机点「改授权」永远看不到档位表)。
