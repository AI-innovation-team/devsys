// 本机作为节点。呼应北极星:**没有「本地/远程」之分,只有节点** —— 你自己这台机器
// 也是织物上的一个节点,可以贡献给团队:
//   · 它本身就是算力(队友直接登进来跑计算)
//   · 或它是通往你内网的**跳板**(队友经它到达你的计算设备)
//
// 要让队友连进来,这台机必须:
//   1) 跑着 sshd(macOS = 系统设置里的「远程登录」;Linux = sshd 服务)
//   2) 有一个队友够得着的地址(同网段 LAN IP;跨网就得 tailnet)
// 两点我们都探测并如实告知 —— 不假装能穿透。
use serde::Serialize;
use std::process::Command;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Addr {
    pub value: String,
    pub kind: String, // tailnet | lan | hostname
    pub hint: String,
}

#[derive(Serialize, Debug)]
pub struct SelfNode {
    pub hostname: String,
    pub username: String,
    pub addrs: Vec<Addr>, // 建议给团队用的地址（tailnet 优先）
    pub sshd: bool,       // 本机是否在监听 22（队友能不能连进来的前提）
    pub notes: Vec<String>,
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// 本机是否在监听 SSH 端口（队友要连进来，这是硬前提）。
fn sshd_listening() -> bool {
    // lsof 在 macOS/Linux 都有；查 22 端口的 LISTEN。
    run("lsof", &["-nP", "-iTCP:22", "-sTCP:LISTEN"]).is_some()
        || run("ss", &["-lnt", "sport = :22"]).is_some_and(|s| s.contains(":22"))
}

// 注：tailnet IP 现在来自我们**内建的** tsnet sidecar（detect 的入参），
// 不再调系统 `tailscale`（那与"零系统依赖"矛盾）。

// 非回环的 IPv4（同网段队友可达）。
fn lan_ips() -> Vec<String> {
    let mut out = Vec::new();
    // macOS: ifconfig；Linux: ip -4 addr
    let text = run("ifconfig", &[])
        .or_else(|| run("ip", &["-4", "addr"]))
        .unwrap_or_default();
    for line in text.lines() {
        let t = line.trim();
        let ip = if let Some(r) = t.strip_prefix("inet ") {
            r.split_whitespace().next()
        } else {
            None
        };
        if let Some(ip) = ip {
            let ip = ip.split('/').next().unwrap_or(ip); // Linux 带 /24
            if ip.starts_with("127.") || ip == "0.0.0.0" {
                continue;
            }
            // 100.64/10 是 tailnet CGNAT 段，单列，不混进 LAN。
            if ip.starts_with("100.") {
                continue;
            }
            if !out.contains(&ip.to_string()) {
                out.push(ip.to_string());
            }
        }
    }
    out
}

// tailnet_ip 由内建 sidecar 提供（未连通则 None）。
pub fn detect(tailnet_ip: Option<String>) -> SelfNode {
    let hostname = run("hostname", &["-s"])
        .or_else(|| run("hostname", &[]))
        .unwrap_or_else(|| "localhost".into());
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();

    let mut addrs = Vec::new();
    if let Some(ts) = tailnet_ip.filter(|s| !s.is_empty()) {
        addrs.push(Addr {
            value: ts,
            kind: "tailnet".into(),
            hint: "经内建 tailnet —— 跨内网也能到达（推荐）".into(),
        });
    }
    for ip in lan_ips() {
        addrs.push(Addr {
            value: ip,
            kind: "lan".into(),
            hint: "同一局域网内的队友可达；跨网不行".into(),
        });
    }
    addrs.push(Addr {
        value: format!("{hostname}.local"),
        kind: "hostname".into(),
        hint: "mDNS 名，同网段可解析".into(),
    });

    let sshd = sshd_listening();
    let mut notes = Vec::new();
    if !sshd {
        notes.push(
            "本机没有在监听 SSH（22）—— 队友连不进来。macOS：系统设置 → 通用 → 共享 → 打开「远程登录」；Linux：启用 sshd。"
                .into(),
        );
    }
    if !addrs.iter().any(|a| a.kind == "tailnet") {
        notes.push(
            "内建 tailnet 未连接。只有 LAN 地址的话，只有同网段的队友能连 —— 跨内网请先在设置里启用 tailnet。"
                .into(),
        );
    }
    notes.push(
        "本机既可作为**算力**贡献（队友登进来跑计算），也可作为**跳板**（队友经它到达你的内网机器）。"
            .into(),
    );

    SelfNode {
        hostname,
        username,
        addrs,
        sshd,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_gives_usable_shape() {
        let n = detect(Some("100.100.1.1".into()));
        assert_eq!(n.addrs[0].kind, "tailnet"); // 内建 tailnet IP 排最前
        let n = detect(None);
        assert!(!n.hostname.is_empty());
        // 至少有个 hostname 兜底地址
        assert!(!n.addrs.is_empty());
        assert!(n.addrs.iter().any(|a| a.kind == "hostname"));
        // tailnet 若存在必须排在最前（跨内网可达是首选）
        if n.addrs.iter().any(|a| a.kind == "tailnet") {
            assert_eq!(n.addrs[0].kind, "tailnet");
        }
        // 永远提示"既是算力也可当跳板"
        assert!(n.notes.iter().any(|s| s.contains("跳板")));
    }

    #[test]
    fn lan_ips_exclude_loopback_and_tailnet() {
        for ip in lan_ips() {
            assert!(!ip.starts_with("127."), "回环不该出现: {ip}");
            assert!(!ip.starts_with("100."), "tailnet 段应单列: {ip}");
        }
    }
}
