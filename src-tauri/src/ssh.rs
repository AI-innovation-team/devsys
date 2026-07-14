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
    map: Mutex<HashMap<String, mpsc::UnboundedSender<SessionCmd>>>,
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
        if let Some(tx) = self.map.lock().unwrap().get(id) {
            let _ = tx.send(cmd);
        }
    }

    // 关闭全部会话（退出登录时用）。
    pub fn close_all(&self) {
        for tx in self.map.lock().unwrap().values() {
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

// 保活：ProxyJump 时目标会话跑在跳板连接的通道流上，需持有跳板 handle 不让它断。
struct Conn {
    handle: Handle<Client>,
    _jump: Option<Handle<Client>>,
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

async fn connect_direct(
    config: Arc<client::Config>,
    server: &Server,
    secret: &str,
) -> Result<Handle<Client>, String> {
    let addr = format!("{}:{}", server.host, server.port);
    let mut handle = client::connect(config, addr, Client)
        .await
        .map_err(|e| format!("连接 {} 失败：{e}", server.host))?;
    auth(&mut handle, server, secret).await?;
    Ok(handle)
}

async fn build_conn(
    target: &Server,
    target_secret: &str,
    jump: Option<(Server, String)>,
) -> Result<Conn, String> {
    let config = Arc::new(client::Config::default());
    match jump {
        Some((jsrv, jsec)) => {
            // 先连跳板，再在其上开一条到目标 SSH 端口的 direct-tcpip 通道。
            let jhandle = connect_direct(config.clone(), &jsrv, &jsec).await?;
            let channel = jhandle
                .channel_open_direct_tcpip(
                    target.host.clone(),
                    target.port as u32,
                    "127.0.0.1".to_string(),
                    0,
                )
                .await
                .map_err(|e| format!("经 {} 开跳板通道失败：{e}", jsrv.name))?;
            let stream = channel.into_stream();
            let mut handle = client::connect_stream(config, stream, Client)
                .await
                .map_err(|e| format!("经跳板连 {} 失败：{e}", target.host))?;
            auth(&mut handle, target, target_secret).await?;
            Ok(Conn {
                handle,
                _jump: Some(jhandle),
            })
        }
        None => Ok(Conn {
            handle: connect_direct(config, target, target_secret).await?,
            _jump: None,
        }),
    }
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
    jump: Option<(Server, String)>,
    command: String,
) -> Result<ExecOut, String> {
    let conn = build_conn(&target, &target_secret, jump).await?;
    let mut channel = conn.handle.channel_open_session().await.map_err(e2s)?;
    channel.exec(true, command).await.map_err(e2s)?;

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

// 建立会话：连接 + PTY + shell，起后台任务桥接，返回 session id。
pub async fn open(
    app: AppHandle,
    sessions: Arc<Sessions>,
    target: Server,
    target_secret: String,
    jump: Option<(Server, String)>,
) -> Result<String, String> {
    let conn = build_conn(&target, &target_secret, jump).await?;

    let mut channel = conn.handle.channel_open_session().await.map_err(e2s)?;
    channel
        .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .map_err(e2s)?;
    channel.request_shell(true).await.map_err(e2s)?;

    let id = sessions.next_id();
    let (tx, mut rx) = mpsc::unbounded_channel::<SessionCmd>();
    sessions.map.lock().unwrap().insert(id.clone(), tx);

    let app2 = app.clone();
    let id2 = id.clone();
    let sessions2 = sessions.clone();
    let data_ev = format!("ssh://data/{id}");
    let close_ev = format!("ssh://close/{id}");

    tokio::spawn(async move {
        let _keepalive = conn; // 持有 handle（含跳板）直到会话结束
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
    });

    Ok(id)
}
