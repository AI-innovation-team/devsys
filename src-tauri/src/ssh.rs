// 原生 SSH（russh）。语义参考门户 backend/devsys_portal/ssh.py + terminal.py：
//   - 直连 / ProxyJump（经跳板开 direct-tcpip 通道，再在其上跑一个 SSH 会话）
//   - 密码 / 私钥认证
//   - 开 PTY + shell，双向桥接到前端 xterm（data 事件 / ssh_write / ssh_resize / ssh_close）
// tailnet 传输在阶段 3 接 SOCKS；当前按直连处理（要求 OS 已在 tailnet 内）。
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use russh::client::{self, Handle};
use russh::ChannelMsg;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::store::Server;

fn e2s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// 前端 → 会话 IO 任务的指令。
pub enum SessionCmd {
    Data(Vec<u8>),
    Resize(u32, u32),
    Close,
}

// 会话注册表：id → 指令发送端。用 std Mutex（send 非阻塞，不跨 await 持锁）。
pub struct Sessions {
    // id → (服务器名, 命令通道)。存名字是为了回答「哪些机器现在有活连接」——
    // 织物图按它把节点画成「已连接·常亮」。
    map: Mutex<HashMap<String, (String, mpsc::UnboundedSender<SessionCmd>)>>,
    seq: AtomicU64,
}

impl Sessions {
    pub fn new() -> Self {
        Sessions {
            map: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(1),
        }
    }
    fn next_id(&self) -> String {
        format!("s{}", self.seq.fetch_add(1, Ordering::Relaxed))
    }
    pub fn send(&self, id: &str, cmd: SessionCmd) {
        if let Some((_, tx)) = self.map.lock().unwrap().get(id) {
            let _ = tx.send(cmd);
        }
    }

    // 注册一条新会话(远程 SSH 与本地 PTY 共用):返回 (id, 命令接收端)。
    pub fn register(&self, name: &str) -> (String, mpsc::UnboundedReceiver<SessionCmd>) {
        let id = self.next_id();
        let (tx, rx) = mpsc::unbounded_channel();
        self.map.lock().unwrap().insert(id.clone(), (name.to_string(), tx));
        (id, rx)
    }
    pub fn unregister(&self, id: &str) {
        self.map.lock().unwrap().remove(id);
    }

    // 当前有活会话的服务器名（去重）。
    pub fn active(&self) -> Vec<String> {
        let mut v: Vec<String> = self.map.lock().unwrap().values().map(|(n, _)| n.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    // 关闭全部会话（退出登录时用）。
    pub fn close_all(&self) {
        for (_, tx) in self.map.lock().unwrap().values() {
            let _ = tx.send(SessionCmd::Close);
        }
    }
}

// 接受任意服务器公钥（暂不校验 known_hosts；门户侧同样 known_hosts=None）。
struct Client;

#[async_trait::async_trait]
impl client::Handler for Client {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

// 保活：ProxyJump 时目标会话跑在跳板连接的通道流上，需持有整条跳板链的 handle 不让它断。
struct Conn {
    handle: Handle<Client>,
    _jumps: Vec<Handle<Client>>,
}

async fn auth(handle: &mut Handle<Client>, server: &Server, secret: &str) -> Result<(), String> {
    let ok = if server.auth == "key" {
        let key = russh_keys::decode_secret_key(secret, None).map_err(e2s)?;
        handle
            .authenticate_publickey(&server.username, Arc::new(key))
            .await
            .map_err(e2s)?
    } else {
        handle
            .authenticate_password(&server.username, secret)
            .await
            .map_err(e2s)?
    };
    if ok {
        Ok(())
    } else {
        Err("认证失败（用户名 / 密码 / 私钥不匹配）".into())
    }
}

// 入口拨号(整条链的第一跳 TCP)。tailnet 机 + 内嵌 tsnet 在跑 → 经 tsnet 的 SOCKS5 走
// (tsnet 是用户态网络、不建 TUN 网卡,流量必须显式从它走);否则裸 TCP 直连
// (系统级 Tailscale 用户由 OS 路由 100.x,直连即通)。
async fn connect_direct(
    config: Arc<client::Config>,
    server: &Server,
    secret: &str,
    socks: Option<&str>,
) -> Result<Handle<Client>, String> {
    let mut handle = if server.transport == "tailnet" && socks.is_some() {
        let proxy = socks.unwrap();
        let stream = tokio_socks::tcp::Socks5Stream::connect(proxy, (server.host.as_str(), server.port))
            .await
            .map_err(|e| format!("经内嵌 tailnet(SOCKS {proxy})连 {} 失败：{e}", server.host))?;
        client::connect_stream(config, stream, Client)
            .await
            .map_err(|e| format!("连接 {} 失败：{e}", server.host))?
    } else {
        let addr = format!("{}:{}", server.host, server.port);
        client::connect(config, addr, Client)
            .await
            .map_err(|e| format!("连接 {} 失败：{e}", server.host))?
    };
    auth(&mut handle, server, secret).await?;
    Ok(handle)
}

// 在已有连接上开一条到 next 的 direct-tcpip 通道并在其上建立新 SSH 连接（认证）。
async fn hop_through(
    config: Arc<client::Config>,
    via: &Handle<Client>,
    via_name: &str,
    next: &Server,
    next_secret: &str,
) -> Result<Handle<Client>, String> {
    let channel = via
        .channel_open_direct_tcpip(next.host.clone(), next.port as u32, "127.0.0.1".to_string(), 0)
        .await
        .map_err(|e| format!("经 {via_name} 开跳板通道失败：{e}"))?;
    let stream = channel.into_stream();
    let mut handle = client::connect_stream(config, stream, Client)
        .await
        .map_err(|e| format!("经跳板连 {} 失败：{e}", next.host))?;
    auth(&mut handle, next, next_secret).await?;
    Ok(handle)
}

// 建连，支持**多跳跳板链**：jumps 按「外→内」排（先连的在前）。
//   直连:  []                    → connect_direct
//   单跳:  [bastion]             → 连 bastion,再 direct-tcpip 到 target
//   多跳:  [edge, login]         → 连 edge → edge 开道到 login → login 开道到 target
// 校园那种「只能经登录节点、再进算力节点」的两跳以上,靠这条链兑现。
async fn build_conn(
    target: &Server,
    target_secret: &str,
    jumps: Vec<(Server, String)>,
    socks: Option<String>,
) -> Result<Conn, String> {
    let config = Arc::new(client::Config::default());
    let socks = socks.as_deref();
    if jumps.is_empty() {
        return Ok(Conn { handle: connect_direct(config, target, target_secret, socks).await?, _jumps: vec![] });
    }
    let mut held: Vec<Handle<Client>> = Vec::new();
    // 最外层跳板:入口拨号(直连或经内嵌 tailnet)。
    let mut cur = connect_direct(config.clone(), &jumps[0].0, &jumps[0].1, socks).await?;
    // 中间每一跳:在前一跳的连接上开道。
    for i in 1..jumps.len() {
        let next = hop_through(config.clone(), &cur, &jumps[i - 1].0.name, &jumps[i].0, &jumps[i].1).await?;
        held.push(cur);
        cur = next;
    }
    // 最后一跳 → 目标。
    let handle = hop_through(config, &cur, &jumps[jumps.len() - 1].0.name, target, target_secret).await?;
    held.push(cur);
    Ok(Conn { handle, _jumps: held })
}

// 一次性命令的执行结果（授权下发用：装公钥、建账号）。
pub struct ExecOut {
    pub code: u32,
    pub output: String, // stdout + stderr 合并（下发脚本自己回显进度）
}

// 非交互执行一条命令并收集输出。与 open() 共用连接逻辑（含 ProxyJump）。
pub async fn exec(
    target: Server,
    target_secret: String,
    jumps: Vec<(Server, String)>,
    command: String,
    socks: Option<String>,
) -> Result<ExecOut, String> {
    exec_stdin(target, target_secret, jumps, command, None, socks).await
}

// 带 stdin 的 exec。**这是 OS 中立的关键**:要往远端写一段脚本/配置时，命令本身保持
// 纯 token（`docker exec -i C tee /path`），内容走 SSH 数据通道 ——
// 宿主 shell 全程不参与解释，于是 sh / cmd.exe / PowerShell 都一样。
// 以前靠 `sh -s <<'EOF'` heredoc，那是对宿主 shell 的硬依赖，Windows 上直接喂不进去。
pub async fn exec_stdin(
    target: Server,
    target_secret: String,
    jumps: Vec<(Server, String)>,
    command: String,
    stdin: Option<Vec<u8>>,
    socks: Option<String>,
) -> Result<ExecOut, String> {
    let conn = build_conn(&target, &target_secret, jumps, socks).await?;
    let mut channel = conn.handle.channel_open_session().await.map_err(e2s)?;
    channel.exec(true, command).await.map_err(e2s)?;
    if let Some(data) = stdin {
        channel.data(&data[..]).await.map_err(e2s)?;
        channel.eof().await.map_err(e2s)?;
    }

    let mut buf = Vec::new();
    let mut code: Option<u32> = None;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { data }) => buf.extend_from_slice(&data),
            Some(ChannelMsg::ExtendedData { data, .. }) => buf.extend_from_slice(&data),
            Some(ChannelMsg::ExitStatus { exit_status }) => code = Some(exit_status),
            Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
            _ => {}
        }
    }
    Ok(ExecOut {
        code: code.unwrap_or(0),
        output: String::from_utf8_lossy(&buf).to_string(),
    })
}

// 工作区名 → 远端 tmux 会话名。**只允许 [A-Za-z0-9_-]**：这串要进远端 shell 命令，
// 白名单是唯一的注入防线（tmux 本身也不收 . 和 :）。
pub fn tmux_session_name(ws: &str) -> Result<String, String> {
    if ws.is_empty() || ws.len() > 64 {
        return Err(format!("工作区名长度不合法：{ws:?}"));
    }
    if !ws.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("工作区名只能用字母/数字/下划线/连字符：{ws:?}"));
    }
    Ok(ws.to_string())
}

// 持久会话的启动命令：有 tmux 就 attach-or-create（`-A`），没有就退回普通 shell 并说清楚。
// 退回而不是报错——远端没装 tmux 不该让人连不上，但要让人知道这次关掉就丢。
pub fn tmux_command(name: &str) -> String {
    format!(
        "if command -v tmux >/dev/null 2>&1; then exec tmux new-session -A -s '{name}'; \
         else printf '\\033[2m[AIT.dev] 远端没装 tmux —— 本次是普通会话，断开即丢。\\033[0m\\n'; \
         exec \"${{SHELL:-/bin/sh}}\" -l; fi"
    )
}

// 建立会话：连接 + PTY + shell，起后台任务桥接，返回 session id。
// `ws` 非空 = 持久工作区：跑 tmux attach-or-create，断开后远端仍在跑，重连即接回。
pub async fn open(
    app: AppHandle,
    sessions: Arc<Sessions>,
    target: Server,
    target_secret: String,
    jumps: Vec<(Server, String)>,
    ws: Option<String>,
    socks: Option<String>,
) -> Result<String, String> {
    // 先校验工作区名再连接：名字不合法就没必要建连接。
    let tmux = match ws.as_deref().filter(|s| !s.is_empty()) {
        Some(w) => Some(tmux_session_name(w)?),
        None => None,
    };

    let conn = build_conn(&target, &target_secret, jumps, socks).await?;

    let mut channel = conn.handle.channel_open_session().await.map_err(e2s)?;
    channel
        .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .map_err(e2s)?;
    match &tmux {
        Some(name) => channel.exec(true, tmux_command(name)).await.map_err(e2s)?,
        None => channel.request_shell(true).await.map_err(e2s)?,
    }

    let id = sessions.next_id();
    let (tx, mut rx) = mpsc::unbounded_channel::<SessionCmd>();
    sessions.map.lock().unwrap().insert(id.clone(), (target.name.clone(), tx));
    // 活跃集变了 → 广播,织物图把这台机点成「已连接」。
    let _ = app.emit("ssh://active", sessions.active());

    let app2 = app.clone();
    let id2 = id.clone();
    let sessions2 = sessions.clone();
    let data_ev = format!("ssh://data/{id}");
    let close_ev = format!("ssh://close/{id}");

    tokio::spawn(async move {
        let _keepalive = conn; // 持有 handle（含整条跳板链）直到会话结束
        loop {
            tokio::select! {
                msg = channel.wait() => match msg {
                    Some(ChannelMsg::Data { data }) => {
                        let _ = app2.emit(&data_ev, data.to_vec());
                    }
                    Some(ChannelMsg::ExtendedData { data, .. }) => {
                        let _ = app2.emit(&data_ev, data.to_vec());
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                    _ => {}
                },
                cmd = rx.recv() => match cmd {
                    Some(SessionCmd::Data(d)) => { let _ = channel.data(&d[..]).await; }
                    Some(SessionCmd::Resize(c, r)) => { let _ = channel.window_change(c, r, 0, 0).await; }
                    Some(SessionCmd::Close) | None => break,
                }
            }
        }
        sessions2.map.lock().unwrap().remove(&id2);
        let _ = app2.emit(&close_ev, ());
        let _ = app2.emit("ssh://active", sessions2.active());
    });

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_name_accepts_safe_chars() {
        assert_eq!(tmux_session_name("ait-turing-1").unwrap(), "ait-turing-1");
        assert_eq!(tmux_session_name("ws_2").unwrap(), "ws_2");
    }

    // 注入防线：这串会进远端 shell 命令，任何 shell 元字符都必须被挡在外面。
    #[test]
    fn tmux_name_rejects_shell_metachars() {
        for bad in ["a;rm -rf /", "a b", "a'b", "a$(id)", "a`id`", "a|b", "a&b", "a\nb", "a.b", "a:b"] {
            assert!(tmux_session_name(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn tmux_name_rejects_empty_and_overlong() {
        assert!(tmux_session_name("").is_err());
        assert!(tmux_session_name(&"a".repeat(65)).is_err());
    }

    // attach-or-create + 没 tmux 时退回 shell，两条路都要在命令里。
    #[test]
    fn tmux_command_attaches_or_creates_with_fallback() {
        let c = tmux_command("ws1");
        assert!(c.contains("new-session -A -s 'ws1'"));
        assert!(c.contains("command -v tmux"));
        assert!(c.contains("SHELL:-/bin/sh"));
    }
}
