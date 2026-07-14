// 内建 tsnet sidecar 的生命周期管理。
//
// 我们不装系统级 Tailscale —— 把一个 tailnet 节点用官方 tsnet 库嵌进随 app 分发的
// helper 二进制(tsnet-helper),零系统依赖。Rust 这边只负责:拉起进程、喂参数、
// 读它经 stdout 报的行分隔 JSON 状态、需要时 kill。
//
// 出站:helper 开本地 SOCKS5(:1055),russh 走它经 tailnet 连内网机。
// 入站:helper 把 tailnet:22 代理到本机 sshd,让队友连进来(app 开着才在网上)。
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

// helper 经 stdout 报的状态（与 tsnet-helper/main.go 的 status 对应）。
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct TailnetStatus {
    pub state: String, // starting | running | error | stopped
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub auth_url: String,
    #[serde(default)]
    pub socks: String,
    #[serde(default)]
    pub ingress: bool,
    #[serde(default)]
    pub error: String,
}

pub struct Tailnet {
    child: Mutex<Option<Child>>,
    last: Mutex<TailnetStatus>,
}

impl Tailnet {
    pub fn new() -> Self {
        Tailnet {
            child: Mutex::new(None),
            last: Mutex::new(TailnetStatus {
                state: "stopped".into(),
                ..Default::default()
            }),
        }
    }

    pub fn status(&self) -> TailnetStatus {
        self.last.lock().unwrap().clone()
    }

    #[allow(dead_code)] // ssh 层走 tailnet 传输时会用到
    pub fn is_running(&self) -> bool {
        self.child.lock().unwrap().is_some()
    }

    // 当前 SOCKS 代理地址（供 ssh 层走 tailnet 传输时用）；未连通则 None。
    #[allow(dead_code)]
    pub fn socks_addr(&self) -> Option<String> {
        let s = self.last.lock().unwrap();
        if s.state == "running" && !s.socks.is_empty() && !s.ip.is_empty() {
            Some(s.socks.clone())
        } else {
            None
        }
    }

    // 启动 sidecar。self_arc = 本实例的 Arc（后台读线程更新状态用，避免 try_state）。
    // authkey 可空（空则 helper 会报 auth_url 让用户浏览器登录）。ingress=是否开入站。
    pub fn start(
        self: &Arc<Self>,
        app: AppHandle,
        helper_path: &str,
        dir: &str,
        hostname: &str,
        authkey: &str,
        ingress: bool,
    ) -> Result<(), String> {
        let mut guard = self.child.lock().unwrap();
        if guard.is_some() {
            return Err("tailnet 已在运行".into());
        }

        let mut cmd = Command::new(helper_path);
        cmd.args(["--dir", dir, "--hostname", hostname, "--socks", "127.0.0.1:1055"]);
        if !authkey.is_empty() {
            cmd.args(["--authkey", authkey]);
        }
        if ingress {
            cmd.arg("--ingress");
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = cmd.spawn().map_err(|e| format!("启动 tailnet sidecar 失败: {e}"))?;
        let stdout = child.stdout.take().ok_or("无法读取 sidecar 输出")?;

        *self.last.lock().unwrap() = TailnetStatus {
            state: "starting".into(),
            ..Default::default()
        };

        // 后台读状态行，更新缓存 + 转发给前端事件。
        let app2 = app.clone();
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let line = line.trim();
                if line.is_empty() || !line.starts_with('{') {
                    continue; // 忽略任何非 JSON 噪声
                }
                if let Ok(st) = serde_json::from_str::<TailnetStatus>(line) {
                    *me.last.lock().unwrap() = st.clone();
                    let _ = app2.emit("tailnet://status", st);
                }
            }
            // stdout 关闭 = sidecar 退出。
            {
                let mut s = me.last.lock().unwrap();
                if s.state != "error" {
                    *s = TailnetStatus { state: "stopped".into(), ..Default::default() };
                }
            }
            *me.child.lock().unwrap() = None;
            let _ = app2.emit("tailnet://status", TailnetStatus { state: "stopped".into(), ..Default::default() });
        });

        *guard = Some(child);
        Ok(())
    }

    // 停止 sidecar（kill 进程；drop 掉的 stdin 会让 helper 的 io.Copy 退出，双保险直接 kill）。
    pub fn stop(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        *self.last.lock().unwrap() = TailnetStatus {
            state: "stopped".into(),
            ..Default::default()
        };
    }
}

impl Drop for Tailnet {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
        }
    }
}
