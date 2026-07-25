// DevSys 桌面核心。
//   阶段 0：脚手架 + SSH 命令桩（打通 transport → 命令 → data/close 事件 → xterm）。
//   阶段 1：本地拓扑（store）+ 凭据（vault，Stronghold 加密保险库）命令。
//   阶段 2：ssh_* 桩替换为 russh 原生会话（直连/ProxyJump/tailnet）。
mod acl;
mod e2e; // 共享闭环的全链路集成测试（#[cfg(test)]）
mod ghauth;
mod github;
mod gitsync;
mod keychain;
mod localpty;
mod provision;
mod selfnode;
mod ssh;
mod tailnet;
mod sshconfig;
mod store;
mod team;
mod vault;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager, State};

// 全局状态：配置目录 + 写锁（拓扑读改写串行化）+ SSH 会话表 + 凭据保险库。
struct AppState {
    dir: PathBuf,
    lock: Mutex<()>,
    sessions: Arc<ssh::Sessions>,
    vault: vault::Vault,
}

// ── 保险库命令 ───────────────────────────────────────────

#[derive(Serialize)]
struct VaultStatus {
    exists: bool,
    unlocked: bool,
    // true = 存在一份旧登录密码建的库、设备密钥开不动，需迁移（vault_migrate）。
    #[serde(default)]
    legacy: bool,
}

#[tauri::command]
fn vault_state(state: State<AppState>) -> VaultStatus {
    VaultStatus {
        exists: state.vault.exists(),
        unlocked: state.vault.is_unlocked(),
        legacy: state.vault.exists() && !state.vault.is_unlocked(),
    }
}

// 异步：Argon2 + Stronghold 建库/解锁较重，放异步运行时线程，避免冻结 UI 主线程。
#[tauri::command]
async fn vault_unlock(state: State<'_, AppState>, password: String) -> Result<(), String> {
    state.vault.unlock(&password)
}

// 用设备密钥自动解锁（无需密码）。app 启动已试过一次，这里供迁移后再解锁。
#[tauri::command]
async fn vault_auto_unlock(state: State<'_, AppState>) -> Result<VaultStatus, String> {
    let _ = state.vault.unlock_device(); // 失败(VAULT_LEGACY)不抛，交由 state 反映
    Ok(VaultStatus {
        exists: state.vault.exists(),
        unlocked: state.vault.is_unlocked(),
        legacy: state.vault.exists() && !state.vault.is_unlocked(),
    })
}

// 迁移旧密码库：销毁旧快照（旧凭据作废，不可逆）后用设备密钥重建。
#[tauri::command]
async fn vault_migrate(state: State<'_, AppState>) -> Result<(), String> {
    state.sessions.close_all();
    state.vault.reset()?;
    {
        let _g = state.lock.lock().unwrap();
        store::clear_all_secrets(&state.dir)?;
    }
    state.vault.unlock_device()
}

// 退出登录：结束所有 SSH 会话 + 锁库（清内存解锁态）。
#[tauri::command]
fn vault_lock(state: State<AppState>) {
    state.sessions.close_all();
    state.vault.lock();
}

// 重置保险库：忘记主密码的唯一出路。销毁快照（凭据全丢，不可逆），
// 同时把 has_secret 标记归零 —— 否则 UI 会以为凭据还在。拓扑保留。
#[tauri::command]
fn vault_reset(state: State<AppState>) -> Result<(), String> {
    state.sessions.close_all();
    state.vault.reset()?;
    let _g = state.lock.lock().unwrap();
    store::clear_all_secrets(&state.dir)?;
    Ok(())
}

// ── GitHub 登录（Device Flow）─────────────────────────────
// 登录 = GitHub 身份（取代本地密码）；登录后按所属 org 自动/手动选团队。

#[derive(Serialize)]
struct GhState {
    logged_in: bool,
    login: String,
    org: String,
    configured: bool, // 是否配了 OAuth App client_id（没配则无法走 device flow）
}

#[tauri::command]
fn gh_state(state: State<AppState>) -> GhState {
    let s = ghauth::session(&state.dir);
    GhState {
        logged_in: ghauth::logged_in(&state.dir),
        login: s.login,
        org: s.org,
        configured: !ghauth::client_id(&state.dir).is_empty(),
    }
}

// 起一次 device flow：返回给用户看的短码 + 授权 URL。前端随后按 interval 轮询。
#[tauri::command]
async fn gh_device_start(state: State<'_, AppState>) -> Result<ghauth::DeviceStart, String> {
    let dir = state.dir.clone();
    tauri::async_runtime::spawn_blocking(move || ghauth::device_start(&dir))
        .await
        .map_err(|e| e.to_string())?
}

// 轮询换 token。ok=登录成功（落 token/会话，返回 login + 所属 orgs）。
#[tauri::command]
async fn gh_device_poll(
    state: State<'_, AppState>,
    device_code: String,
) -> Result<ghauth::PollResult, String> {
    let dir = state.dir.clone();
    tauri::async_runtime::spawn_blocking(move || ghauth::device_poll(&dir, &device_code))
        .await
        .map_err(|e| e.to_string())?
}

// 列出已登录用户所属的 org（切换组织时用）。
#[tauri::command]
async fn gh_orgs(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let dir = state.dir.clone();
    tauri::async_runtime::spawn_blocking(move || ghauth::orgs(&dir))
        .await
        .map_err(|e| e.to_string())?
}

// 选定团队所属组织。
#[tauri::command]
fn gh_set_org(state: State<AppState>, org: String) -> Result<(), String> {
    ghauth::set_org(&state.dir, &org)
}

// 约定:每个 org 的团队配置仓库固定叫这个名(github.com/<org>/ait-team)。
// 里面放 team.yaml(角色 + org 绑定)+ members/*.yaml(各人贡献的机器/档位)。
const TEAM_REPO: &str = "ait-team";

#[derive(Serialize)]
struct ActivateResult {
    path: String,  // team.yaml 路径（空 = 没建成）
    org: String,
    mode: String,  // repo=克隆到约定仓库 / local=仓库缺失，退化为本地草稿（仅成员）
}

// 选 org → 让它「成为团队」的闭环(一个 org = 一个 repo = 一份 team.yaml):
//   · 约定仓库 github.com/<org>/ait-team 存在 → 自动 clone/pull(走用户自己的 git 权限,
//     私有仓库照常);机器共享 = 队友 push 各自 members/*.yaml,你 pull 就看到。
//   · 仓库不存在/无权 → 退化成本地 team.yaml 草稿,只按 org 花名册显示成员(人),
//     并告知需管理员建仓库,机器共享才生效。
//   · 无论哪种,都尽力拉一次 org 花名册(成员/公钥/角色)。
// 返回 team.yaml 路径 + mode,供前端设为当前团队并按需提示。切换 org 复用本命令。
#[tauri::command]
async fn gh_activate_org(state: State<'_, AppState>, org: String) -> Result<ActivateResult, String> {
    let org = org.trim().to_string();
    if org.is_empty() {
        return Err("org 不能为空".into());
    }
    ghauth::set_org(&state.dir, &org)?;

    let team_dir = state.dir.join("teams").join(team::slug_login(&org));
    let dest_str = team_dir.to_string_lossy().to_string();

    // 已 clone 过 → pull 最新;否则试 clone 约定仓库;都不行 → 本地草稿。
    let (path_str, mode) = if team_dir.join(".git").exists() {
        let ty = find_team_yaml(&team_dir).ok_or("本地团队仓库里没有 team.yaml")?;
        let _ = gitsync::pull(&ty); // 网络/冲突失败不致命,用本地已有的
        (ty, "repo")
    } else {
        // 之前退化生成过草稿(无 .git 的目录)会挡住 git clone(拒绝非空目录)——
        // 先挪开草稿再试 clone;clone 失败(仓库还没建)则挪回来继续当草稿。
        // 不挪就永远接不上管理员后来建的真仓库。
        let draft_bak = state.dir.join("teams").join(format!("{}.draft", team::slug_login(&org)));
        let had_draft = team_dir.exists();
        if had_draft {
            let _ = std::fs::remove_dir_all(&draft_bak);
            std::fs::rename(&team_dir, &draft_bak).map_err(|e| e.to_string())?;
        }
        let url = format!("git@github.com:{org}/{TEAM_REPO}.git");
        let cloned = {
            let url = url.clone();
            let dest = dest_str.clone();
            tokio::task::spawn_blocking(move || gitsync::clone(&url, &dest))
                .await
                .map_err(|e| e.to_string())?
        };
        if cloned.is_err() && had_draft {
            let _ = std::fs::remove_dir_all(&team_dir); // 清掉 clone 可能留下的半成品
            let _ = std::fs::rename(&draft_bak, &team_dir); // 还原草稿
        }
        match cloned {
            Ok(ty) => (ty, "repo"),
            Err(_) => {
                // 退化:本地 team.yaml 草稿(绑定 org),只为让成员先显示出来。
                std::fs::create_dir_all(&team_dir).map_err(|e| e.to_string())?;
                let p = team_dir.join("team.yaml");
                let p_str = p.to_string_lossy().to_string();
                if !p.exists() {
                    let mut root = team::new_root(&org);
                    root.github = Some(github::GithubBinding {
                        org: org.clone(),
                        role_map: Default::default(),
                    });
                    write_root(&p_str, &root)?;
                }
                (p_str, "local")
            }
        }
    };

    // 尽力同步花名册(用已存的 gh token + team.yaml 里的 role_map);失败只是暂无成员。
    if let Some(token) = ghauth::token(&state.dir) {
        let role_map = read_root(&path_str)
            .ok()
            .and_then(|r| r.github)
            .map(|g| g.role_map)
            .unwrap_or_default();
        let binding = github::GithubBinding { org: org.clone(), role_map };
        let cache = gh_cache_path(&path_str);
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(members) = github::sync(&binding, Some(&token)) {
                if let Ok(json) = serde_json::to_string_pretty(&members) {
                    let _ = std::fs::write(&cache, json);
                }
            }
        })
        .await;
    }

    // 激活即折进 store(不用再去团队页手点「加载」):织物图立刻可点连,
    // 且顺带清掉上一个团队的残余节点(单活跃团队不变量,见 fold_team_into_store)。
    let _ = fold_team_into_store(&state, &path_str);

    Ok(ActivateResult { path: path_str, org, mode: mode.into() })
}

// 为「当前选中的 org」生成团队仓库初始模板(不写死任何 org —— org 由前端传当前选中的)。
// 生成 team.yaml + members/.gitkeep + README,git init + 初始提交,返回 team.yaml 路径。
// 用户随后建 GitHub 空仓库 <org>/ait-team 并 push(README 里有命令)。
#[tauri::command]
fn gh_init_team(state: State<AppState>, org: String) -> Result<String, String> {
    let org = org.trim().to_string();
    if org.is_empty() {
        return Err("org 不能为空".into());
    }
    let dest = state.dir.join("teams").join(team::slug_login(&org));
    if dest.join(".git").exists() {
        return Err("这个组织已经有团队仓库了，无需再生成模板".into());
    }
    std::fs::create_dir_all(dest.join("members")).map_err(|e| e.to_string())?;

    let team_yaml = TEAM_YAML_TPL.replace("__ORG__", &org);
    std::fs::write(dest.join("team.yaml"), team_yaml).map_err(|e| e.to_string())?;
    std::fs::write(dest.join("members/.gitkeep"), MEMBERS_GITKEEP.replace("__ORG__", &org))
        .map_err(|e| e.to_string())?;
    std::fs::write(dest.join("README.md"), README_TPL.replace("__ORG__", &org))
        .map_err(|e| e.to_string())?;

    let _ = gitsync::init(&dest.to_string_lossy());
    Ok(dest.join("team.yaml").to_string_lossy().to_string())
}

const TEAM_YAML_TPL: &str = "# AIT.dev 团队配置 —— __ORG__ 的团队仓库(github.com/__ORG__/ait-team)。\n# 只放拓扑 / 公钥 / 档位声明，绝不含任何密码或私钥。\nteam: __ORG__\nroles:\n  - core     # 核心成员(通常给档 2:sudo/整机)\n  - member   # 普通 org 成员(默认,通常给档 1:受限计算)\n  - pub      # org 外公开借用(通常档 0:纯跳板)\ngithub:\n  org: __ORG__\n  role_map:\n    # GitHub Team(slug) → 角色;\"*\" = 其余 org 成员默认。\n    # 有 core-team 就取消下一行注释并改成你的 team slug:\n    # core-team: core\n    \"*\": member\n# 团队的 tailnet(可达性地基):填你在 Tailscale 建的 tailnet 名/组织域,\n# 成员据此加入同一张网(能出网的设备直连、内网的经门都要在同一 tailnet)。\n# tailnet: your-org.ts.net\n";

const MEMBERS_GITKEEP: &str = "# 各成员贡献的机器放这个目录,每人一份 members/<你>.yaml,由 AIT.dev app 自动维护。\n# 这个 .gitkeep 只为让 git 保留空目录。示例格式:\n#\n# member:\n#   name: alice\n#   identity: alice@__ORG__\n#   pubkey: ssh-ed25519 AAAA...\n#   role: core\n# machines:\n#   - name: gpu-01\n#     host: 100.x.x.x            # tailnet / 内网地址\n#     grants: { core: 2, member: 1 }        # 角色→档位(0跳板/1受限/2完全信任)\n#     share_limit: { cpus: 4, mem: 16G, gpus: 1 }   # 主人最多借出多少\n#     share_data:                # 可选:主人只读暴露的数据集\n#       - { host: /data/imagenet, as: /datasets/imagenet, mode: ro }\n";

const README_TPL: &str = "# __ORG__ — AIT.dev 团队配置\n\n本仓库是 __ORG__ 的团队配置(AIT.dev)。**只含拓扑、公钥、档位声明,绝不含任何密码 / 私钥。**\n\n- `team.yaml` — 角色定义 + GitHub org 绑定(成员 / 公钥 / 角色从 org 自动导出)。\n- `members/<你>.yaml` — 你贡献的机器 + 每台对哪个角色开哪个档。由 AIT.dev app 自动维护,你只管 push。\n\n## 成员怎么用\n打开 AIT.dev app → 用 GitHub 登录 → 选组织 __ORG__ → app 自动拉取本仓库。\n\n## 首次推送到 GitHub(管理员做一次)\n先在 GitHub 建一个**私有**仓库 `__ORG__/ait-team`,然后:\n\n```\ngit add -A && git commit -m \"init ait-team\"   # 若尚未提交\ngit remote add origin git@github.com:__ORG__/ait-team.git\ngit branch -M main\ngit push -u origin main\n```\n\n或用 gh CLI 一步到位:`gh repo create __ORG__/ait-team --private --source=. --push`\n";

// 目录里找 team.yaml（根优先）。
fn find_team_yaml(dir: &std::path::Path) -> Option<String> {
    for c in ["team.yaml", "team.yml"] {
        let p = dir.join(c);
        if p.exists() {
            return Some(p.to_string_lossy().to_string());
        }
    }
    None
}

// 退出 GitHub 登录（清 token + 会话；保险库不动，仍由设备密钥解锁）。
#[tauri::command]
fn gh_logout(state: State<AppState>) {
    state.sessions.close_all();
    ghauth::logout(&state.dir);
    // 换身份 = 团队上下文作废:清掉 store 里所有团队机,否则织物图残留旧团队节点。
    let _g = state.lock.lock().unwrap();
    let _ = store::remove_team_sources(&state.dir);
}

// 本 OAuth App 的授权管理页 URL：用户在此对某 org 点 Grant(owner)/ Request(成员),
// 解除"第三方 App 访问限制"让该 org 出现在 /user/orgs。我们代劳不了(GitHub 安全设计),
// 只能一键把用户带到正确的页面。
#[tauri::command]
fn gh_authorize_url(state: State<AppState>) -> String {
    let cid = ghauth::client_id(&state.dir);
    if cid.is_empty() {
        return String::new();
    }
    format!("https://github.com/settings/connections/applications/{cid}")
}

// GitHub org 的「新建仓库」页,预填约定名 ait-team + private。用户点一次 Create 即可
// (建仓库需写权限,我们只 read:org → 这一步在网页完成,不扩权限)。
#[tauri::command]
fn gh_new_repo_url(org: String) -> String {
    let org = org.trim();
    if org.is_empty() {
        return String::new();
    }
    format!("https://github.com/organizations/{org}/repositories/new?name=ait-team&visibility=private")
}

// 一键建仓库并推送:先用 GitHub API 建私有仓库 <org>/ait-team(需 repo 权限),
// 再用系统 git 连远程推送(走用户自己的凭据)。token 缺 repo 权限 → 返回 REAUTH: 提示重新授权。
#[tauri::command]
async fn gh_push_team(state: State<'_, AppState>, org: String, path: String) -> Result<String, String> {
    let org = org.trim().to_string();
    if org.is_empty() {
        return Err("org 不能为空".into());
    }
    // 先建仓库（有 token 才试；已存在忽略）。
    if let Some(token) = ghauth::token(&state.dir) {
        let o = org.clone();
        let created = tokio::task::spawn_blocking(move || ghauth::create_repo(&o, &token))
            .await
            .map_err(|e| e.to_string())?;
        if let Err(e) = created {
            if e == "SCOPE" {
                return Err("REAUTH:当前 GitHub 授权没有仓库权限。重新授权一次即可自动建仓库(或用下面「手动在网页建」)。".into());
            }
            return Err(e);
        }
    }
    let url = format!("git@github.com:{org}/ait-team.git");
    tokio::task::spawn_blocking(move || gitsync::set_remote_push(&path, &url))
        .await
        .map_err(|e| e.to_string())?
}

// 用系统默认浏览器打开一个链接（GitHub 授权页）。只放行 http(s)，Command 传参不过 shell，
// 杜绝命令注入 / 本地 scheme 诱导。
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("只允许打开 http(s) 链接".into());
    }
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(target_os = "linux")]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

// 本地用户名（非机密，纯文件）。
#[tauri::command]
fn get_username(state: State<AppState>) -> String {
    std::fs::read_to_string(state.dir.join("profile"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[tauri::command]
fn set_username(state: State<AppState>, name: String) -> Result<(), String> {
    std::fs::create_dir_all(&state.dir).ok();
    std::fs::write(state.dir.join("profile"), name.trim()).map_err(|e| e.to_string())
}

// ── 拓扑命令 ─────────────────────────────────────────────
// 全部只读/写 servers.json，不碰钥匙串（has_secret 用标记位，避免启动弹窗）。

#[tauri::command]
fn list_servers(state: State<AppState>) -> Vec<store::Server> {
    let _g = state.lock.lock().unwrap();
    store::load(&state.dir)
}

#[tauri::command]
fn upsert_server(state: State<AppState>, server: store::Server) -> Result<Vec<store::Server>, String> {
    let _g = state.lock.lock().unwrap();
    store::upsert(&state.dir, server)
}

#[tauri::command]
fn del_server(state: State<AppState>, name: String) -> Result<Vec<store::Server>, String> {
    let _g = state.lock.lock().unwrap();
    state.vault.delete_lenient(&name); // 连带清掉该机凭据（锁定则跳过）
    store::remove(&state.dir, &name)
}

#[derive(Serialize)]
struct SshConfigResult {
    path: String,
    hosts: Vec<sshconfig::SshHost>,
}

// 解析 SSH config。path 为空则用各平台默认 ~/.ssh/config；否则读指定文件。
#[tauri::command]
fn read_ssh_config(app: AppHandle, path: Option<String>) -> Result<SshConfigResult, String> {
    let p = match path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => app
            .path()
            .home_dir()
            .map_err(|e| e.to_string())?
            .join(".ssh")
            .join("config"),
    };
    let text = std::fs::read_to_string(&p).map_err(|_| format!("未找到或无法读取 {}", p.display()))?;
    Ok(SshConfigResult {
        path: p.display().to_string(),
        hosts: sshconfig::parse(&text),
    })
}

fn expand_tilde(path: &str, home: &std::path::Path) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        home.join(rest)
    } else if path == "~" {
        home.to_path_buf()
    } else {
        PathBuf::from(path)
    }
}

#[derive(serde::Deserialize)]
struct ImportHost {
    name: String,
    host: String,
    port: u16,
    username: String,
    jump: Option<String>,
    auth: String,
    identity_file: Option<String>,
}

// 导入所选主机：写拓扑；有 IdentityFile 的读私钥文件直接存进保险库（需已解锁）。
#[tauri::command]
async fn import_ssh_hosts(
    app: AppHandle,
    state: State<'_, AppState>,
    hosts: Vec<ImportHost>,
) -> Result<Vec<store::Server>, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    let _g = state.lock.lock().unwrap();
    let mut list = store::load(&state.dir);
    for h in hosts {
        let mut has_secret = false;
        if let Some(idf) = h.identity_file.as_deref().filter(|s| !s.is_empty()) {
            if let Ok(key) = std::fs::read_to_string(expand_tilde(idf, &home)) {
                state.vault.set(&h.name, key.trim())?; // 锁定 → VAULT_LOCKED
                has_secret = true;
            }
        }
        let transport = if h.jump.is_some() { "jump" } else { "direct" }.to_string();
        let mut srv = store::Server {
            name: h.name,
            host: h.host,
            port: h.port,
            jump: h.jump,
            username: h.username,
            auth: h.auth,
            transport,
            source: "mine".into(), // 从本机 ~/.ssh/config 导入的都是自己的
            shared_to: vec![],
            has_secret,
        };
        if let Some(e) = list.iter_mut().find(|x| x.name == srv.name) {
            srv.has_secret = srv.has_secret || e.has_secret; // 别抹掉已有凭据标记
            *e = srv;
        } else {
            list.push(srv);
        }
    }
    store::save(&state.dir, &list)?;
    Ok(list)
}

// ── 团队命令 ─────────────────────────────────────────────
// 加载 team.yaml:团队共享机作为只读节点(source=team:<名>)合并进列表。
// 共享的是拓扑,不是凭据 —— 队友用自己的凭据连(has_secret 是本机各自的标记)。

// ── 团队目录布局（配置即代码）──────────────────────────
//   <dir>/team.yaml            团队根：角色定义（管理员维护）
//   <dir>/members/<name>.yaml  每人一份：我 + 我贡献的机器（只有本人改，git 无冲突）
// 前端传的 team_path 始终指向 team.yaml；members 目录据此推导。

fn members_dir(team_yaml: &str) -> PathBuf {
    PathBuf::from(team_yaml)
        .parent()
        .map(|p| p.join("members"))
        .unwrap_or_else(|| PathBuf::from("members"))
}
fn member_path(team_yaml: &str, name: &str) -> PathBuf {
    members_dir(team_yaml).join(format!("{name}.yaml"))
}

fn read_root(team_yaml: &str) -> Result<team::TeamRoot, String> {
    let text = std::fs::read_to_string(team_yaml).map_err(|_| format!("未找到或无法读取 {team_yaml}"))?;
    team::parse_root(&text)
}
fn write_root(team_yaml: &str, root: &team::TeamRoot) -> Result<(), String> {
    std::fs::write(team_yaml, team::root_to_yaml(root)?).map_err(|e| e.to_string())
}
fn read_member_file(path: &std::path::Path) -> Result<team::MemberFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    team::parse_member(&text)
}
fn write_member_file(team_yaml: &str, mf: &team::MemberFile) -> Result<(), String> {
    let dir = members_dir(team_yaml);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join(format!("{}.yaml", mf.member.name));
    std::fs::write(p, team::member_to_yaml(mf)?).map_err(|e| e.to_string())
}

// 加载全部成员文件。
fn load_member_files(team_yaml: &str) -> Vec<team::MemberFile> {
    let dir = members_dir(team_yaml);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut out: Vec<team::MemberFile> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "yaml" || x == "yml"))
        .filter_map(|e| read_member_file(&e.path()).ok())
        .collect();
    out.sort_by(|a, b| a.member.name.cmp(&b.member.name));
    out
}

// 加载并合并成统一视图。若 team.yaml 还是旧平表，一次性迁移成新结构再加载。
fn load_view(team_yaml: &str) -> Result<team::TeamView, String> {
    let text = std::fs::read_to_string(team_yaml).map_err(|_| format!("未找到或无法读取 {team_yaml}"))?;

    // 旧平表迁移：members/ 为空且旧格式能解析 → 落地新结构。
    if load_member_files(team_yaml).is_empty() {
        if let Some((root, files)) = team::migrate_flat(&text) {
            // 只有当它确实是"旧平表"（有 machines/members 顶层且非新根）才迁移
            if team::parse_root(&text).map(|r| r.roles.is_empty()).unwrap_or(true)
                || !files.is_empty()
            {
                write_root(team_yaml, &root)?;
                for mf in &files {
                    write_member_file(team_yaml, mf)?;
                }
            }
        }
    }

    let root = read_root(team_yaml)?;
    let mut view = team::merge(&root, &load_member_files(team_yaml));
    // 绑定了 GitHub org → 折叠进缓存的花名册（成员/公钥/角色自动，无需手动登记）。
    // 缓存是本地的（不入 git），由 sync_github 刷新；这里只读，不发网络。
    if root.github.is_some() {
        if let Ok(text) = std::fs::read_to_string(gh_cache_path(team_yaml)) {
            if let Ok(roster) = serde_json::from_str::<Vec<github::GhMember>>(&text) {
                team::fold_github(&mut view, &roster);
            }
        }
    }
    Ok(view)
}

// GitHub 花名册缓存路径（团队目录下的隐藏文件，不入 git）。
fn gh_cache_path(team_yaml: &str) -> PathBuf {
    PathBuf::from(team_yaml)
        .parent()
        .map(|p| p.join(".github-roster.json"))
        .unwrap_or_else(|| PathBuf::from(".github-roster.json"))
}

#[derive(Serialize)]
struct LoadTeamResult {
    team: String,
    added: usize,
    skipped: Vec<String>,
    servers: Vec<store::Server>,
}

#[tauri::command]
fn load_team(path: String, state: State<AppState>) -> Result<LoadTeamResult, String> {
    fold_team_into_store(&state, &path)
}

// 无活跃团队时的兜底清理:清掉 store 里所有 team:* 机器(单活跃团队不变量的第四个执行点 ——
// 激活/加载/登出之外,启动时若既没 org 也没 teamPath,旧团队残余也必须走)。
#[tauri::command]
fn prune_team_sources(state: State<AppState>) -> Result<Vec<store::Server>, String> {
    let _g = state.lock.lock().unwrap();
    store::remove_team_sources(&state.dir)
}

// 把 team.yaml 的机器折进本地 store(团队机作只读节点)。**单活跃团队不变量**:
// 清掉**所有** team:* 旧项再写当前团队 —— 换团队/换 org 不留上一个团队的残余节点
// (织物图/服务器列表都吃 store,残余会一直画在图上)。has_secret 按机器名跨团队保留
// (同一台机换个团队名,你配过的凭据标记还在)。
fn fold_team_into_store(state: &AppState, path: &str) -> Result<LoadTeamResult, String> {
    let view = load_view(path)?;
    let src = format!("team:{}", view.team);

    let _g = state.lock.lock().unwrap();
    let mut list = store::load(&state.dir);

    let prev: std::collections::HashMap<String, bool> = list
        .iter()
        .filter(|s| s.source.starts_with("team:"))
        .map(|s| (s.name.clone(), s.has_secret))
        .collect();
    list.retain(|s| !s.source.starts_with("team:"));

    let mut skipped = Vec::new();
    let mut added = 0usize;
    for m in &view.machines {
        if list.iter().any(|s| s.name == m.name) {
            skipped.push(m.name.clone());
            continue;
        }
        let has_secret = *prev.get(&m.name).unwrap_or(&false);
        list.push(store::Server {
            name: m.name.clone(),
            host: m.host.clone(),
            port: m.port,
            jump: m.jump.clone(),
            username: m.username.clone(),
            auth: "password".into(),
            transport: m.transport.clone(),
            source: src.clone(),
            shared_to: vec![],
            has_secret,
        });
        added += 1;
    }

    store::save(&state.dir, &list)?;
    Ok(LoadTeamResult {
        team: view.team,
        added,
        skipped,
        servers: list,
    })
}

// 读团队合并视图（给贡献 UI 展示成员/机器/角色，也是拓扑图的数据源）。
#[tauri::command]
fn read_team_view(path: String) -> Result<team::TeamView, String> {
    load_view(&path)
}

// 新建团队：生成 team.yaml（角色定义）+ members/<我>.yaml（创建者自己）。
#[tauri::command]
fn create_team(
    tn: State<Arc<tailnet::Tailnet>>,
    state: State<AppState>,
    path: String,
    team_name: String,
    member: String,
    pubkey: String,
    role: Option<String>,
) -> Result<(), String> {
    if team_name.trim().is_empty() {
        return Err("团队名不能为空".into());
    }
    if member.trim().is_empty() {
        return Err("用户名不能为空".into());
    }
    let root = team::new_root(&team_name);
    write_root(&path, &root)?;
    // 创建者默认 core（团队发起人）。盖上验证身份（GitHub 优先,防冒名）。
    let mut mf = team::new_member_file(&member, &pubkey, role.as_deref().unwrap_or("core"));
    if let Some((login, _)) = verified_identity(&state.dir, &tn) {
        mf.member.identity = login;
    }
    write_member_file(&path, &mf)
}

// 探测本机作为节点：地址（内建 tailnet 优先）、sshd 是否在跑、能当算力还是跳板。
// 心智：没有「本地/远程」之分，只有节点 —— 你自己这台机也能贡献给团队。
#[tauri::command]
fn detect_self(tn: State<Arc<tailnet::Tailnet>>) -> selfnode::SelfNode {
    let ip = tn.status().ip;
    selfnode::detect(if ip.is_empty() { None } else { Some(ip) })
}

// 读本机 ~/.ssh/*.pub —— 加入团队时登记自己的公钥（公钥非机密，可以出本机；私钥永不）。
#[tauri::command]
fn my_pubkeys(app: AppHandle) -> Result<Vec<String>, String> {
    let dir = app.path().home_dir().map_err(|e| e.to_string())?.join(".ssh");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Ok(vec![]);
    };
    let mut keys: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "pub"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    keys.sort();
    keys.dedup();
    Ok(keys)
}

// 邀成员 / 登记自己：写 members/<name>.yaml（本人或管理员为其建档）。返回更新后的视图。
#[tauri::command]
fn add_member(
    path: String,
    name: String,
    pubkey: String,
    role: Option<String>,
    identity: Option<String>, // 被邀成员的 SSO 身份（邮箱）—— 声明"谁是这个成员"
) -> Result<team::TeamView, String> {
    // 已有该成员文件则保留其机器，只更新身份字段。
    let mut mf = read_member_file(&member_path(&path, &name))
        .unwrap_or_else(|_| team::new_member_file(&name, &pubkey, role.as_deref().unwrap_or("member")));
    mf.member.name = name.clone();
    mf.member.pubkey = pubkey;
    if let Some(r) = role {
        mf.member.role = r;
    }
    if let Some(id) = identity {
        if !id.trim().is_empty() {
            mf.member.identity = id.trim().into();
        }
    }
    write_member_file(&path, &mf)?;
    load_view(&path)
}

// 统一验证身份锚:**GitHub 登录名(主)→ tailnet SSO(备)**。团队写操作防冒名一律走这里。
// 统一的意义:roster 成员=GitHub login,成员档 name=slug_login(同一 login) —— 锚一致,
// 同一个人才不会在织物图裂成两个节点(双命名空间对齐靠的就是这一条)。
fn verified_identity(dir: &std::path::Path, tn: &tailnet::Tailnet) -> Option<(String, String)> {
    let s = ghauth::session(dir);
    if !s.login.is_empty() && ghauth::token(dir).is_some() {
        return Some((s.login.clone(), s.login));
    }
    tn.status().identity()
}

// 防冒名：我要写的 member 文件，其 identity 必须为空（未认领）或等于我的验证身份。
// 别人已用 SSO 身份认领的文件，我改不了 —— 除非我就是那个身份。
// 真正的门禁在 tailnet（SSO 决定能否真访问）；这一层挡的是本地误操作 / 自觉冒名。
fn guard_member_owner(mf: &team::MemberFile, my_identity: &Option<String>) -> Result<(), String> {
    let claimed = &mf.member.identity;
    if claimed.is_empty() {
        return Ok(()); // 未认领
    }
    match my_identity {
        Some(me) if me == claimed => Ok(()),
        Some(me) => Err(format!(
            "成员档 {} 已由 {} 认领，你（{}）不能修改",
            mf.member.name, claimed, me
        )),
        None => Err(format!(
            "成员档 {} 已由 {} 认领 —— 请先登录 GitHub(或连 tailnet)以验证身份",
            mf.member.name, claimed
        )),
    }
}

// 从 GitHub 花名册缓存里取「我」的第一把公钥与角色(共享时自动登记自己用)。
// 花名册按 GitHub login 存,成员档名是 slug_login(login) —— 两边按 slug 对齐。
fn my_roster_entry(team_yaml: &str, member: &str) -> (String, String) {
    if let Ok(text) = std::fs::read_to_string(gh_cache_path(team_yaml)) {
        if let Ok(roster) = serde_json::from_str::<Vec<github::GhMember>>(&text) {
            if let Some(m) = roster
                .iter()
                .find(|m| team::slug_login(&m.login) == member)
            {
                return (m.pubkeys.first().cloned().unwrap_or_default(), m.role.clone());
            }
        }
    }
    (String::new(), "member".into())
}

// 一台本地机 → member 文件里的机器条目（带 grants + 兑现方式）。
// `prev` = 这台机在 member 文件里已有的条目：advertises（门/subnet router 声明）这类
// **不由共享面板管**的字段必须原样留住，否则「改授权」一次就把它抹了。
fn to_machine(
    s: &store::Server,
    grants: std::collections::BTreeMap<String, u8>,
    sharing: team::Sharing,
    prev: Option<&team::Machine>,
) -> team::Machine {
    team::Machine {
        name: s.name.clone(),
        host: s.host.clone(),
        port: s.port,
        jump: s.jump.clone(),
        username: s.username.clone(),
        transport: s.transport.clone(),
        grants,
        advertises: prev.map(|p| p.advertises.clone()).unwrap_or_default(),
        sharing,
    }
}

// 贡献一台机：写进「我的」member 文件 + 打本地 shared_to 标记（凭据不出本机）。
// grants = 角色→档位（RBAC，「开多少权」）。sharing = 兑现方式（「怎么关」：裸机账号 / 一人一容器
// + 借出上限 + 点名共享的数据集）。两者正交 —— 改隔离方式不动 grants，反之亦然。
// 跳板链自动补全（跳板对所有角色开档 0：只借道不给 shell）。
#[tauri::command]
fn share_server(
    tn: State<Arc<tailnet::Tailnet>>,
    state: State<AppState>,
    team_path: String,
    member: String,
    server: String,
    grants: std::collections::BTreeMap<String, u8>,
    sharing: Option<team::Sharing>,
) -> Result<Vec<store::Server>, String> {
    let _g = state.lock.lock().unwrap();
    let list = store::load(&state.dir);
    let srv = list.iter().find(|s| s.name == server).ok_or("找不到该服务器")?;
    if srv.source != "mine" {
        return Err("只能贡献自己的机器（团队给的机器不可再贡献）".into());
    }
    let view = load_view(&team_path)?;
    let team_src = format!("team:{}", view.team);

    // 我的 member 文件（贡献写进这里 —— 各人各文件，git 无冲突）。
    // 不存在就**自动登记我自己**(共享即加入团队):公钥优先取 GitHub 花名册里我的,
    // 否则用本机 ~/.ssh/*.pub。否则用户会撞上「先加入团队」这个死胡同。
    let mpath = member_path(&team_path, &member);
    let mut mf = match read_member_file(&mpath) {
        Ok(f) => f,
        Err(_) => {
            let (pubkey, role) = my_roster_entry(&team_path, &member);
            team::new_member_file(&member, &pubkey, &role)
        }
    };
    // 防冒名：只能写自己认领的成员档。有验证身份(GitHub 优先)则顺带盖上。
    let my_id = verified_identity(&state.dir, &tn).map(|(l, _)| l);
    guard_member_owner(&mf, &my_id)?;
    if let Some(id) = &my_id {
        if mf.member.identity.is_empty() {
            mf.member.identity = id.clone();
        }
    }

    // 跳板链补全（逐级向上）。
    let mut chain: Vec<store::Server> = Vec::new();
    let mut hop = srv.jump.clone();
    let mut guard = 0;
    while let Some(jn) = hop {
        guard += 1;
        if guard > 8 {
            return Err("跳板链过长（可能成环）".into());
        }
        let j = list
            .iter()
            .find(|s| s.name == jn)
            .ok_or_else(|| format!("跳板 {jn} 不在你的服务器列表里 —— 队友将无法经它到达"))?;
        if j.source != "mine" {
            return Err(format!("跳板 {jn} 不是你的机器，无法一并共享"));
        }
        hop = j.jump.clone();
        chain.push(j.clone());
    }
    // 跳板：对每个角色开档 0（列进配置但不给 shell），已存在则不改。
    let jump_grants: std::collections::BTreeMap<String, u8> =
        view.roles.iter().map(|r| (r.clone(), 0u8)).collect();
    for j in &chain {
        if !mf.machines.iter().any(|m| m.name == j.name) {
            // 跳板只是门：不给容器（isolation 保持默认），也没有借出上限可言。
            team::upsert_machine(&mut mf, to_machine(j, jump_grants.clone(), Default::default(), None));
        }
    }

    // 「改授权」时保留已有条目里共享面板管不到的字段（advertises 等）。
    let prev = mf.machines.iter().find(|m| m.name == srv.name).cloned();
    let sharing = sharing.unwrap_or_else(|| prev.as_ref().map(|p| p.sharing.clone()).unwrap_or_default());
    let m = to_machine(srv, grants, sharing, prev.as_ref());
    team::upsert_machine(&mut mf, m);
    write_member_file(&team_path, &mf)?;

    let mut out = store::set_shared(&state.dir, &server, &team_src, true)?;
    for j in &chain {
        out = store::set_shared(&state.dir, &j.name, &team_src, true)?;
    }
    Ok(out)
}

// 撤销贡献：从「我的」member 文件删该机 + 去本地标记。
// 不允许撤掉仍被（任何成员的）共享机当跳板的机器 —— 会把队友的跳板链弄断。
#[tauri::command]
fn unshare_server(
    tn: State<Arc<tailnet::Tailnet>>,
    state: State<AppState>,
    team_path: String,
    member: String,
    server: String,
) -> Result<Vec<store::Server>, String> {
    let _g = state.lock.lock().unwrap();
    let view = load_view(&team_path)?;

    let dependents: Vec<&str> = view
        .machines
        .iter()
        .filter(|m| m.name != server && m.jump.as_deref() == Some(server.as_str()))
        .map(|m| m.name.as_str())
        .collect();
    if !dependents.is_empty() {
        return Err(format!(
            "{server} 仍是这些共享机的跳板：{} —— 先撤销它们，否则队友会连不上",
            dependents.join("、")
        ));
    }

    let mut mf = read_member_file(&member_path(&team_path, &member))
        .map_err(|_| format!("找不到你的成员档 members/{member}.yaml"))?;
    guard_member_owner(&mf, &verified_identity(&state.dir, &tn).map(|(l, _)| l))?;
    team::remove_machine(&mut mf, &server);
    write_member_file(&team_path, &mf)?;
    store::set_shared(&state.dir, &server, &format!("team:{}", view.team), false)
}

// 把 RBAC（角色×grant）编译成 Tailscale ACL 计划。只产出计划供人 review —— 责任为门。
#[tauri::command]
fn compile_acl(path: String) -> Result<acl::AclPlan, String> {
    Ok(acl::compile(&load_view(&path)?))
}

// ── GitHub org 花名册（成员/公钥/角色自动导出，消掉手动登记）──

// 绑定 GitHub org + 角色映射，写进 team.yaml（团队级声明）。
#[tauri::command]
fn bind_github(
    path: String,
    org: String,
    role_map: std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    if org.trim().is_empty() {
        return Err("org 不能为空".into());
    }
    let mut root = read_root(&path)?;
    root.github = Some(github::GithubBinding { org: org.trim().into(), role_map });
    write_root(&path, &root)
}

#[derive(Serialize)]
struct SyncGithubResult {
    count: usize,
    with_keys: usize,
    members: Vec<github::GhMember>,
}

// 同步 GitHub 花名册：拉 org 成员 + 公钥 + 角色，缓存到本地（不入 git）。async。
#[tauri::command]
async fn sync_github(state: State<'_, AppState>, path: String, token: Option<String>) -> Result<SyncGithubResult, String> {
    let root = read_root(&path)?;
    let binding = root.github.ok_or("这个团队还没绑定 GitHub org")?;
    let cache = gh_cache_path(&path);
    // token 未显式给 → 用登录时存进钥匙串的(UI 不必再要用户贴 token)。
    let token = token.filter(|t| !t.trim().is_empty()).or_else(|| ghauth::token(&state.dir));
    let members = tokio::task::spawn_blocking(move || github::sync(&binding, token.as_deref()))
        .await
        .map_err(|e| e.to_string())??;

    let with_keys = members.iter().filter(|m| !m.pubkeys.is_empty()).count();
    let json = serde_json::to_string_pretty(&members).map_err(|e| e.to_string())?;
    std::fs::write(&cache, json).map_err(|e| e.to_string())?;

    Ok(SyncGithubResult { count: members.len(), with_keys, members })
}

// ── team.yaml 的 git 同步（配置即代码：团队配置放 git，天然有历史与 review）──
// 网络操作放 async（spawn_blocking），避免冻结 UI。

#[tauri::command]
fn team_git_status(path: String) -> Result<gitsync::GitStatus, String> {
    gitsync::status(&path)
}

#[tauri::command]
async fn team_git_pull(path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || gitsync::pull(&path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn team_git_push(path: String, message: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || gitsync::push(&path, &message))
        .await
        .map_err(|e| e.to_string())?
}

// 克隆团队仓库，返回其中 team.yaml 的路径（加入团队最顺的入口）。
#[tauri::command]
async fn team_git_clone(url: String, dest: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || gitsync::clone(&url, &dest))
        .await
        .map_err(|e| e.to_string())?
}

// ── 授权下发（让队友真能登进去）──────────────────────────
// 两步走：先 preview（纯生成，看得见要干什么），确认后 apply（经 SSH 以 root 执行）。
// 我们**不静默改别人的机器** —— 责任为门：机器主人先看脚本，再点执行。

// 生成下发脚本（不连接、不执行）。每个成员的档位按其角色 × 该机 grant 算出。
#[tauri::command]
fn provision_preview(team_path: String, server: String) -> Result<provision::ProvisionPlan, String> {
    provision::plan(&load_view(&team_path)?, &server)
}

#[derive(Serialize)]
struct ProvisionResult {
    ok: bool,
    code: u32,
    output: String,
}

// 真执行：经 SSH（含 ProxyJump）在被共享机上以 root 跑下发脚本。
// 要求这台机的凭据本身有 root/sudo 权（你是机器主人，本该有）。
#[tauri::command]
async fn provision_apply(
    state: State<'_, AppState>,
    tn: State<'_, Arc<tailnet::Tailnet>>,
    team_path: String,
    server: String,
) -> Result<ProvisionResult, String> {
    let plan = provision_preview(team_path, server.clone())?;
    if plan.accounts.is_empty() {
        return Ok(ProvisionResult {
            ok: true,
            code: 0,
            output: "没有需要下发的账号（无成员获此机 shell 授权，或成员缺公钥）。".into(),
        });
    }

    let (target, secret, jumps) = resolve_conn(&state, &server)?;
    // 脚本经 stdin 喂给 sh，避免超长命令行；sudo -n 需免密，否则请用 root 凭据。
    let cmd = format!(
        "sudo -n sh -s <<'DEVSYS_EOF'\n{}\nDEVSYS_EOF\n",
        plan.script
    );
    let out = ssh::exec(target, secret, jumps, cmd, tn.socks_addr()).await?;
    Ok(ProvisionResult {
        ok: out.code == 0,
        code: out.code,
        output: out.output,
    })
}

// ── 内建 tailnet（tsnet sidecar）─────────────────────────
// 出站 SOCKS5 供 russh 走 tailnet；入站把 :22 代理到本机 sshd（贡献侧）。
// 零系统依赖：helper 是随 app 分发的 tsnet 二进制。

// 解析 helper 二进制路径：开发期用仓库里编好的，发布期在资源目录。
fn helper_path(app: &AppHandle) -> Result<String, String> {
    let name = "tsnet-helper";
    // 发布：资源目录
    if let Ok(res) = app.path().resource_dir() {
        let p = res.join(name);
        if p.exists() {
            return Ok(p.to_string_lossy().to_string());
        }
    }
    // 开发：仓库根 target 约定位置
    for cand in [
        "../tsnet-helper/tsnet-helper",
        "tsnet-helper/tsnet-helper",
    ] {
        let p = std::path::Path::new(cand);
        if p.exists() {
            return Ok(p.to_string_lossy().to_string());
        }
    }
    Err("找不到 tsnet-helper 二进制（先构建 sidecar）".into())
}

#[tauri::command]
fn tailnet_status(tn: State<Arc<tailnet::Tailnet>>) -> tailnet::TailnetStatus {
    tn.status()
}

#[tauri::command]
fn tailnet_up(
    app: AppHandle,
    tn: State<Arc<tailnet::Tailnet>>,
    state: State<AppState>,
    authkey: Option<String>,
    ingress: bool,
    control: Option<String>,
) -> Result<(), String> {
    let helper = helper_path(&app)?;
    let control = control.unwrap_or_default();
    // 已在跑(比如启动时自动连了官方网)→ 先停,否则切换控制面会报「已在运行」。
    if tn.is_running() {
        tn.stop();
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    let dir = tsnet_dir(&state.dir, &control);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let host = std::fs::read_to_string(state.dir.join("profile"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "devsys".into());
    tn.start(
        app.clone(),
        &helper,
        &dir.to_string_lossy(),
        &host,
        authkey.as_deref().unwrap_or(""),
        ingress,
        &control,
    )?;
    // 记下自启意向 + 控制面(下次静默重连要连回同一个 Headscale/官方网)。
    let _ = std::fs::write(
        state.dir.join("tailnet.json"),
        serde_json::json!({ "ingress": ingress, "control": control }).to_string(),
    );
    Ok(())
}

#[tauri::command]
fn tailnet_down(tn: State<Arc<tailnet::Tailnet>>, state: State<AppState>) {
    tn.stop();
    // 显式断开 = 清掉自启意向；下次进 app 不再自动重连。
    let _ = std::fs::remove_file(state.dir.join("tailnet.json"));
}

// tsnet 状态目录**按控制面分开**:节点身份是与某个控制面绑定的,官方 Tailscale 与
// 自建 Headscale 混用同一目录会导致注册态互相污染(表现为死活连不上)。
fn tsnet_dir(base: &std::path::Path, control: &str) -> PathBuf {
    let c = control.trim();
    if c.is_empty() {
        return base.join("tsnet"); // 官方 Tailscale
    }
    let host: String = c
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("control")
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '.' { ch } else { '-' })
        .collect();
    base.join(format!("tsnet-{host}"))
}

// 上次连过（tailnet.json 存在）就在启动时静默重连，兑现「登录一次就保留、别反复」。
// tsnet 的节点身份持久化在 dir/tsnet，空 authkey 即可复用，无需再走浏览器登录。
fn tailnet_autostart(app: AppHandle, tn: Arc<tailnet::Tailnet>, dir: PathBuf) {
    let Ok(raw) = std::fs::read_to_string(dir.join("tailnet.json")) else { return };
    let v = serde_json::from_str::<serde_json::Value>(&raw).unwrap_or(serde_json::Value::Null);
    let ingress = v.get("ingress").and_then(|b| b.as_bool()).unwrap_or(false);
    let control = v.get("control").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let Ok(helper) = helper_path(&app) else { return };
    let tsdir = tsnet_dir(&dir, &control);
    if std::fs::create_dir_all(&tsdir).is_err() {
        return;
    }
    let host = std::fs::read_to_string(dir.join("profile"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "devsys".into());
    let _ = tn.start(app, &helper, &tsdir.to_string_lossy(), &host, "", ingress, &control);
}

#[derive(Serialize)]
struct Identity {
    login: String,   // 验证过的 SSO 登录名（空=未连 tailnet / 未登录）
    display: String,
    name: String,    // 派生的 unix 账号句柄
}

// 当前验证过的团队身份(GitHub 登录优先,tailnet SSO 备选)。团队写操作据此防冒名。
// 命令名保留 tailnet_identity(前端已布线),语义已升级为「统一验证身份」。
#[tauri::command]
fn tailnet_identity(tn: State<Arc<tailnet::Tailnet>>, state: State<AppState>) -> Identity {
    match verified_identity(&state.dir, &tn) {
        Some((login, display)) => Identity {
            name: team::slug_login(&login),
            login,
            display,
        },
        None => Identity { login: String::new(), display: String::new(), name: String::new() },
    }
}

// 「我是谁」的**显示用**身份（只用于图上标「我」等场景，非防冒名的严肃链）。
// 兜底顺序：GitHub 登录 → 内建 tsnet SSO → 系统 Tailscale 身份 → OS 用户名。
#[tauri::command]
fn local_identity(tn: State<Arc<tailnet::Tailnet>>, state: State<AppState>) -> Identity {
    if let Some((login, display)) = verified_identity(&state.dir, &tn) {
        return Identity { name: team::slug_login(&login), login, display };
    }
    if let Some((login, display)) = system_tailscale_identity() {
        return Identity { name: team::slug_login(&login), login, display };
    }
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
    Identity { name: team::slug_login(&user), login: user.clone(), display: user }
}

// 读系统 Tailscale 的登录身份（若装了且已连）。纯显示兜底，失败静默返回 None。
fn system_tailscale_identity() -> Option<(String, String)> {
    let bins = ["tailscale", "/Applications/Tailscale.app/Contents/MacOS/Tailscale"];
    for bin in bins {
        let out = std::process::Command::new(bin).args(["status", "--json"]).output().ok()?;
        if !out.status.success() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
        let uid = v.get("Self")?.get("UserID")?.to_string();
        let u = v.get("User")?.get(uid.trim_matches('"'))?;
        let login = u.get("LoginName")?.as_str()?.to_string();
        let display = u.get("DisplayName").and_then(|d| d.as_str()).unwrap_or(&login).to_string();
        return Some((login, display));
    }
    None
}

// ── 凭据命令 ─────────────────────────────────────────────
// 更新该机 username/auth（拓扑）+ 存密钥（keychain）+ 置 has_secret 标记位。

// 异步：写保险库涉及 Stronghold 提交（加密落盘），放异步线程避免冻结 UI。
#[tauri::command]
async fn save_credential(
    state: State<'_, AppState>,
    server: String,
    username: String,
    auth: String,
    secret: Option<String>,
) -> Result<bool, String> {
    let _g = state.lock.lock().unwrap();
    let sec = secret.filter(|s| !s.is_empty());
    let mut list = store::load(&state.dir);
    let idx = list
        .iter()
        .position(|x| x.name == server)
        .ok_or_else(|| format!("服务器 {server} 不存在"))?;
    // 先写保险库（锁定则返回 VAULT_LOCKED，前端弹解锁），成功后再更新拓扑标记。
    if let Some(v) = &sec {
        state.vault.set(&server, v)?;
    }
    let s = &mut list[idx];
    s.username = username;
    s.auth = auth;
    if sec.is_some() {
        s.has_secret = true;
    }
    let now = s.has_secret;
    store::save(&state.dir, &list)?;
    Ok(now)
}

#[tauri::command]
fn del_credential(state: State<AppState>, server: String) -> Result<(), String> {
    let _g = state.lock.lock().unwrap();
    state.vault.delete_lenient(&server);
    let mut list = store::load(&state.dir);
    if let Some(s) = list.iter_mut().find(|x| x.name == server) {
        s.has_secret = false;
        store::save(&state.dir, &list)?;
    }
    Ok(())
}

// ── SSH 会话（russh 原生：直连 / ProxyJump）───────────────

// 解析一台机的连接材料：拓扑 + 凭据（含**多跳跳板链**）。锁不跨 await。
// 跳板是「指向另一台机的名字」，多跳 = 链式引用：target.jump→login、login.jump→edge。
// 这里跟着引用把链走全（内→外），再反转成「外→内」（先连的在前）交给 ssh 层。
type ConnParts = (store::Server, String, Vec<(store::Server, String)>);
fn resolve_conn(state: &State<'_, AppState>, server: &str) -> Result<ConnParts, String> {
    let (target, chain) = {
        let _g = state.lock.lock().unwrap();
        let list = store::load(&state.dir);
        let target = list
            .iter()
            .find(|s| s.name == server)
            .cloned()
            .ok_or_else(|| format!("服务器 {server} 不存在"))?;
        // 跟着 jump 引用把跳板链走全（内→外）。纯逻辑在 store::jump_chain（含防环/防悬空）。
        let chain = store::jump_chain(&list, &target)?;
        (target, chain)
    };

    let target_secret = state.vault.get(server)?;
    // 反转成「外→内」（最外层先连），并取每跳凭据。
    let mut jumps = Vec::new();
    for j in chain.into_iter().rev() {
        let js = state.vault.get(&j.name)?;
        jumps.push((j, js));
    }
    Ok((target, target_secret, jumps))
}

#[tauri::command]
async fn ssh_open(
    app: AppHandle,
    state: State<'_, AppState>,
    tn: State<'_, Arc<tailnet::Tailnet>>,
    server: String,
    ws: Option<String>,
) -> Result<String, String> {
    // 本机也是节点:保留名 ~local 直接开本地 PTY(不查 store/保险库、不走 SSH)。
    if server == localpty::LOCAL_NODE {
        return localpty::open(app, state.sessions.clone(), ws);
    }
    let (target, target_secret, jumps) = resolve_conn(&state, &server)?;
    // 内嵌 tsnet 在跑 → 把它的 SOCKS 递给 SSH 层(tailnet 入口经它走;其余照旧直连)。
    ssh::open(app, state.sessions.clone(), target, target_secret, jumps, ws, tn.socks_addr()).await
}

#[tauri::command]
fn ssh_write(state: State<AppState>, id: String, data: String) {
    state.sessions.send(&id, ssh::SessionCmd::Data(data.into_bytes()));
}

#[tauri::command]
fn ssh_resize(state: State<AppState>, id: String, cols: u32, rows: u32) {
    state.sessions.send(&id, ssh::SessionCmd::Resize(cols, rows));
}

#[tauri::command]
fn ssh_close(state: State<AppState>, id: String) {
    state.sessions.send(&id, ssh::SessionCmd::Close);
}

// 当前有活 SSH 会话的服务器名（织物图「已连接·常亮」的数据源;之后靠 ssh://active 事件推送）。
#[tauri::command]
fn ssh_active(state: State<AppState>) -> Vec<String> {
    state.sessions.active()
}

// 可达性探测:对 store 每台机,TCP 摸它的**入口**(有跳板链则摸链最外层的门,直连则摸它本身)。
// 只测「拨得通拨不通」,不做 SSH 握手 —— 轻、快、并行,织物图据此把节点画成 可达(脉冲)/不可达(灰)。
// 与拨号同一规则:tailnet 入口 + 内嵌 tsnet 在跑 → 经它的 SOCKS 摸;否则裸 TCP。
#[tauri::command]
async fn probe_reach(
    state: State<'_, AppState>,
    tn: State<'_, Arc<tailnet::Tailnet>>,
) -> Result<std::collections::HashMap<String, bool>, String> {
    let socks = tn.socks_addr();
    // 快照 + 每台解析入口(host, port, transport),不持锁跨 await。
    let targets: Vec<(String, String, u16, String)> = {
        let _g = state.lock.lock().unwrap();
        let list = store::load(&state.dir);
        list.iter()
            .map(|s| {
                let entry = store::jump_chain(&list, s)
                    .ok()
                    .and_then(|c| c.last().cloned())
                    .unwrap_or_else(|| s.clone());
                (s.name.clone(), entry.host, entry.port, entry.transport)
            })
            .collect()
    };
    let mut tasks = Vec::new();
    for (name, host, port, transport) in targets {
        let socks = socks.clone();
        tasks.push(tokio::spawn(async move {
            let dial = async {
                if transport == "tailnet" && socks.is_some() {
                    tokio_socks::tcp::Socks5Stream::connect(socks.as_deref().unwrap(), (host.as_str(), port))
                        .await
                        .is_ok()
                } else {
                    tokio::net::TcpStream::connect((host.as_str(), port)).await.is_ok()
                }
            };
            let ok = tokio::time::timeout(std::time::Duration::from_millis(1500), dial)
                .await
                .unwrap_or(false);
            (name, ok)
        }));
    }
    let mut out = std::collections::HashMap::new();
    for t in tasks {
        if let Ok((name, ok)) = t.await {
            out.insert(name, ok);
        }
    }
    Ok(out)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app
                .path()
                .app_config_dir()
                .map_err(|e| format!("无法定位配置目录: {e}"))?;
            // 保险库用设备密钥自动解锁（登录已改走 GitHub，本地不再问密码）。
            // 首次会随机生成钥匙存进 OS 钥匙串；旧密码库解不动 → 留待 UI 迁移，不致命。
            let vault = vault::Vault::new(dir.clone());
            let _ = vault.unlock_device();
            app.manage(AppState {
                dir: dir.clone(),
                lock: Mutex::new(()),
                sessions: Arc::new(ssh::Sessions::new()),
                vault,
            });
            let tn = Arc::new(tailnet::Tailnet::new());
            app.manage(tn.clone());
            // 上次连过 tailnet 就静默重连（不必每次进 app 重新登录）。
            tailnet_autostart(app.handle().clone(), tn, dir);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_state,
            vault_unlock,
            vault_auto_unlock,
            vault_migrate,
            vault_lock,
            vault_reset,
            gh_state,
            gh_device_start,
            gh_device_poll,
            gh_orgs,
            gh_set_org,
            gh_activate_org,
            gh_init_team,
            gh_logout,
            gh_authorize_url,
            gh_new_repo_url,
            gh_push_team,
            open_url,
            get_username,
            set_username,
            list_servers,
            upsert_server,
            del_server,
            read_ssh_config,
            import_ssh_hosts,
            load_team,
            prune_team_sources,
            read_team_view,
            create_team,
            add_member,
            my_pubkeys,
            detect_self,
            share_server,
            unshare_server,
            compile_acl,
            bind_github,
            sync_github,
            team_git_status,
            team_git_pull,
            team_git_push,
            team_git_clone,
            provision_preview,
            provision_apply,
            tailnet_status,
            tailnet_up,
            tailnet_down,
            tailnet_identity,
            local_identity,
            save_credential,
            del_credential,
            ssh_open,
            ssh_write,
            ssh_resize,
            ssh_close,
            ssh_active,
            probe_reach,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
