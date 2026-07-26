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
    return invoke("vault_state") as Promise<import("./index").VaultState>;
  },

  async vaultUnlock(password: string): Promise<void> {
    await invoke("vault_unlock", { password });
  },

  vaultAutoUnlock() {
    return invoke("vault_auto_unlock") as Promise<import("./index").VaultState>;
  },

  async vaultMigrate(): Promise<void> {
    await invoke("vault_migrate");
  },

  async vaultReset(): Promise<void> {
    await invoke("vault_reset");
  },

  ghState() {
    return invoke("gh_state") as Promise<import("./index").GhState>;
  },

  ghDeviceStart() {
    return invoke("gh_device_start") as Promise<import("./index").GhDeviceStart>;
  },

  ghDevicePoll(deviceCode: string) {
    return invoke("gh_device_poll", { deviceCode }) as Promise<import("./index").GhPoll>;
  },

  ghOrgs() {
    return invoke("gh_orgs") as Promise<string[]>;
  },

  async ghSetOrg(org: string): Promise<void> {
    await invoke("gh_set_org", { org });
  },

  ghActivateOrg(org: string) {
    return invoke("gh_activate_org", { org }) as Promise<import("./index").GhActivateResult>;
  },

  ghInitTeam(org: string) {
    return invoke("gh_init_team", { org }) as Promise<string>;
  },

  async ghLogout(): Promise<void> {
    await invoke("gh_logout");
  },

  ghAuthorizeUrl() {
    return invoke("gh_authorize_url") as Promise<string>;
  },

  ghNewRepoUrl(org: string) {
    return invoke("gh_new_repo_url", { org }) as Promise<string>;
  },

  ghPushTeam(org: string, path: string) {
    return invoke("gh_push_team", { org, path }) as Promise<string>;
  },

  async openUrl(url: string): Promise<void> {
    await invoke("open_url", { url });
  },

  sshActive() {
    return invoke("ssh_active") as Promise<string[]>;
  },

  probeReach() {
    return invoke("probe_reach") as Promise<Record<string, boolean>>;
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

  async pruneTeamSources(): Promise<void> {
    await invoke("prune_team_sources");
  },

  compileAcl(path: string) {
    return invoke("compile_acl", { path }) as Promise<import("./index").AclPlan>;
  },

  async bindGithub(path: string, org: string, roleMap: Record<string, string>): Promise<void> {
    await invoke("bind_github", { path, org, roleMap });
  },

  syncGithub(path: string, token?: string) {
    return invoke("sync_github", { path, token: token || null }) as Promise<{ count: number; with_keys: number; members: { login: string; pubkeys: string[]; role: string }[] }>;
  },

  readTeamView(path: string) {
    return invoke("read_team_view", { path }) as Promise<import("./index").TeamView>;
  },

  async createTeam(path: string, teamName: string, member: string, pubkey: string, role?: string): Promise<void> {
    await invoke("create_team", { path, teamName, member, pubkey, role: role ?? null });
  },

  addMember(path: string, name: string, pubkey: string, role?: string, identity?: string) {
    return invoke("add_member", { path, name, pubkey, role: role ?? null, identity: identity ?? null }) as Promise<import("./index").TeamView>;
  },

  shareServer(
    teamPath: string,
    member: string,
    server: string,
    grants: Record<string, number>,
    sharing?: import("./index").Sharing | null,
  ) {
    return invoke("share_server", { teamPath, member, server, grants, sharing: sharing ?? null }) as Promise<Server[]>;
  },

  unshareServer(teamPath: string, member: string, server: string) {
    return invoke("unshare_server", { teamPath, member, server }) as Promise<Server[]>;
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

  tailnetLog() {
    return invoke("tailnet_log") as Promise<string[]>;
  },

  async tailnetUp(authkey: string, ingress: boolean, control?: string): Promise<void> {
    await invoke("tailnet_up", { authkey: authkey || null, ingress, control: control || null });
  },

  async tailnetDown(): Promise<void> {
    await invoke("tailnet_down");
  },

  tailnetIdentity() {
    return invoke("tailnet_identity") as Promise<{ login: string; display: string; name: string }>;
  },

  localIdentity() {
    return invoke("local_identity") as Promise<{ login: string; display: string; name: string }>;
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

  probeHost(server: string) {
    return invoke("probe_host", { server }) as Promise<import("./index").HostProbe>;
  },

  provisionPreview(teamPath: string, server: string) {
    return invoke("provision_preview", { teamPath, server }) as Promise<import("./index").ProvisionPlan>;
  },

  provisionApply(teamPath: string, server: string) {
    return invoke("provision_apply", { teamPath, server }) as Promise<import("./index").ProvisionResult>;
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
