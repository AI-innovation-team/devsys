// 本机终端:本地 PTY。「本机也是节点」的兑现 —— 点织物图上的本机节点,直接出本地 shell,
// 不走 SSH、不依赖 sshd。事件与远程会话同构(ssh://data/<id> / ssh://close/<id>,
// 写入/resize/关闭同一套 SessionCmd),前端 xterm/传输层零改动。
// 持久工作区同样兑现:ws 名非空就跑同一段 tmux attach-or-create 脚本 —— 本地远程一个心智。
use std::io::{Read, Write};
use std::sync::Arc;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tauri::{AppHandle, Emitter};

use crate::ssh::{tmux_command, tmux_session_name, SessionCmd, Sessions};

// 本机节点的保留名(前端织物图/工作区用同一个;"~" 避开用户自定义服务器名)。
pub const LOCAL_NODE: &str = "~local";

pub fn open(app: AppHandle, sessions: Arc<Sessions>, ws: Option<String>) -> Result<String, String> {
    // 先校验工作区名(注入白名单与远程同一道防线)。
    let tmux = match ws.as_deref().filter(|s| !s.is_empty()) {
        Some(w) => Some(tmux_session_name(w)?),
        None => None,
    };

    let pty = native_pty_system()
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("开本地 PTY 失败: {e}"))?;
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut cmd = match &tmux {
        // 与远程完全同一段脚本:tmux attach-or-create,没装 tmux 回退登录 shell。
        Some(name) => {
            let mut c = CommandBuilder::new("/bin/sh");
            c.args(["-c", &tmux_command(name)]);
            c
        }
        None => {
            let mut c = CommandBuilder::new(&shell);
            c.arg("-l");
            c
        }
    };
    cmd.env("TERM", "xterm-256color");
    if let Ok(home) = std::env::var("HOME") {
        cmd.cwd(home);
    }
    let mut child = pty.slave.spawn_command(cmd).map_err(|e| format!("启动 shell 失败: {e}"))?;
    drop(pty.slave); // 关掉我们这端的 slave,否则子进程退出后 reader 永远读不到 EOF

    let mut reader = pty.master.try_clone_reader().map_err(|e| e.to_string())?;
    let mut writer = pty.master.take_writer().map_err(|e| e.to_string())?;
    let master = pty.master;

    let (id, mut rx) = sessions.register(LOCAL_NODE);
    let _ = app.emit("ssh://active", sessions.active());

    let data_ev = format!("ssh://data/{id}");
    let close_ev = format!("ssh://close/{id}");

    // 读线程(PTY 读是阻塞 IO):输出 → 前端事件。EOF(shell 退出/被杀)→ 发 Close 触发清理。
    let app_r = app.clone();
    let sess_r = sessions.clone();
    let id_r = id.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = app_r.emit(&data_ev, buf[..n].to_vec());
                }
            }
        }
        sess_r.send(&id_r, SessionCmd::Close);
    });

    // 命令任务:写入 / resize / 关闭(写是小块阻塞 IO,可接受)。
    let app_c = app.clone();
    let sessions2 = sessions.clone();
    let id2 = id.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                SessionCmd::Data(d) => {
                    let _ = writer.write_all(&d);
                    let _ = writer.flush();
                }
                SessionCmd::Resize(c, r) => {
                    let _ = master.resize(PtySize {
                        rows: r as u16,
                        cols: c as u16,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
                SessionCmd::Close => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        sessions2.unregister(&id2);
        let _ = app_c.emit(&close_ev, ());
        let _ = app_c.emit("ssh://active", sessions2.active());
    });

    Ok(id)
}
