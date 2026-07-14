// 团队配置（team.yaml）读写。团队 = 一份共享配置:成员公钥 + 共享机器(拓扑 + 开放档位)。
// 呼应「配置即代码」:消费侧只读加载(团队机作为只读节点合并进 servers);
// 贡献侧把自己的机器拓扑写进 team.yaml(共享拓扑不共享凭据,连接时各自用自己的钥匙)。
// 核心逻辑是纯函数(new/upsert/remove),便于单测;lib.rs 命令只做文件 IO 编排。
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TeamMember {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pubkey: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TeamMachine {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    // 开放档位:0 纯跳板 / 1 受限计算账号(默认) / 2 完全信任。授权落地时才据此配权限。
    #[serde(default = "default_tier")]
    pub tier: u8,
}

fn default_port() -> u16 {
    22
}
fn default_transport() -> String {
    "direct".into()
}
fn default_tier() -> u8 {
    1
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TeamConfig {
    pub team: String,
    #[serde(default)]
    pub members: Vec<TeamMember>,
    #[serde(default)]
    pub machines: Vec<TeamMachine>,
}

// ── 解析 / 序列化 ─────────────────────────────────────────

// 解析 team.yaml 文本。团队名为空视为无效(用于给条目打 source=team:<名>)。
pub fn parse(text: &str) -> Result<TeamConfig, String> {
    let cfg: TeamConfig = serde_yaml::from_str(text).map_err(|e| format!("team.yaml 解析失败: {e}"))?;
    if cfg.team.trim().is_empty() {
        return Err("team.yaml 缺少 team 字段(团队名)".into());
    }
    Ok(cfg)
}

pub fn to_yaml(cfg: &TeamConfig) -> Result<String, String> {
    serde_yaml::to_string(cfg).map_err(|e| format!("team.yaml 序列化失败: {e}"))
}

// ── 纯操作(可单测,无 IO) ─────────────────────────────────

// 新建一份团队配置,创建者作为首个成员。
pub fn new_config(team: &str, creator: TeamMember) -> TeamConfig {
    TeamConfig {
        team: team.trim().to_string(),
        members: vec![creator],
        machines: vec![],
    }
}

// 加/更新一台共享机(按 name 唯一)。
pub fn upsert_machine(cfg: &mut TeamConfig, m: TeamMachine) {
    match cfg.machines.iter_mut().find(|x| x.name == m.name) {
        Some(e) => *e = m,
        None => cfg.machines.push(m),
    }
}

// 撤销共享一台机;返回是否确实移除了。
pub fn remove_machine(cfg: &mut TeamConfig, name: &str) -> bool {
    let before = cfg.machines.len();
    cfg.machines.retain(|x| x.name != name);
    cfg.machines.len() != before
}

// 加/更新一个成员(按 name 唯一)。
pub fn upsert_member(cfg: &mut TeamConfig, m: TeamMember) {
    match cfg.members.iter_mut().find(|x| x.name == m.name) {
        Some(e) => *e = m,
        None => cfg.members.push(m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine(name: &str, host: &str) -> TeamMachine {
        TeamMachine {
            name: name.into(),
            host: host.into(),
            port: 22,
            jump: None,
            username: String::new(),
            transport: "direct".into(),
            tier: 1,
        }
    }

    #[test]
    fn parses_minimal() {
        let cfg = parse(
            r#"
team: neuroai
members:
  - name: alice
    pubkey: ssh-ed25519 AAAA
machines:
  - name: a100
    host: a100.neuroai.ts.net
    transport: tailnet
    tier: 1
  - name: lab-store
    host: 10.0.0.20
    port: 2222
"#,
        )
        .unwrap();
        assert_eq!(cfg.team, "neuroai");
        assert_eq!(cfg.members.len(), 1);
        assert_eq!(cfg.machines.len(), 2);
        assert_eq!(cfg.machines[0].transport, "tailnet");
        assert_eq!(cfg.machines[1].port, 2222);
        assert_eq!(cfg.machines[1].tier, 1); // 默认档
    }

    #[test]
    fn rejects_no_team_name() {
        assert!(parse("machines: []").is_err());
    }

    #[test]
    fn new_config_has_creator() {
        let cfg = new_config("  neuroai  ", TeamMember { name: "alice".into(), pubkey: "K".into() });
        assert_eq!(cfg.team, "neuroai"); // trim
        assert_eq!(cfg.members.len(), 1);
        assert_eq!(cfg.machines.len(), 0);
    }

    #[test]
    fn upsert_machine_dedupes_by_name() {
        let mut cfg = new_config("t", TeamMember { name: "a".into(), pubkey: String::new() });
        upsert_machine(&mut cfg, machine("gpu", "1.1.1.1"));
        upsert_machine(&mut cfg, machine("gpu", "2.2.2.2")); // 同名 → 更新非新增
        assert_eq!(cfg.machines.len(), 1);
        assert_eq!(cfg.machines[0].host, "2.2.2.2");
    }

    #[test]
    fn remove_machine_reports() {
        let mut cfg = new_config("t", TeamMember { name: "a".into(), pubkey: String::new() });
        upsert_machine(&mut cfg, machine("gpu", "1.1.1.1"));
        assert!(remove_machine(&mut cfg, "gpu"));
        assert!(!remove_machine(&mut cfg, "gpu")); // 已不在
        assert_eq!(cfg.machines.len(), 0);
    }

    #[test]
    fn upsert_member_dedupes() {
        let mut cfg = new_config("t", TeamMember { name: "a".into(), pubkey: "old".into() });
        upsert_member(&mut cfg, TeamMember { name: "a".into(), pubkey: "new".into() });
        upsert_member(&mut cfg, TeamMember { name: "b".into(), pubkey: "kb".into() });
        assert_eq!(cfg.members.len(), 2);
        assert_eq!(cfg.members[0].pubkey, "new");
    }

    #[test]
    fn yaml_round_trips() {
        let mut cfg = new_config("neuroai", TeamMember { name: "alice".into(), pubkey: "K".into() });
        upsert_machine(&mut cfg, TeamMachine {
            name: "gpu".into(), host: "10.0.0.5".into(), port: 2222,
            jump: Some("bastion".into()), username: "alice".into(),
            transport: "jump".into(), tier: 2,
        });
        let text = to_yaml(&cfg).unwrap();
        let back = parse(&text).unwrap();
        assert_eq!(back.team, "neuroai");
        assert_eq!(back.machines.len(), 1);
        assert_eq!(back.machines[0].port, 2222);
        assert_eq!(back.machines[0].jump.as_deref(), Some("bastion"));
        assert_eq!(back.machines[0].tier, 2);
    }
}
