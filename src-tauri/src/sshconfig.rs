// 解析 OpenSSH 客户端配置（~/.ssh/config）→ 可导入的服务器条目。
// 支持 Host / HostName / Port / User / ProxyJump / IdentityFile；跳过通配 Host 与 Include。
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct SshHost {
    pub name: String,           // Host 别名
    pub host: String,           // HostName（缺省用别名）
    pub port: u16,              // Port，默认 22
    pub username: String,       // User
    pub jump: Option<String>,   // ProxyJump 的主机（别名）
    pub auth: String,           // 有 IdentityFile → "key"，否则 "password"
    pub identity_file: Option<String>,
}

// 拆 "Keyword value" / "Keyword=value" / "Keyword = value"，并去引号。
fn split_kv(line: &str) -> (&str, &str) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
        i += 1;
    }
    let kw = &line[..i];
    let val = line[i..]
        .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
        .trim()
        .trim_matches('"');
    (kw, val)
}

fn is_wildcard(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('!')
}

pub fn parse(text: &str) -> Vec<SshHost> {
    let mut hosts: Vec<SshHost> = Vec::new();
    let mut cur: Option<SshHost> = None;

    let flush = |cur: &mut Option<SshHost>, hosts: &mut Vec<SshHost>| {
        if let Some(h) = cur.take() {
            hosts.push(h);
        }
    };

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (kw, val) = split_kv(line);
        match kw.to_ascii_lowercase().as_str() {
            "host" => {
                flush(&mut cur, &mut hosts);
                // 取第一个非通配别名作为条目名
                let alias = val.split_whitespace().find(|a| !is_wildcard(a));
                cur = alias.map(|a| SshHost {
                    name: a.to_string(),
                    host: a.to_string(),
                    port: 22,
                    username: String::new(),
                    jump: None,
                    auth: "password".into(),
                    identity_file: None,
                });
            }
            "hostname" => {
                if let Some(h) = cur.as_mut() {
                    h.host = val.to_string();
                }
            }
            "port" => {
                if let Some(h) = cur.as_mut() {
                    if let Ok(p) = val.parse::<u16>() {
                        h.port = p;
                    }
                }
            }
            "user" => {
                if let Some(h) = cur.as_mut() {
                    h.username = val.to_string();
                }
            }
            "proxyjump" => {
                if let Some(h) = cur.as_mut() {
                    // 取第一跳，剥掉 user@ 和 :port
                    let first = val.split(',').next().unwrap_or(val).trim();
                    let after_at = first.rsplit('@').next().unwrap_or(first);
                    let host_only = after_at.split(':').next().unwrap_or(after_at).trim();
                    if !host_only.is_empty() && !host_only.eq_ignore_ascii_case("none") {
                        h.jump = Some(host_only.to_string());
                    }
                }
            }
            "identityfile" => {
                if let Some(h) = cur.as_mut() {
                    h.auth = "key".into();
                    h.identity_file = Some(val.to_string());
                }
            }
            _ => {}
        }
    }
    flush(&mut cur, &mut hosts);
    hosts.into_iter().filter(|h| !h.name.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_basic() {
        let cfg = "\
Host *
  ForwardAgent yes

Host gpu\n  HostName 10.0.0.5\n  User alice\n  Port 2222\n  IdentityFile ~/.ssh/id_ed25519

Host far
  HostName 10.0.0.9
  ProxyJump alice@gpu:2222
";
        let hs = super::parse(cfg);
        assert_eq!(hs.len(), 2); // Host * 被过滤
        let gpu = &hs[0];
        assert_eq!(gpu.name, "gpu");
        assert_eq!(gpu.host, "10.0.0.5");
        assert_eq!(gpu.port, 2222);
        assert_eq!(gpu.username, "alice");
        assert_eq!(gpu.auth, "key");
        let far = &hs[1];
        assert_eq!(far.jump.as_deref(), Some("gpu"));
    }
}
