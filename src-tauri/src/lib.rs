// DevSys 桌面核心。
//   阶段 0：脚手架 + SSH 命令桩（打通 transport → 命令 → data/close 事件 → xterm）。
//   阶段 1：本地拓扑（store）+ 凭据（vault，Stronghold 加密保险库）命令。
//   阶段 2：ssh_* 桩替换为 russh 原生会话（直连/ProxyJump/tailnet）。
mod acl;
mod e2e; // 共享闭环的全链路集成测试（#[cfg(test)]）
mod github;
mod gitsync;
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
}

#[tauri::command]
fn vault_state(state: State<AppState>) -> VaultStatus {
    VaultStatus {
        exists: state.vault.exists(),
        unlocked: state.vault.is_unlocked(),
    }
}

// 异步：Argon2 + Stronghold 建库/解锁较重，放异步运行时线程，避免冻结 UI 主线程。
#[tauri::command]
async fn vault_unlock(state: State<'_, AppState>, password: String) -> Result<(), String> {
    state.vault.unlock(&password)
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

// 本地用户名（非机密，纯文件；登录密码即保险库主密码，不单独存）。
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
    let view = load_view(&path)?;
    let src = format!("team:{}", view.team);

    let _g = state.lock.lock().unwrap();
    let mut list = store::load(&state.dir);

    let prev: std::collections::HashMap<String, bool> = list
        .iter()
        .filter(|s| s.source == src)
        .map(|s| (s.name.clone(), s.has_secret))
        .collect();
    list.retain(|s| s.source != src);

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
    // 创建者默认 core（团队发起人）。若已连 tailnet，盖上验证过的身份（防冒名）。
    let mut mf = team::new_member_file(&member, &pubkey, role.as_deref().unwrap_or("core"));
    if let Some((login, _)) = tn.status().identity() {
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
            "成员档 {} 已由 {} 认领 —— 请先连 tailnet 以验证身份",
            mf.member.name, claimed
        )),
    }
}

// 一台本地机 → member 文件里的机器条目（带 grants）。
fn to_machine(s: &store::Server, grants: std::collections::BTreeMap<String, u8>) -> team::Machine {
    team::Machine {
        name: s.name.clone(),
        host: s.host.clone(),
        port: s.port,
        jump: s.jump.clone(),
        username: s.username.clone(),
        transport: s.transport.clone(),
        grants,
    }
}

// 贡献一台机：写进「我的」member 文件 + 打本地 shared_to 标记（凭据不出本机）。
// grants = 角色→档位（RBAC）。跳板链自动补全（跳板对所有角色开档 0：只借道不给 shell）。
#[tauri::command]
fn share_server(
    tn: State<Arc<tailnet::Tailnet>>,
    state: State<AppState>,
    team_path: String,
    member: String,
    server: String,
    grants: std::collections::BTreeMap<String, u8>,
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
    let mut mf = read_member_file(&member_path(&team_path, &member))
        .map_err(|_| format!("找不到你的成员档 members/{member}.yaml —— 先加入团队"))?;
    // 防冒名：只能写自己认领的成员档。已连 tailnet 则顺带盖上验证身份。
    let my_id = tn.status().identity().map(|(l, _)| l);
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
            team::upsert_machine(&mut mf, to_machine(j, jump_grants.clone()));
        }
    }

    team::upsert_machine(&mut mf, to_machine(srv, grants));
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
    guard_member_owner(&mf, &tn.status().identity().map(|(l, _)| l))?;
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
async fn sync_github(path: String, token: Option<String>) -> Result<SyncGithubResult, String> {
    let root = read_root(&path)?;
    let binding = root.github.ok_or("这个团队还没绑定 GitHub org")?;
    let cache = gh_cache_path(&path);
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

    let (target, secret, jump) = resolve_conn(&state, &server)?;
    // 脚本经 stdin 喂给 sh，避免超长命令行；sudo -n 需免密，否则请用 root 凭据。
    let cmd = format!(
        "sudo -n sh -s <<'DEVSYS_EOF'\n{}\nDEVSYS_EOF\n",
        plan.script
    );
    let out = ssh::exec(target, secret, jump, cmd).await?;
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
) -> Result<(), String> {
    let helper = helper_path(&app)?;
    let dir = state.dir.join("tsnet");
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
    )
}

#[tauri::command]
fn tailnet_down(tn: State<Arc<tailnet::Tailnet>>) {
    tn.stop();
}

#[derive(Serialize)]
struct Identity {
    login: String,   // 验证过的 SSO 登录名（空=未连 tailnet / 未登录）
    display: String,
    name: String,    // 派生的 unix 账号句柄
}

// 当前验证过的团队身份（来自 tailnet SSO 登录）。团队写操作据此防冒名。
#[tauri::command]
fn tailnet_identity(tn: State<Arc<tailnet::Tailnet>>) -> Identity {
    match tn.status().identity() {
        Some((login, display)) => Identity {
            name: team::slug_login(&login),
            login,
            display,
        },
        None => Identity { login: String::new(), display: String::new(), name: String::new() },
    }
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

// 解析一台机的连接材料：拓扑 + 凭据（含可选跳板）。锁不跨 await。
type ConnParts = (store::Server, String, Option<(store::Server, String)>);
fn resolve_conn(state: &State<'_, AppState>, server: &str) -> Result<ConnParts, String> {
    let (target, jump_srv) = {
        let _g = state.lock.lock().unwrap();
        let list = store::load(&state.dir);
        let target = list
            .iter()
            .find(|s| s.name == server)
            .cloned()
            .ok_or_else(|| format!("服务器 {server} 不存在"))?;
        let jump_srv = if target.transport == "jump" {
            match target.jump.as_deref() {
                Some(jn) => Some(
                    list.iter()
                        .find(|s| s.name == jn)
                        .cloned()
                        .ok_or_else(|| format!("跳板 {jn} 不存在"))?,
                ),
                None => None,
            }
        } else {
            None
        };
        (target, jump_srv)
    };

    let target_secret = state.vault.get(server)?;
    let jump = match jump_srv {
        Some(j) => {
            let js = state.vault.get(&j.name)?;
            Some((j, js))
        }
        None => None,
    };
    Ok((target, target_secret, jump))
}

#[tauri::command]
async fn ssh_open(
    app: AppHandle,
    state: State<'_, AppState>,
    server: String,
    ws: Option<String>,
) -> Result<String, String> {
    let _ = ws; // 阶段 4：tmux 持久会话
    let (target, target_secret, jump) = resolve_conn(&state, &server)?;
    ssh::open(app, state.sessions.clone(), target, target_secret, jump).await
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app
                .path()
                .app_config_dir()
                .map_err(|e| format!("无法定位配置目录: {e}"))?;
            app.manage(AppState {
                dir: dir.clone(),
                lock: Mutex::new(()),
                sessions: Arc::new(ssh::Sessions::new()),
                vault: vault::Vault::new(dir),
            });
            app.manage(Arc::new(tailnet::Tailnet::new()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_state,
            vault_unlock,
            vault_lock,
            vault_reset,
            get_username,
            set_username,
            list_servers,
            upsert_server,
            del_server,
            read_ssh_config,
            import_ssh_hosts,
            load_team,
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
            save_credential,
            del_credential,
            ssh_open,
            ssh_write,
            ssh_resize,
            ssh_close,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
