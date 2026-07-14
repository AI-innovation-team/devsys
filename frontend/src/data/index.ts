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
  exists: boolean; // 已建过保险库（有主密码）
  unlocked: boolean; // 本会话已解锁
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
  vaultLock(): Promise<void>; // 退出登录（锁库）
  // 忘记密码的唯一出路：销毁保险库（凭据全丢，不可逆），拓扑保留。
  vaultReset(): Promise<void>;
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
  // 打开文件选择器选一个 team.yaml，返回路径（取消返回 null）。
  pickTeamFile(): Promise<string | null>;
  // 把 team.yaml 的 tier 档位编译成 Tailscale ACL 计划（只产出计划，不改 tailnet）。
  compileAcl(path: string): Promise<AclPlan>;

  // ── 贡献侧：把自己的机器给团队（共享拓扑，凭据不出本机）──
  readTeamFile(path: string): Promise<TeamConfig>;
  createTeam(path: string, teamName: string, member: string, pubkey: string): Promise<void>;
  addMember(path: string, name: string, pubkey: string): Promise<TeamConfig>;
  shareServer(teamPath: string, server: string, tier: number): Promise<Server[]>;
  unshareServer(teamPath: string, server: string): Promise<Server[]>;
  myPubkeys(): Promise<string[]>; // 本机 ~/.ssh/*.pub（登记自己时用）
  pickTeamSavePath(): Promise<string | null>; // 新建 team.yaml 的另存为
  // 探测本机作为节点（地址 / sshd / 能当算力还是跳板）。没有本地远程之分，只有节点。
  detectSelf(): Promise<SelfNode>;

  // ── 内建 tailnet（tsnet sidecar）：零系统依赖的 tailnet 节点 ──
  tailnetStatus(): Promise<TailnetStatus>;
  tailnetUp(authkey: string, ingress: boolean): Promise<void>;
  tailnetDown(): Promise<void>;

  // ── 授权下发：让队友真能登进去（先 preview 看脚本，再 apply 执行）──
  provisionPreview(teamPath: string, server: string, tier?: number): Promise<ProvisionPlan>;
  provisionApply(teamPath: string, server: string, tier?: number): Promise<ProvisionResult>;

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

export interface TeamConfig {
  team: string;
  members: { name: string; pubkey?: string }[];
  machines: { name: string; host: string; port: number; jump?: string | null; username?: string; transport: string; tier: number }[];
}

// 授权下发计划：要在被共享机上以 root 跑的脚本（先给人看，再执行）。
export interface ProvisionPlan {
  server: string;
  tier: number;
  accounts: string[]; // 将建立的独立账号（身份到人）
  sudo: boolean;      // 仅 tier 2
  script: string;
  warnings: string[];
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
  group: string;
  members: string[];
  tag_owners: string[];
  ssh: {
    action: string; // accept | check
    src: string[];
    dst: string[];
    users: string[];
    check_period?: string;
  }[];
  machines: {
    name: string;
    host: string;
    tier: number;
    tag: string;
    command: string;   // 这台机上要跑的 tailscale 命令
    hardening: string; // 屋内层（OS/容器）责任 —— Tailscale 不管这层
  }[];
  notes: string[];
}

export const supportsLocalTopology = isTauri;

import { webData } from "./web";
import { tauriData } from "./tauri";

export const data: DataSource = isTauri ? tauriData : webData;
