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

  // ── GitHub org 花名册：成员/公钥/角色自动导出，消掉手动登记 ──
  bindGithub(path: string, org: string, roleMap: Record<string, string>): Promise<void>;
  syncGithub(path: string, token?: string): Promise<{ count: number; with_keys: number; members: { login: string; pubkeys: string[]; role: string }[] }>;

  // ── 贡献侧：把自己的机器给团队（共享拓扑，凭据不出本机）──
  readTeamView(path: string): Promise<TeamView>; // 合并视图（成员/机器/角色），也是拓扑图数据源
  createTeam(path: string, teamName: string, member: string, pubkey: string, role?: string): Promise<void>;
  addMember(path: string, name: string, pubkey: string, role?: string, identity?: string): Promise<TeamView>;
  // grants = 角色→档位（RBAC）；member = 我是谁（写进我的 members/<我>.yaml）
  shareServer(teamPath: string, member: string, server: string, grants: Record<string, number>): Promise<Server[]>;
  unshareServer(teamPath: string, member: string, server: string): Promise<Server[]>;
  myPubkeys(): Promise<string[]>; // 本机 ~/.ssh/*.pub（登记自己时用）
  pickTeamSavePath(): Promise<string | null>; // 新建 team.yaml 的另存为
  // 探测本机作为节点（地址 / sshd / 能当算力还是跳板）。没有本地远程之分，只有节点。
  detectSelf(): Promise<SelfNode>;

  // ── 内建 tailnet（tsnet sidecar）：零系统依赖的 tailnet 节点 ──
  tailnetStatus(): Promise<TailnetStatus>;
  tailnetUp(authkey: string, ingress: boolean): Promise<void>;
  tailnetDown(): Promise<void>;
  // 验证过的团队身份（来自 tailnet SSO 登录）。login 为空 = 未连/未登录。
  tailnetIdentity(): Promise<{ login: string; display: string; name: string }>;

  // ── 授权下发：让队友真能登进去（先 preview 看脚本，再 apply 执行）。档位按角色自动算 ──
  provisionPreview(teamPath: string, server: string): Promise<ProvisionPlan>;
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
  roles: Record<string, { tier: number }>;
  members: { name: string; identity: string; pubkey: string; role: string }[];
  machines: {
    name: string; host: string; port: number; jump?: string | null;
    username: string; transport: string;
    grants: Record<string, number>; // 角色 → 档位（RBAC）
    owner: string;                   // 贡献者
  }[];
}

// 授权下发计划：要在被共享机上以 root 跑的脚本（先给人看，再执行）。
export interface ProvisionAccount {
  name: string;
  role: string;
  tier: number;
  sudo: boolean;
}
export interface ProvisionPlan {
  server: string;
  accounts: ProvisionAccount[]; // 每个可登入成员一条（含各自档位/是否 sudo）
  any_sudo: boolean;
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
