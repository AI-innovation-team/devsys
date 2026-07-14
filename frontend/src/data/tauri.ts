// Tauri 数据源：走本地 Rust 命令（src-tauri/src/lib.rs）。
// 服务器拓扑存 app 配置目录 servers.json，凭据存 OS keychain。
import type { Me, Server, SettingsBody } from "../api";
import type { DataSource, ServerInput } from "./index";

const invoke = (cmd: string, args?: Record<string, unknown>): Promise<unknown> =>
  (window as unknown as { __TAURI__: any }).__TAURI__.core.invoke(cmd, args);

export const tauriData: DataSource = {
  async loadMe(): Promise<Me> {
    const servers = (await invoke("list_servers")) as Server[];
    const user = (await invoke("get_username")) as string;
    // 自包含 app 是本地单用户：无门户邮箱登录/管理员概念。
    return { user, email_login: false, is_admin: false, servers };
  },

  async vaultLock(): Promise<void> {
    await invoke("vault_lock");
  },

  getUsername() {
    return invoke("get_username") as Promise<string>;
  },

  async setUsername(name: string): Promise<void> {
    await invoke("set_username", { name });
  },

  async saveCredential(b: SettingsBody) {
    const has_secret = (await invoke("save_credential", {
      server: b.server,
      username: b.username,
      auth: b.auth,
      secret: b.secret ?? null,
    })) as boolean;
    return { ok: true, has_secret };
  },

  upsertServer(s: ServerInput): Promise<Server[]> {
    return invoke("upsert_server", { server: s }) as Promise<Server[]>;
  },

  delServer(name: string): Promise<Server[]> {
    return invoke("del_server", { name }) as Promise<Server[]>;
  },

  async delCredential(name: string): Promise<void> {
    await invoke("del_credential", { server: name });
  },

  vaultState() {
    return invoke("vault_state") as Promise<{ exists: boolean; unlocked: boolean }>;
  },

  async vaultUnlock(password: string): Promise<void> {
    await invoke("vault_unlock", { password });
  },

  async vaultReset(): Promise<void> {
    await invoke("vault_reset");
  },

  readSshConfig(path?: string) {
    return invoke("read_ssh_config", { path: path ?? null }) as Promise<{
      path: string;
      hosts: import("./index").SshHost[];
    }>;
  },

  importHosts(hosts) {
    return invoke("import_ssh_hosts", { hosts }) as Promise<Server[]>;
  },

  async pickSshConfigFile(): Promise<string | null> {
    const r = await invoke("plugin:dialog|open", {
      options: { multiple: false, directory: false, title: "选择 SSH config 文件" },
    });
    if (typeof r === "string") return r;
    if (r && typeof r === "object" && "path" in r) return (r as { path: string }).path;
    return null;
  },

  loadTeam(path: string) {
    return invoke("load_team", { path }) as Promise<import("./index").LoadTeamResult>;
  },

  compileAcl(path: string) {
    return invoke("compile_acl", { path }) as Promise<import("./index").AclPlan>;
  },

  readTeamFile(path: string) {
    return invoke("read_team_file", { path }) as Promise<import("./index").TeamConfig>;
  },

  async createTeam(path: string, teamName: string, member: string, pubkey: string): Promise<void> {
    await invoke("create_team", { path, teamName, member, pubkey });
  },

  addMember(path: string, name: string, pubkey: string) {
    return invoke("add_member", { path, name, pubkey }) as Promise<import("./index").TeamConfig>;
  },

  shareServer(teamPath: string, server: string, tier: number) {
    return invoke("share_server", { teamPath, server, tier }) as Promise<Server[]>;
  },

  unshareServer(teamPath: string, server: string) {
    return invoke("unshare_server", { teamPath, server }) as Promise<Server[]>;
  },

  myPubkeys() {
    return invoke("my_pubkeys") as Promise<string[]>;
  },

  detectSelf() {
    return invoke("detect_self") as Promise<import("./index").SelfNode>;
  },

  tailnetStatus() {
    return invoke("tailnet_status") as Promise<import("./index").TailnetStatus>;
  },

  async tailnetUp(authkey: string, ingress: boolean): Promise<void> {
    await invoke("tailnet_up", { authkey: authkey || null, ingress });
  },

  async tailnetDown(): Promise<void> {
    await invoke("tailnet_down");
  },

  async pickTeamSavePath(): Promise<string | null> {
    const r = await invoke("plugin:dialog|save", {
      options: {
        title: "新建 team.yaml",
        defaultPath: "team.yaml",
        filters: [{ name: "team config", extensions: ["yaml", "yml"] }],
      },
    });
    return typeof r === "string" ? r : null;
  },

  provisionPreview(teamPath: string, server: string, tier?: number) {
    return invoke("provision_preview", { teamPath, server, tier: tier ?? null }) as Promise<import("./index").ProvisionPlan>;
  },

  provisionApply(teamPath: string, server: string, tier?: number) {
    return invoke("provision_apply", { teamPath, server, tier: tier ?? null }) as Promise<import("./index").ProvisionResult>;
  },

  teamGitStatus(path: string) {
    return invoke("team_git_status", { path }) as Promise<import("./index").GitStatus>;
  },

  teamGitPull(path: string) {
    return invoke("team_git_pull", { path }) as Promise<string>;
  },

  teamGitPush(path: string, message: string) {
    return invoke("team_git_push", { path, message }) as Promise<string>;
  },

  teamGitClone(url: string, dest: string) {
    return invoke("team_git_clone", { url, dest }) as Promise<string>;
  },

  async pickDir(): Promise<string | null> {
    const r = await invoke("plugin:dialog|open", {
      options: { multiple: false, directory: true, title: "选择克隆到哪个目录" },
    });
    if (typeof r === "string") return r;
    if (r && typeof r === "object" && "path" in r) return (r as { path: string }).path;
    return null;
  },

  async pickTeamFile(): Promise<string | null> {
    const r = await invoke("plugin:dialog|open", {
      options: {
        multiple: false,
        directory: false,
        title: "选择 team.yaml",
        filters: [{ name: "team config", extensions: ["yaml", "yml"] }],
      },
    });
    if (typeof r === "string") return r;
    if (r && typeof r === "object" && "path" in r) return (r as { path: string }).path;
    return null;
  },
};
