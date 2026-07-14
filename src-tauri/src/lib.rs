// DevSys 桌面核心。
//   阶段 0：脚手架 + SSH 命令桩（打通 transport → 命令 → data/close 事件 → xterm）。
//   阶段 1：本地拓扑（store）+ 凭据（vault，Stronghold 加密保险库）命令。
//   阶段 2：ssh_* 桩替换为 russh 原生会话（直连/ProxyJump/tailnet）。
mod acl;
mod e2e; // 共享闭环的全链路集成测试（#[cfg(test)]）
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

#[derive(Serialize)]
struct LoadTeamResult {
    team: String,
    added: usize,
    skipped: Vec<String>, // 名字与本地/其它来源冲突 → 让本地的赢,跳过
    servers: Vec<store::Server>,
}

#[tauri::command]
fn load_team(path: String, state: State<AppState>) -> Result<LoadTeamResult, String> {
    let text = std::fs::read_to_string(&path).map_err(|_| format!("未找到或无法读取 {path}"))?;
    let cfg = team::parse(&text)?;
    let src = format!("team:{}", cfg.team);

    let _g = state.lock.lock().unwrap();
    let mut list = store::load(&state.dir);

    // 记住本团队旧条目的 has_secret(用户已配自己的凭据,刷新式加载别抹掉)。
    let prev: std::collections::HashMap<String, bool> = list
        .iter()
        .filter(|s| s.source == src)
        .map(|s| (s.name.clone(), s.has_secret))
        .collect();
    // 刷新:先移除本团队旧条目,再按最新 team.yaml 重建。
    list.retain(|s| s.source != src);

    let mut skipped = Vec::new();
    let mut added = 0usize;
    for m in cfg.machines {
        // 与本地(我加的)或其它来源同名 → 本地优先,跳过团队条目。
        if list.iter().any(|s| s.name == m.name) {
            skipped.push(m.name);
            continue;
        }
        let has_secret = *prev.get(&m.name).unwrap_or(&false);
        list.push(store::Server {
            name: m.name,
            host: m.host,
            port: m.port,
            jump: m.jump,
            username: m.username,
            auth: "password".into(), // 连接时用户在「设置凭据」里各自选 key/password
            transport: m.transport,
            source: src.clone(),
            shared_to: vec![],
            has_secret,
        });
        added += 1;
    }

    store::save(&state.dir, &list)?;
    Ok(LoadTeamResult {
        team: cfg.team,
        added,
        skipped,
        servers: list,
    })
}

// team.yaml 读/写小助手（命令层只做 IO 编排，逻辑在 team.rs 纯函数里）。
fn read_team(path: &str) -> Result<team::TeamConfig, String> {
    let text = std::fs::read_to_string(path).map_err(|_| format!("未找到或无法读取 {path}"))?;
    team::parse(&text)
}
fn write_team(path: &str, cfg: &team::TeamConfig) -> Result<(), String> {
    std::fs::write(path, team::to_yaml(cfg)?).map_err(|e| e.to_string())
}

// 读一份 team.yaml 的内容（给贡献 UI 展示当前成员/共享机）。
#[tauri::command]
fn read_team_file(path: String) -> Result<team::TeamConfig, String> {
    read_team(&path)
}

// 新建团队：生成一份 team.yaml，创建者作为首个成员。
#[tauri::command]
fn create_team(path: String, team_name: String, member: String, pubkey: String) -> Result<(), String> {
    if team_name.trim().is_empty() {
        return Err("团队名不能为空".into());
    }
    let cfg = team::new_config(&team_name, team::TeamMember { name: member, pubkey });
    write_team(&path, &cfg)
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

// 邀成员：把 {name, pubkey} 写进 team.yaml（贡献侧据此把公钥同步进各机 authorized_keys）。
#[tauri::command]
fn add_member(path: String, name: String, pubkey: String) -> Result<team::TeamConfig, String> {
    let mut cfg = read_team(&path)?;
    team::upsert_member(&mut cfg, team::TeamMember { name, pubkey });
    write_team(&path, &cfg)?;
    Ok(cfg)
}

// 一台本地机 → team.yaml 的条目。
fn to_machine(s: &store::Server, tier: u8) -> team::TeamMachine {
    team::TeamMachine {
        name: s.name.clone(),
        host: s.host.clone(),
        port: s.port,
        jump: s.jump.clone(),
        username: s.username.clone(),
        transport: s.transport.clone(),
        tier,
    }
}

// 贡献一台机：把本地机的拓扑写进 team.yaml + 给本地机打 shared_to 标记（凭据不出本机）。
//
// 跳板链必须完整：共享一台走跳板的机器时，**跳板本身也得在 team.yaml 里**，
// 否则队友拿到的是一个指向不存在跳板的条目 —— 根本连不上。
// 跳板按档 0（纯跳板：只借道、不给 shell）自动一并共享，除非它已被单独共享（那就不动它的档）。
#[tauri::command]
fn share_server(
    state: State<AppState>,
    team_path: String,
    server: String,
    tier: u8,
) -> Result<Vec<store::Server>, String> {
    let _g = state.lock.lock().unwrap();
    let list = store::load(&state.dir);
    let srv = list.iter().find(|s| s.name == server).ok_or("找不到该服务器")?;
    if srv.source != "mine" {
        return Err("只能贡献自己的机器（团队给的机器不可再贡献）".into());
    }

    let mut cfg = read_team(&team_path)?;
    let team_src = format!("team:{}", cfg.team);

    // 先补跳板（可能是链：A 经 B 经 C，逐级向上）。
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
    for j in &chain {
        // 已在 team.yaml 里就别改它的档位（可能被主人单独设过）。
        if !cfg.machines.iter().any(|m| m.name == j.name) {
            team::upsert_machine(&mut cfg, to_machine(j, 0)); // 档 0：只借道
        }
    }

    team::upsert_machine(&mut cfg, to_machine(srv, tier));
    write_team(&team_path, &cfg)?;

    // 本地标记：目标机 + 跳板链上的每一台都算"已共享给这个团队"。
    let mut out = store::set_shared(&state.dir, &server, &team_src, true)?;
    for j in &chain {
        out = store::set_shared(&state.dir, &j.name, &team_src, true)?;
    }
    Ok(out)
}

// 撤销贡献：从 team.yaml 删该机 + 去掉本地 shared_to 标记。
// 不允许撤掉仍被其他共享机当跳板的机器 —— 那会把队友的跳板链弄断（他们连不上了）。
#[tauri::command]
fn unshare_server(
    state: State<AppState>,
    team_path: String,
    server: String,
) -> Result<Vec<store::Server>, String> {
    let _g = state.lock.lock().unwrap();
    let mut cfg = read_team(&team_path)?;

    let dependents: Vec<&str> = cfg
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

    team::remove_machine(&mut cfg, &server);
    write_team(&team_path, &cfg)?;
    store::set_shared(&state.dir, &server, &format!("team:{}", cfg.team), false)
}

// 把 team.yaml 的 tier 档位编译成 Tailscale ACL 计划（policy 片段 + 每台机的落地命令）。
// 只产出计划供人 review —— 我们不替用户改他的 tailnet（责任为门：主人自己拍板、自己贴）。
#[tauri::command]
fn compile_acl(path: String) -> Result<acl::AclPlan, String> {
    Ok(acl::compile(&read_team(&path)?))
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

// 生成下发脚本（不连接、不执行）。tier 缺省用 team.yaml 里该机的档位。
#[tauri::command]
fn provision_preview(
    team_path: String,
    server: String,
    tier: Option<u8>,
) -> Result<provision::ProvisionPlan, String> {
    let cfg = read_team(&team_path)?;
    let t = match tier {
        Some(t) => t,
        None => cfg
            .machines
            .iter()
            .find(|m| m.name == server)
            .map(|m| m.tier)
            .ok_or_else(|| format!("team.yaml 里没有机器 {server}"))?,
    };
    provision::plan(&cfg, &server, t)
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
    tier: Option<u8>,
) -> Result<ProvisionResult, String> {
    let plan = provision_preview(team_path, server.clone(), tier)?;
    if plan.tier == 0 {
        return Ok(ProvisionResult { ok: true, code: 0, output: "档 0（纯跳板）无需下发。".into() });
    }
    if plan.accounts.is_empty() {
        return Err("没有可下发的成员公钥（team.yaml 里成员缺 pubkey）".into());
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
            read_team_file,
            create_team,
            add_member,
            my_pubkeys,
            detect_self,
            share_server,
            unshare_server,
            compile_acl,
            team_git_status,
            team_git_pull,
            team_git_push,
            team_git_clone,
            provision_preview,
            provision_apply,
            tailnet_status,
            tailnet_up,
            tailnet_down,
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
