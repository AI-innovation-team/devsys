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
}

// 加载 team.yaml 的结果。added=新增几台，skipped=因与本地同名而跳过的名字。
export interface LoadTeamResult {
  team: string;
  added: number;
  skipped: string[];
  servers: Server[];
}

export const supportsLocalTopology = isTauri;

import { webData } from "./web";
import { tauriData } from "./tauri";

export const data: DataSource = isTauri ? tauriData : webData;
