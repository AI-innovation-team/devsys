// DevSys 桌面核心。
//   阶段 0：脚手架 + SSH 命令桩（打通 transport → 命令 → data/close 事件 → xterm）。
//   阶段 1：本地拓扑（store）+ 凭据（vault，Stronghold 加密保险库）命令。
//   阶段 2：ssh_* 桩替换为 russh 原生会话（直连/ProxyJump/tailnet）。
mod ssh;
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

// 邀成员：把 {name, pubkey} 写进 team.yaml（贡献侧据此把公钥同步进各机 authorized_keys）。
#[tauri::command]
fn add_member(path: String, name: String, pubkey: String) -> Result<team::TeamConfig, String> {
    let mut cfg = read_team(&path)?;
    team::upsert_member(&mut cfg, team::TeamMember { name, pubkey });
    write_team(&path, &cfg)?;
    Ok(cfg)
}

// 贡献一台机：把本地机的拓扑写进 team.yaml + 给本地机打 shared_to 标记（凭据不出本机）。
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
    team::upsert_machine(
        &mut cfg,
        team::TeamMachine {
            name: srv.name.clone(),
            host: srv.host.clone(),
            port: srv.port,
            jump: srv.jump.clone(),
            username: srv.username.clone(),
            transport: srv.transport.clone(),
            tier,
        },
    );
    write_team(&team_path, &cfg)?;
    store::set_shared(&state.dir, &server, &format!("team:{}", cfg.team), true)
}

// 撤销贡献：从 team.yaml 删该机 + 去掉本地 shared_to 标记。
#[tauri::command]
fn unshare_server(
    state: State<AppState>,
    team_path: String,
    server: String,
) -> Result<Vec<store::Server>, String> {
    let _g = state.lock.lock().unwrap();
    let mut cfg = read_team(&team_path)?;
    team::remove_machine(&mut cfg, &server);
    write_team(&team_path, &cfg)?;
    store::set_shared(&state.dir, &server, &format!("team:{}", cfg.team), false)
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

#[tauri::command]
async fn ssh_open(
    app: AppHandle,
    state: State<'_, AppState>,
    server: String,
    ws: Option<String>,
) -> Result<String, String> {
    let _ = ws; // 阶段 4：tmux 持久会话
    // 同步读取拓扑 + 可选跳板（不跨 await 持锁）
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

    let target_secret = state.vault.get(&server)?;
    let jump = match jump_srv {
        Some(j) => {
            let js = state.vault.get(&j.name)?;
            Some((j, js))
        }
        None => None,
    };

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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            vault_state,
            vault_unlock,
            vault_lock,
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
            share_server,
            unshare_server,
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
