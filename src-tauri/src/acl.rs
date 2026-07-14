// tier 档位 → Tailscale ACL policy 的「编译器」。
//
// 这是我们在整个栈里真正独有的那一层:把「团队要怎么共享」的**意图**(tier 档位),
// 编译成底层现成工具(Tailscale)的配置。底层机制一律不自研 —— 同 SkyPilot / 沙箱的定位。
//
// 两层要分清(Tailscale 只管前者):
//   门禁层 = 谁能进、以哪个 unix 用户进、要不要二次确认  ← 本文件产出
//   屋内层 = 进来后能干什么(受限账号 / cgroup / 容器)   ← 机器主人的 OS 配置，另一回事
//
// 档位语义:
//   tier 0 纯跳板   → 只放转发(不给 shell):不生成 ssh 规则，只给 tag 打上可达
//   tier 1 受限账号 → 以「各人自己的账号」登入(autogroup:nonroot)，action=accept
//   tier 2 完全信任 → 同上但 action=check(每次高权限访问要 SSO 二次确认 = 轻量审批)
//
// 生成的是 policy **片段**(grants + tagOwners + ssh)，供人 review 后并进团队 policy。
// 我们不代替用户直接改他的 tailnet —— 责任为门:机器主人自己拍板、自己贴。
use serde::Serialize;

use crate::team::TeamConfig;

// 一台共享机在 tailnet 里的 tag(按团队 + 档位分池，便于 policy 里整池授权)。
pub fn tag_for(team: &str, tier: u8) -> String {
    format!("tag:{}-tier{}", slug(team), tier)
}

// 团队名 → tailscale tag 安全字符(小写字母数字与连字符)。
fn slug(s: &str) -> String {
    let out: String = s
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "team".into() } else { out }
}

#[derive(Serialize, Debug, PartialEq)]
pub struct SshRule {
    pub action: String,            // accept | check
    pub src: Vec<String>,          // 谁能发起 = 团队 group
    pub dst: Vec<String>,          // 连到哪些机 = 该档位的 tag 池
    pub users: Vec<String>,        // 允许登入为哪个 unix 用户
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_period: Option<String>, // action=check 时的二次确认有效期
}

// 一台机的部署提示：主人需要在这台机上做什么(tailscale 命令 + 屋内层责任)。
#[derive(Serialize, Debug, PartialEq)]
pub struct MachinePlan {
    pub name: String,
    pub host: String,
    pub tier: u8,
    pub tag: String,
    pub command: String,  // 在这台机上跑的 tailscale 命令
    pub hardening: String, // 屋内层(OS/容器)责任提示 —— Tailscale 不管这层
}

#[derive(Serialize, Debug, PartialEq)]
pub struct AclPlan {
    pub team: String,
    pub group: String,              // group:<team>
    pub members: Vec<String>,       // 团队成员(进 group)
    pub tag_owners: Vec<String>,    // 需要声明的 tag(tagOwners 段)
    pub ssh: Vec<SshRule>,          // policy 的 ssh 段
    pub machines: Vec<MachinePlan>, // 每台机的落地动作
    pub notes: Vec<String>,         // 必须让人看到的边界与警告
}

// 把一份 team.yaml 编译成 Tailscale ACL 计划。纯函数:不碰网络、不碰文件。
pub fn compile(cfg: &TeamConfig) -> AclPlan {
    let team = slug(&cfg.team);
    let group = format!("group:{}", team);

    // 按档位聚合:同档位的机器共用一个 tag 池，一条规则整池授权。
    let mut tiers: Vec<u8> = cfg.machines.iter().map(|m| m.tier).collect();
    tiers.sort_unstable();
    tiers.dedup();

    let mut ssh = Vec::new();
    let mut tag_owners = Vec::new();
    for &t in &tiers {
        let tag = tag_for(&cfg.team, t);
        tag_owners.push(tag.clone());
        // tier 0 = 纯跳板:只借道，不给 shell → 不生成 ssh 规则。
        if t == 0 {
            continue;
        }
        ssh.push(SshRule {
            // tier 2 = 完全信任 → check(SSO 二次确认，轻量审批)；tier 1 → accept。
            action: if t >= 2 { "check" } else { "accept" }.into(),
            src: vec![group.clone()],
            dst: vec![tag],
            // 身份到人:登入为发起者自己的 unix 账号，不用共享账号 —— 追踪链不能断。
            users: vec!["autogroup:nonroot".into()],
            check_period: if t >= 2 { Some("12h".into()) } else { None },
        });
    }

    let machines = cfg
        .machines
        .iter()
        .map(|m| {
            let tag = tag_for(&cfg.team, m.tier);
            let command = if m.tier == 0 {
                format!("tailscale up --advertise-tags={tag}") // 纯跳板:不开 --ssh
            } else {
                format!("tailscale up --ssh --advertise-tags={tag}")
            };
            let hardening = match m.tier {
                0 => "纯跳板:不开 SSH server，只借道转发".into(),
                1 => "为每位成员建独立账号(无 sudo)+ cgroup 限额；或把 shell 关进容器".into(),
                _ => "完全信任(有 sudo):仅限核心成员；建议配合 session recording".into(),
            };
            MachinePlan {
                name: m.name.clone(),
                host: m.host.clone(),
                tier: m.tier,
                tag,
                command,
                hardening,
            }
        })
        .collect();

    let mut notes = vec![
        "Tailscale 只管「门禁」(谁能进、以谁的身份进)；进来后能干什么由这台机的 OS 决定 —— 受限账号 / cgroup / 容器要机器主人自己配。".into(),
        "身份到人:登入为各人自己的账号(autogroup:nonroot)，不用共享账号 —— 否则操作追踪链会断。".into(),
        "这是 policy 片段，请 review 后并进团队 tailnet policy；app 不会替你改 tailnet。".into(),
    ];
    if cfg.machines.iter().any(|m| m.tier >= 2) {
        notes.push("有 tier 2(完全信任、可 sudo)的机器 —— 已用 action=check(12h SSO 二次确认)，请确认这是你想开的档。".into());
    }
    if cfg.members.is_empty() {
        notes.push("team.yaml 里没有成员 —— group 为空，没人能连进来。".into());
    }

    AclPlan {
        team: cfg.team.clone(),
        group,
        members: cfg.members.iter().map(|m| m.name.clone()).collect(),
        tag_owners,
        ssh,
        machines,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{TeamMachine, TeamMember};

    fn cfg(tiers: &[u8]) -> TeamConfig {
        TeamConfig {
            team: "NeuroAI Lab".into(), // 带空格与大写，测 slug
            members: vec![TeamMember { name: "alice".into(), pubkey: "K".into() }],
            machines: tiers
                .iter()
                .enumerate()
                .map(|(i, &t)| TeamMachine {
                    name: format!("m{i}"),
                    host: format!("10.0.0.{i}"),
                    port: 22,
                    jump: None,
                    username: String::new(),
                    transport: "tailnet".into(),
                    tier: t,
                })
                .collect(),
        }
    }

    #[test]
    fn slugs_team_name_into_safe_tag() {
        assert_eq!(tag_for("NeuroAI Lab", 1), "tag:neuroai-lab-tier1");
        assert_eq!(tag_for("!!!", 1), "tag:team-tier1"); // 全非法字符 → 兜底
    }

    #[test]
    fn tier1_is_accept_tier2_is_check() {
        let plan = compile(&cfg(&[1, 2]));
        assert_eq!(plan.group, "group:neuroai-lab");
        assert_eq!(plan.ssh.len(), 2);

        let t1 = plan.ssh.iter().find(|r| r.dst[0].ends_with("tier1")).unwrap();
        assert_eq!(t1.action, "accept");
        assert_eq!(t1.check_period, None);
        assert_eq!(t1.users, vec!["autogroup:nonroot".to_string()]); // 身份到人

        let t2 = plan.ssh.iter().find(|r| r.dst[0].ends_with("tier2")).unwrap();
        assert_eq!(t2.action, "check"); // 高权限 → 二次确认
        assert_eq!(t2.check_period.as_deref(), Some("12h"));
    }

    #[test]
    fn tier0_gets_no_ssh_rule_but_still_tagged() {
        let plan = compile(&cfg(&[0]));
        assert!(plan.ssh.is_empty(), "纯跳板不该给 shell");
        assert_eq!(plan.tag_owners, vec!["tag:neuroai-lab-tier0".to_string()]);
        assert!(!plan.machines[0].command.contains("--ssh"), "纯跳板不开 SSH server");
    }

    #[test]
    fn same_tier_machines_share_one_rule() {
        let plan = compile(&cfg(&[1, 1, 1]));
        assert_eq!(plan.ssh.len(), 1, "同档位机器共用一条规则(整池授权)");
        assert_eq!(plan.machines.len(), 3);
        assert!(plan.machines.iter().all(|m| m.tag == "tag:neuroai-lab-tier1"));
    }

    #[test]
    fn warns_on_tier2_and_empty_members() {
        let plan = compile(&cfg(&[2]));
        assert!(plan.notes.iter().any(|n| n.contains("tier 2")));

        let mut c = cfg(&[1]);
        c.members.clear();
        let plan = compile(&c);
        assert!(plan.notes.iter().any(|n| n.contains("没有成员")));
    }

    #[test]
    fn machine_plan_carries_hardening_duty() {
        let plan = compile(&cfg(&[1]));
        // 屋内层责任必须显式告知机器主人 —— Tailscale 不管这层。
        assert!(plan.machines[0].hardening.contains("无 sudo"));
        assert!(plan.notes.iter().any(|n| n.contains("门禁")));
    }
}
