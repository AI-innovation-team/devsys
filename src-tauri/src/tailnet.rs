// 内建 tsnet sidecar 的生命周期管理。
//
// 我们不装系统级 Tailscale —— 把一个 tailnet 节点用官方 tsnet 库嵌进随 app 分发的
// helper 二进制(tsnet-helper),零系统依赖。Rust 这边只负责:拉起进程、喂参数、
// 读它经 stdout 报的行分隔 JSON 状态、需要时 kill。
//
// 出站:helper 开本地 SOCKS5(:1055),russh 走它经 tailnet 连内网机。
// 入站:helper 把 tailnet:22 代理到本机 sshd,让队友连进来(app 开着才在网上)。
use std::io::{BufRead, BufReader};
use std::collections::VecDeque;
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
    // 验证过的身份（团队 IdP 的 SSO 登录，不可伪造）。
    #[serde(default)]
    pub login: String,
    #[serde(default)]
    pub display: String,
    #[serde(default)]
    pub error: String,
    // 从 helper 日志里认出来的**人话结论**（比如「系统代理把控制面截断了」）。
    // helper 自己只会闷头重试，这条是我们替用户读日志得出的判断。
    #[serde(default)]
    pub hint: String,
}

impl TailnetStatus {
    // 已登录且拿到身份 → 返回 (login, display)。用于团队操作的可验证身份锚。
    pub fn identity(&self) -> Option<(String, String)> {
        if self.state == "running" && !self.login.is_empty() {
            Some((self.login.clone(), self.display.clone()))
        } else {
            None
        }
    }
}

pub struct Tailnet {
    child: Mutex<Option<Child>>,
    last: Mutex<TailnetStatus>,
    // helper 最近的日志行。接入卡住时这是唯一的线索来源。
    log: Mutex<VecDeque<String>>,
}

// 保留多少行 helper 日志（够定位、又不至于让状态包变胖）。
const LOG_KEEP: usize = 40;

// 把 tsnet 的日志翻译成人话。只认**确定性的**症状 —— 认不出就别瞎猜，
// 原始日志照样给用户看。
fn diagnose(line: &str) -> Option<String> {
    let l = line.to_lowercase();
    if l.contains("fetch control key") || (l.contains("control") && l.contains("eof")) {
        return Some(
            "连不上控制面：像是**系统代理/VPN 把 HTTPS 截断了**（clash、mihomo 这类）。\
             我们已给 helper 设了 NO_PROXY 直连控制面；如果你在「设置」页点的连接，\
             那里没带团队控制面地址 —— 改到「团队」页点，或先退出代理再试。"
                .into(),
        );
    }
    if l.contains("no such host") || l.contains("dns") && l.contains("fail") {
        return Some("解析不了控制面域名 —— 检查这台机的 DNS / 域名是否写对。".into());
    }
    if l.contains("certificate") || l.contains("x509") {
        return Some("控制面的 TLS 证书校验失败 —— 检查系统时间，或代理是否在做中间人。".into());
    }
    if l.contains("connection refused") {
        return Some("控制面拒绝连接 —— 服务可能没在跑，或端口不对。".into());
    }
    None
}

impl Tailnet {
    pub fn new() -> Self {
        Tailnet {
            child: Mutex::new(None),
            log: Mutex::new(VecDeque::new()),
            last: Mutex::new(TailnetStatus {
                state: "stopped".into(),
                ..Default::default()
            }),
        }
    }

    pub fn status(&self) -> TailnetStatus {
        self.last.lock().unwrap().clone()
    }

    // helper 最近的日志。接入卡住时唯一的线索 —— UI 上给个「看日志」直接摊开，
    // 免得再来一轮「你那边报什么错」「不知道，就一直转」。
    pub fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().cloned().collect()
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
        control: &str,
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
        if !control.is_empty() {
            cmd.args(["--control", control]); // 团队自建 Headscale;空则官方 Tailscale
            // 自建控制面必须**绕过系统代理直连** —— clash/mihomo/VPN 会把控制面 HTTPS
            // 截成 EOF(实测:tsnet 走系统代理连 headscale 一直 fetch control key: EOF)。
            let host = control
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .split('/')
                .next()
                .unwrap_or(control);
            cmd.env("NO_PROXY", host);
            cmd.env("no_proxy", host);
        }
        if ingress {
            cmd.arg("--ingress");
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // ★ 别丢 stderr。tsnet 的日志全在这儿，而**接入失败几乎只在这儿说话** ——
            // 头号坑「系统代理把控制面 HTTPS 截成 fetch control key: EOF」就是这样：
            // 状态一直停在 starting，stdout 上一个字都没有。以前 stderr 扔了，
            // 用户只看到「一直在连接」，我们也无从判断。
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| format!("启动 tailnet sidecar 失败: {e}"))?;
        let stdout = child.stdout.take().ok_or("无法读取 sidecar 输出")?;
        let stderr = child.stderr.take().ok_or("无法读取 sidecar 日志")?;

        // 日志线程：留最近若干行，塞进状态里给 UI 看。
        {
            let me = Arc::clone(self);
            let app3 = app.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    let hint = diagnose(&line);
                    let mut log = me.log.lock().unwrap();
                    log.push_back(line);
                    while log.len() > LOG_KEEP {
                        log.pop_front();
                    }
                    drop(log);
                    // 认得出的致命症状：直接把人话结论推给 UI，别让人干等。
                    if let Some(h) = hint {
                        let mut s = me.last.lock().unwrap();
                        if s.hint != h {
                            s.hint = h;
                            let _ = app3.emit("tailnet://status", s.clone());
                        }
                    }
                }
            });
        }

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
