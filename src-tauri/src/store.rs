// 本地服务器拓扑存储（替代门户的 servers.json + config.yaml）。
// 真源是 app 配置目录下的 servers.json，用户在界面里增删改，即时落盘。
// 参考现有门户的 servers 数据模型（backend/devsys_portal 的 Server），并新增 transport 字段。
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

fn default_auth() -> String {
    "password".into()
}
fn default_transport() -> String {
    "direct".into()
}
fn default_source() -> String {
    "mine".into()
}

// 一台服务器的拓扑与连接方式。凭据（密码/私钥）不在这里，存 OS keychain（见 creds.rs）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Server {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub jump: Option<String>,
    #[serde(default)]
    pub username: String,
    #[serde(default = "default_auth")]
    pub auth: String, // "password" | "key"
    // 可达性：direct（LAN/VPN 直连）| jump（经 ProxyJump）| tailnet（经内嵌 tailscaled SOCKS）。
    #[serde(default = "default_transport")]
    pub transport: String,
    // 来源：mine（我自己加的）| team:<名>（团队 config 给的，本地只读）。
    // 呼应「按来源合并」：一份列表里区分自持节点与团队共享节点。
    #[serde(default = "default_source")]
    pub source: String,
    // 我把这台机贡献给了哪些团队（team:<名>）。与 source 正交：source=从哪来，shared_to=给谁。
    #[serde(default)]
    pub shared_to: Vec<String>,
    // 是否已配凭据（标记位，避免启动时读钥匙串弹窗；真实密钥在 keychain）。
    #[serde(default)]
    pub has_secret: bool,
}

fn default_port() -> u16 {
    22
}

fn path(dir: &PathBuf) -> PathBuf {
    dir.join("servers.json")
}

// 读取全部服务器；文件不存在时返回空列表（首次启动）。
pub fn load(dir: &PathBuf) -> Vec<Server> {
    let p = path(dir);
    let Ok(text) = fs::read_to_string(&p) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

// 整表落盘（indent + 保留 UTF-8）。调用方已在 Mutex 内串行化。
pub fn save(dir: &PathBuf, servers: &[Server]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(servers).map_err(|e| e.to_string())?;
    fs::write(path(dir), text).map_err(|e| e.to_string())
}

// 增或改（按 name 唯一）。返回更新后的整表。
// 编辑既有机器时保留其 has_secret 标记（前端表单不带该字段，勿被默认 false 覆盖）。
pub fn upsert(dir: &PathBuf, mut s: Server) -> Result<Vec<Server>, String> {
    let mut list = load(dir);
    if s.source.is_empty() {
        s.source = default_source();
    }
    match list.iter_mut().find(|x| x.name == s.name) {
        Some(existing) => {
            s.has_secret = existing.has_secret;
            s.source = existing.source.clone(); // 来源由团队 config 决定，编辑不改
            s.shared_to = existing.shared_to.clone(); // 贡献状态由 share/unshare 改，表单编辑不动
            *existing = s;
        }
        None => list.push(s),
    }
    save(dir, &list)?;
    Ok(list)
}

// 给某机的 shared_to 加/去一个团队标记（贡献/撤销共享时用）。返回更新后的整表。
// 幂等：先去重再按 on 决定是否加入，重复调用不会堆叠。
pub fn set_shared(dir: &PathBuf, name: &str, team: &str, on: bool) -> Result<Vec<Server>, String> {
    let mut list = load(dir);
    if let Some(s) = list.iter_mut().find(|x| x.name == name) {
        s.shared_to.retain(|t| t != team);
        if on {
            s.shared_to.push(team.to_string());
        }
    }
    save(dir, &list)?;
    Ok(list)
}

// 删除；同时清掉任何把它当 jump 的悬空引用（置空该字段、转回 direct）。
pub fn remove(dir: &PathBuf, name: &str) -> Result<Vec<Server>, String> {
    let mut list = load(dir);
    list.retain(|x| x.name != name);
    for x in list.iter_mut() {
        if x.jump.as_deref() == Some(name) {
            x.jump = None;
            if x.transport == "jump" {
                x.transport = "direct".into();
            }
        }
    }
    save(dir, &list)?;
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srv(name: &str) -> Server {
        Server {
            name: name.into(),
            host: "1.2.3.4".into(),
            port: 22,
            jump: None,
            username: "me".into(),
            auth: "password".into(),
            transport: "direct".into(),
            source: "mine".into(),
            shared_to: vec![],
            has_secret: false,
        }
    }

    // 每个测试用独立临时目录，避免并行串扰。
    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("devsys_store_test_{tag}"));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn upsert_preserves_shared_and_source() {
        let dir = tmp("preserve");
        let mut s = srv("gpu");
        s.has_secret = true;
        upsert(&dir, s).unwrap();
        set_shared(&dir, "gpu", "team:neuroai", true).unwrap();

        // 表单式编辑（不带 shared_to/source/has_secret）应保留这些标记。
        let mut edit = srv("gpu");
        edit.host = "9.9.9.9".into();
        edit.source = "".into(); // 模拟前端不传
        let list = upsert(&dir, edit).unwrap();
        let g = list.iter().find(|x| x.name == "gpu").unwrap();
        assert_eq!(g.host, "9.9.9.9");
        assert_eq!(g.source, "mine");
        assert_eq!(g.shared_to, vec!["team:neuroai".to_string()]);
        assert!(g.has_secret);
    }

    #[test]
    fn set_shared_is_idempotent() {
        let dir = tmp("idem");
        upsert(&dir, srv("gpu")).unwrap();
        set_shared(&dir, "gpu", "team:a", true).unwrap();
        set_shared(&dir, "gpu", "team:a", true).unwrap(); // 重复不堆叠
        let list = set_shared(&dir, "gpu", "team:b", true).unwrap();
        let g = list.iter().find(|x| x.name == "gpu").unwrap();
        assert_eq!(g.shared_to, vec!["team:a".to_string(), "team:b".to_string()]);

        let list = set_shared(&dir, "gpu", "team:a", false).unwrap(); // 撤销 a
        let g = list.iter().find(|x| x.name == "gpu").unwrap();
        assert_eq!(g.shared_to, vec!["team:b".to_string()]);
    }
}
