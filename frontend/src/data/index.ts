// 数据层抽象：服务器拓扑 / 凭据的读写，一套 UI 同喂两目标。
//   web    → 门户 REST（api.me / api.saveSettings）；服务器由门户/Admin 管，前端不增删。
//   tauri  → 本地 Rust 命令（list_servers / upsert_server / del_server / save_credential …）。
import type { Me, SettingsBody, Server } from "../api";
import { isTauri } from "../transport";

// 新增/编辑一台服务器时提交的字段（tauri 本地拓扑）。
export interface ServerInput {
  name: string;
  host: string;
  port: number;
  jump?: string | null;
  username?: string;
  auth?: "password" | "key";
  transport?: "direct" | "jump" | "tailnet";
}

export interface VaultState {
  exists: boolean; // 已建过保险库
  unlocked: boolean; // 本会话已解锁
  legacy?: boolean; // 旧密码库，设备密钥开不动 → 需迁移
}

// GitHub 登录（device flow）状态。登录 = GitHub 身份，取代本地密码。
export interface GhState {
  logged_in: boolean;
  login: string;      // GitHub 用户名
  org: string;        // 选定的团队组织
  configured: boolean; // 是否配了 OAuth App client_id
}
export interface GhDeviceStart {
  device_code: string;
  user_code: string;       // 给用户念/贴的短码
  verification_uri: string; // 浏览器打开这里输码
  verification_uri_complete: string; // 码已预填的直达链接（自动打开后只需点 Authorize）
  interval: number;         // 轮询间隔（秒）
  expires_in: number;
}
export interface GhPoll {
  status: "pending" | "slow_down" | "ok" | "error";
  login?: string;
  orgs: string[];
  error?: string;
}
export interface GhActivateResult {
  path: string;            // team.yaml 路径（空 = 没建成）
  org: string;
  mode: "repo" | "local";  // repo=克隆到约定仓库 / local=仓库缺失，退化本地草稿（仅成员）
}

// ~/.ssh/config 解析出的一台可导入主机。
export interface SshHost {
  name: string;
  host: string;
  port: number;
  username: string;
  jump: string | null;
  auth: "password" | "key";
  identity_file: string | null;
}

export interface DataSource {
  // 载入「我 + 服务器清单」。web 带门户身份；tauri 是本地单用户。
  loadMe(): Promise<Me>;
  // 存凭据（username/auth + 可选 secret）。
  saveCredential(b: SettingsBody): Promise<{ ok: boolean; has_secret: boolean }>;
  // 以下为自包含 app 的本地拓扑管理（web 不支持，由门户负责）。
  upsertServer(s: ServerInput): Promise<Server[]>;
  delServer(name: string): Promise<Server[]>;
  delCredential(name: string): Promise<void>;
  // 凭据保险库（tauri Stronghold）：web 无保险库，恒为已解锁。
  // 登录密码 = 保险库主密码（首次创建、之后解锁）。
  vaultState(): Promise<VaultState>;
  vaultUnlock(password: string): Promise<void>;
  vaultAutoUnlock(): Promise<VaultState>; // 用设备密钥自动解锁（无需密码）
  vaultMigrate(): Promise<void>;          // 迁移旧密码库（销毁旧凭据后设备密钥重建）
  vaultLock(): Promise<void>; // 退出登录（锁库）
  // 忘记密码的唯一出路：销毁保险库（凭据全丢，不可逆），拓扑保留。
  vaultReset(): Promise<void>;

  // ── GitHub 登录（device flow）：登录 = GitHub 身份，本地不再验密码 ──
  ghState(): Promise<GhState>;
  ghDeviceStart(): Promise<GhDeviceStart>;    // 起 device flow，拿短码 + 授权 URL
  ghDevicePoll(deviceCode: string): Promise<GhPoll>; // 轮询换 token
  ghOrgs(): Promise<string[]>;                 // 列所属组织（切换团队用）
  ghSetOrg(org: string): Promise<void>;        // 选定团队组织
  // 选/切 org → 自动 clone 约定仓库 <org>/ait-team（或退化本地草稿）+ 同步花名册。
  ghActivateOrg(org: string): Promise<GhActivateResult>;
  // 为当前 org 生成团队仓库初始模板（team.yaml + members/.gitkeep + README + git init），返回 team.yaml 路径。
  ghInitTeam(org: string): Promise<string>;
  ghLogout(): Promise<void>;                   // 退出 GitHub 登录
  ghAuthorizeUrl(): Promise<string>;           // 本 app 的 GitHub 授权管理页（Grant/Request org 访问）
  ghNewRepoUrl(org: string): Promise<string>;  // GitHub 新建仓库页（org + ait-team + private 预填）
  ghPushTeam(org: string, path: string): Promise<string>; // 连上约定远程 <org>/ait-team 并推送
  openUrl(url: string): Promise<void>;         // 用系统浏览器打开链接（GitHub 授权页）

  // ── 织物图状态源 ──
  sshActive(): Promise<string[]>;              // 有活 SSH 会话的服务器名(增量靠 ssh://active 事件)
  probeReach(): Promise<Record<string, boolean>>; // TCP 摸每台机的入口:可达/不可达
  getUsername(): Promise<string>;
  setUsername(name: string): Promise<void>;
  // 解析 SSH config（path 空=默认 ~/.ssh/config）。仅 tauri。
  readSshConfig(path?: string): Promise<{ path: string; hosts: SshHost[] }>;
  // 导入所选主机（写拓扑 + 有 IdentityFile 则读私钥入库）。
  importHosts(hosts: SshHost[]): Promise<Server[]>;
  // 打开文件选择器选一个 config 文件，返回路径（取消返回 null）。
  pickSshConfigFile(): Promise<string | null>;
  // 加载 team.yaml：团队共享机作为只读节点（source=team:<名>）合并进列表。仅 tauri。
  loadTeam(path: string): Promise<LoadTeamResult>;
  // 无活跃团队时清掉所有 team:* 机器（单活跃团队不变量的兜底）。仅 tauri。
  pruneTeamSources(): Promise<void>;
  // 打开文件选择器选一个 team.yaml，返回路径（取消返回 null）。
  pickTeamFile(): Promise<string | null>;
  // 把 team.yaml 的 tier 档位编译成 Tailscale ACL 计划（只产出计划，不改 tailnet）。
  compileAcl(path: string): Promise<AclPlan>;

  // ── GitHub org 花名册：成员/公钥/角色自动导出，消掉手动登记 ──
  bindGithub(path: string, org: string, roleMap: Record<string, string>): Promise<void>;
  syncGithub(path: string, token?: string): Promise<{ count: number; with_keys: number; members: { login: string; pubkeys: string[]; role: string }[] }>;

  // ── 贡献侧：把自己的机器给团队（共享拓扑，凭据不出本机）──
  readTeamView(path: string): Promise<TeamView>; // 合并视图（成员/机器/角色），也是拓扑图数据源
  createTeam(path: string, teamName: string, member: string, pubkey: string, role?: string): Promise<void>;
  addMember(path: string, name: string, pubkey: string, role?: string, identity?: string): Promise<TeamView>;
  // grants = 角色→档位（RBAC）；member = 我是谁（写进我的 members/<我>.yaml）
  // asDevice=true → 写进 member.devices（我自己的设备，跟着人走）；
  // false → 写进 machines（我贡献的服务器，是基建）。只有本人分得清，所以由他勾。
  shareServer(teamPath: string, member: string, server: string, grants: Record<string, number>, sharing?: Sharing | null, asDevice?: boolean): Promise<Server[]>;
  unshareServer(teamPath: string, member: string, server: string): Promise<Server[]>;
  myPubkeys(): Promise<string[]>; // 本机 ~/.ssh/*.pub（登记自己时用）
  pickTeamSavePath(): Promise<string | null>; // 新建 team.yaml 的另存为
  // 探测本机作为节点（地址 / sshd / 能当算力还是跳板）。没有本地远程之分，只有节点。
  detectSelf(): Promise<SelfNode>;

  // ── 内建 tailnet（tsnet sidecar）：零系统依赖的 tailnet 节点 ──
  tailnetStatus(): Promise<TailnetStatus>;
  tailnetLog(): Promise<string[]>;   // sidecar 最近日志（诊断接入卡住）
  // control 空 = 官方 Tailscale;填团队 Headscale URL = 接入团队自持网。
  tailnetUp(authkey: string, ingress: boolean, control?: string): Promise<void>;
  tailnetDown(): Promise<void>;
  // 验证过的团队身份（来自 tailnet SSO 登录）。login 为空 = 未连/未登录。
  tailnetIdentity(): Promise<{ login: string; display: string; name: string }>;
  // 「我是谁」的显示用身份（内建 tsnet → 系统 tailscale → OS 用户兜底）。仅用于图上标「我」。
  localIdentity(): Promise<{ login: string; display: string; name: string }>;

  // ── 授权下发：让队友真能登进去（先 preview 看脚本，再 apply 执行）。档位按角色自动算 ──
  provisionPreview(teamPath: string, server: string): Promise<ProvisionPlan>;
  probeHost(server: string): Promise<HostProbe>;
  provisionApply(teamPath: string, server: string): Promise<ProvisionResult>;

  // ── team.yaml 的 git 同步（配置即代码：团队配置放 git）──
  teamGitStatus(path: string): Promise<GitStatus>;
  teamGitPull(path: string): Promise<string>;
  teamGitPush(path: string, message: string): Promise<string>;
  teamGitClone(url: string, dest: string): Promise<string>; // 返回 team.yaml 路径
  pickDir(): Promise<string | null>; // 选克隆目标目录
}

export interface GitStatus {
  is_repo: boolean;
  branch: string;
  dirty: boolean; // team.yaml 有未提交改动
  has_remote: boolean;
  remote: string;
}

// 内建 tailnet 节点状态（tsnet sidecar 经 stdout 报的行分隔 JSON）。
export interface TailnetStatus {
  state: string; // starting | running | error | stopped
  backend?: string;
  ip?: string;
  name?: string;
  auth_url?: string; // 需要浏览器登录时的 URL（没填 authkey 时）
  socks?: string;
  ingress?: boolean;
  hint?: string;    // 我们替用户读 helper 日志得出的人话结论（卡住时最有用）
  login?: string;   // 验证过的 SSO 登录名
  display?: string;
  error?: string;
}

// 本机作为节点：它可能本身就是算力，也可能是通往你内网的跳板。
export interface SelfNode {
  hostname: string;
  username: string;
  addrs: { value: string; kind: "tailnet" | "lan" | "hostname"; hint: string }[];
  sshd: boolean; // 没开 sshd → 队友连不进来
  notes: string[];
}

// 合并视图：team.yaml + members/*.yaml 合并出的统一结构（也是拓扑图数据源）。
export interface TeamView {
  team: string;
  tailnet?: string; // 团队声明的 tailnet（可达性地基；管理员在 team.yaml 填）
  roles: string[]; // 纯角色名（无 tier）—— tier 只在 machine.grants
  members: { name: string; identity: string; pubkey: string; role: string }[];
  machines: {
    name: string; host: string; port: number; jump?: string | null;
    username: string; transport: string;
    grants: Record<string, number>; // 角色 → 档位（RBAC）
    owner: string;                   // 贡献者
    is_self: boolean;                // 是不是 owner 本人的设备（图里折进人节点）
    advertises: string[];            // 广播的子网 CIDR（非空 = subnet router / 网关 = 门）
    sharing: Sharing;                // 档位怎么兑现（裸机账号 / 一人一容器 + 上限 + 数据集）
  }[];
}

// ── 统一织物（驾驶舱地图的数据源）────────────────────────
// 北极星「没有本地/远程之分，只有节点」：本地自持服务器与团队共享机在同一张图里。
// 机器按 name 合并 —— store 服务器名 == 团队机名 == 成员设备名，这个 name 就是 ssh_open 的连接键。
export interface FabricMachine {
  name: string;                   // 连接键（ssh_open 按它解析）
  host: string;
  jump?: string | null;
  transport?: string;
  grants: Record<string, number>; // 团队授权（纯本地机为空）
  owner: string;                  // 贡献者（纯本地机为空）
  is_self: boolean;               // owner 本人的设备（图里折进人节点）
  advertises: string[];           // 广播的子网（非空 = 门/网关）
  source: string;                 // mine | team:<名>
  connectable: boolean;           // 在本地 store 里 → ssh_open 能解析
  has_secret: boolean;            // 已配凭据
}
export interface Fabric {
  team: string;                   // 空 = 未加载团队（图里只有本地机）
  roles: string[];
  members: { name: string; identity: string; pubkey: string; role: string }[];
  machines: FabricMachine[];
}

// 本机节点的保留名(与 Rust localpty::LOCAL_NODE 一致)。点它开本地 PTY,不走 SSH。
export const LOCAL_NODE = "~local";

// 合成统一织物：本地 store 服务器 ∪ 团队视图机器（按 name 合并）。
// 团队视图给 owner/grants/is_self/advertises；store 给可连性与真实传输方式。
export function toFabric(servers: Server[], team?: TeamView | null): Fabric {
  const byName = new Map(servers.map((s) => [s.name, s]));
  const machines: FabricMachine[] = [];
  const seen = new Set<string>();

  // 本机也是节点(仅桌面):恒可达、恒可点 —— 点开是本地 PTY 终端。
  if (isTauri) {
    machines.push({
      name: LOCAL_NODE, host: "本地终端", jump: null, transport: "local",
      grants: {}, owner: "", is_self: false, advertises: [],
      source: "local", connectable: true, has_secret: true,
    });
  }

  for (const m of team?.machines ?? []) {
    const s = byName.get(m.name);
    seen.add(m.name);
    machines.push({
      name: m.name,
      host: m.host,
      jump: s?.jump ?? m.jump ?? null,
      transport: s?.transport ?? m.transport,
      grants: m.grants ?? {},
      owner: m.owner,
      is_self: m.is_self,
      advertises: m.advertises ?? [],
      source: s?.source ?? `team:${team?.team ?? ""}`,
      connectable: !!s,             // 没进 store（没加载团队/重名跳过）→ 连不上
      has_secret: !!s?.has_secret,
    });
  }
  // 纯本地机（没在团队里）：孤立机器节点，无 owner/无 grants，照样可点连。
  for (const s of servers) {
    if (seen.has(s.name)) continue;
    machines.push({
      name: s.name,
      host: s.host,
      jump: s.jump ?? null,
      transport: s.transport,
      grants: {},
      owner: "",
      is_self: false,
      advertises: [],
      source: s.source ?? "mine",
      connectable: true,
      has_secret: s.has_secret,
    });
  }
  return {
    team: team?.team ?? "",
    roles: team?.roles ?? [],
    members: team?.members ?? [],
    machines,
  };
}

// ── 档位的兑现方式（v2：一人一容器）──────────────────────
// 「档」= 开多少权（grants）与「容器」= 怎么关（sharing），两者**正交**：
// 改隔离方式不动 grants，反之亦然。
export interface ShareLimit {
  cpus?: number | null; // CPU 核数（可小数）
  mem?: string;         // "32g" / "4096m"
  gpus?: string;        // "all" / "2" / "device=0,1"（GPU 不受 cgroup 父池约束）
}
export interface ShareDataset {
  host: string;   // 宿主上的路径
  as?: string;    // 容器内路径（空 = 同 host）
  mode?: string;  // ro（默认）| rw
}
// container 一人一容器（**主线，三平台**：Linux / macOS / Windows 都只要 docker/podman）
// account   裸机账号（**Linux only 备选**：要 useradd/sudoers，给不了限额与隔离）
export type Isolation = "account" | "container";
export interface Sharing {
  isolation: Isolation;
  image?: string;             // 空 = 用 app 现建的基础镜像（带 tmux；免 root 的还带 sshd）
  limit?: ShareLimit | null;  // 主人的借出上限 → 父 cgroup 池（免 root 下只到逐容器）
  data?: ShareDataset[];      // 点名只读挂进来的数据集
  port_base?: number | null;  // 每人一个高位端口，从这里往上排（默认 2200）
}
export const DEFAULT_SHARING: Sharing = { isolation: "container", image: "", limit: null, data: [] };

// 被共享机的实际现状。**全靠容器引擎自己回答**（`docker version` / `docker info`），
// 不依赖宿主 shell —— 所以 Windows 上也能探。共享**之前**就探，
// 别等到下发那一步才告诉主人「这台机没装 docker」。
export interface HostProbe {
  os: string;       // linux | darwin | windows | unknown（宿主系统）
  engine: string;   // docker | podman（空 = 没找到容器引擎）
  rootless: boolean;
  desktop: boolean; // 容器跑在 Docker Desktop / podman machine 的 VM 里
  ncpu: number;     // 引擎看得到的核数（Desktop 上 = VM 配额 = 外层父池）
  mem_mib: number;
  gpu: boolean;
  host_root: boolean; // 宿主有 root / 免密 sudo（建真父池用）
  systemd: boolean;
  install_hint: string;
}

// 授权下发计划：要在被共享机上以 root 跑的脚本（先给人看，再执行）。
export interface ProvisionAccount {
  name: string;
  role: string;
  tier: number;
  sudo: boolean;
  mode: "forward" | "account" | "container"; // 这个人怎么被兑现
  container?: string;                         // 他的容器名（容器模式）
  limits?: string;                            // 限额人话
  port?: number;                              // 免 root：他专属的高位端口
}
// 一条要在被共享机上执行的命令。容器档全是 **OS 中立**的裸 token
// （宿主 shell 不参与解释），要写文件的那条内容走 SSH stdin。
export interface ProvisionCmd {
  label: string;
  argv: string[];
  stdin?: string;
  optional: boolean;
  host_shell: boolean;
}
export interface ProvisionPlan {
  server: string;
  accounts: ProvisionAccount[]; // 每个可登入成员一条（含各自档位/是否 sudo）
  any_sudo: boolean;
  script: string;          // 裸机账号档（Linux only）才有
  commands: ProvisionCmd[]; // 容器档才有
  warnings: string[];
  isolation: Isolation;
  pool: string;        // 借出上限的人话；空 = 没设上限
  pool_kind: string;   // cgroup（真父池，硬顶）| divided（按人数分摊）| 空
  datasets: string[];  // 只读挂进容器的共享数据集
}

export interface ProvisionResult {
  ok: boolean;
  code: number;
  output: string;
}

// 加载 team.yaml 的结果。added=新增几台，skipped=因与本地同名而跳过的名字。
export interface LoadTeamResult {
  team: string;
  added: number;
  skipped: string[];
  servers: Server[];
}

// tier 档位编译成的 Tailscale ACL 计划。只是「计划」——供人 review 后自己贴进 tailnet policy。
export interface AclPlan {
  team: string;
  groups: [string, string[]][]; // group 名 → 成员名（各角色）
  tag_owners: string[];
  ssh: {
    action: string; // accept | check
    src: string[];
    dst: string[];
    users: string[];
    check_period?: string;
    role: string;
  }[];
  machines: {
    name: string;
    host: string;
    tag: string;
    owner: string;
    command: string;     // 这台机上要跑的 tailscale 命令
    grants_desc: string; // 如 "core→2, member→1"
    hardening: string;
  }[];
  notes: string[];
}

export const supportsLocalTopology = isTauri;

import { webData } from "./web";
import { tauriData } from "./tauri";

export const data: DataSource = isTauri ? tauriData : webData;
