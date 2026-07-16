// RBAC → Tailscale ACL policy 的「编译器」。
//
// 我们独有的那一层：把「团队怎么共享」的**意图**（角色 × 机器 grant）编译成
// 底层现成工具（Tailscale）的配置。底层机制不自研 —— 同 SkyPilot / 沙箱的定位。
//
// 两层分清（Tailscale 只管前者）：
//   门禁层 = 谁能进、以哪个 unix 用户进、要不要二次确认  ← 本文件产出
//   屋内层 = 进来后能干什么（受限账号 / cgroup / 容器）   ← 机器主人的 OS 配置
//
// RBAC 模型：每个角色一个 group（成员按角色归入）；每台机对某角色开的 grant 决定规则：
//   grant 0 → 纯跳板：不生成 ssh 规则（只借道）
//   grant 1 → accept，以各人自己账号登入（autogroup:nonroot）
//   grant 2 → check（每次 SSO 二次确认 = 轻量审批）
//
// 产出 policy 片段（groups + tagOwners + ssh），供人 review 后自己贴 —— 责任为门。
use serde::Serialize;

use crate::team::TeamView;

// 团队名 → tailscale 安全字符（小写字母数字与连字符）。
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

// 一台机在 tailnet 里的 tag（按机器命名，一机一 tag）。
pub fn tag_for(team: &str, machine: &str) -> String {
    format!("tag:{}-{}", slug(team), slug(machine))
}

// 一个角色的 group 名。
pub fn group_for(team: &str, role: &str) -> String {
    format!("group:{}-{}", slug(team), slug(role))
}

#[derive(Serialize, Debug, PartialEq)]
pub struct SshRule {
    pub action: String,   // accept | check
    pub src: Vec<String>, // 谁能发起 = 某角色 group
    pub dst: Vec<String>, // 连到哪台机 = 该机 tag
    pub users: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_period: Option<String>,
    pub role: String, // 这条规则对应哪个角色（给 UI 展示）
}

// 一台机的落地动作。
#[derive(Serialize, Debug, PartialEq)]
pub struct MachinePlan {
    pub name: String,
    pub host: String,
    pub tag: String,
    pub owner: String,
    pub command: String,   // 在这台机上跑的 tailscale 命令
    pub grants_desc: String, // 人读的授权摘要，如 "core→2, member→1"
    pub hardening: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct AclPlan {
    pub team: String,
    pub groups: Vec<(String, Vec<String>)>, // group 名 → 成员名（各角色）
    pub tag_owners: Vec<String>,
    pub ssh: Vec<SshRule>,
    pub machines: Vec<MachinePlan>,
    pub notes: Vec<String>,
}

// 把合并视图编译成 Tailscale ACL 计划。纯函数。
pub fn compile(view: &TeamView) -> AclPlan {
    // 每个角色一个 group（即使暂时没成员，也声明出来供规则引用）。
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for role in &view.roles {
        let mut names: Vec<String> = view
            .members_of_role(role)
            .iter()
            .map(|m| m.name.clone())
            .collect();
        names.sort();
        groups.push((group_for(&view.team, role), names));
    }

    let mut ssh = Vec::new();
    let mut tag_owners = Vec::new();
    let mut any_sudo = false;

    for m in &view.machines {
        let tag = tag_for(&view.team, &m.name);
        tag_owners.push(tag.clone());
        // 该机对每个角色开的 grant → 一条规则（档 0 不发 ssh）。稳定顺序。
        for role in view.machine_roles(m) {
            let t = *m.grants.get(&role).unwrap_or(&0);
            if t == 0 {
                continue;
            }
            if t >= 2 {
                any_sudo = true;
            }
            ssh.push(SshRule {
                action: if t >= 2 { "check" } else { "accept" }.into(),
                src: vec![group_for(&view.team, &role)],
                dst: vec![tag.clone()],
                users: vec!["autogroup:nonroot".into()],
                check_period: if t >= 2 { Some("12h".into()) } else { None },
                role,
            });
        }
    }

    let machines = view
        .machines
        .iter()
        .map(|m| {
            let tag = tag_for(&view.team, &m.name);
            // 有任一角色 grant>0 才开 --ssh；全是 0（纯跳板）则不开。
            let any_shell = m.grants.values().any(|&t| t > 0);
            let command = if any_shell {
                format!("tailscale up --ssh --advertise-tags={tag}")
            } else {
                format!("tailscale up --advertise-tags={tag}")
            };
            let max_tier = m.grants.values().copied().max().unwrap_or(0);
            let hardening = match max_tier {
                0 => "纯跳板：不开 SSH server，只借道转发".into(),
                1 => "为每位成员建独立账号（无 sudo）+ cgroup 限额；或把 shell 关进容器".into(),
                _ => "有角色获 sudo（档 2）：仅限核心成员；建议配合 session recording".into(),
            };
            let mut gd: Vec<String> = view
                .machine_roles(m)
                .iter()
                .map(|r| format!("{r}→{}", m.grants.get(r).unwrap_or(&0)))
                .collect();
            gd.sort();
            MachinePlan {
                name: m.name.clone(),
                host: m.host.clone(),
                tag,
                owner: m.owner.clone(),
                command,
                grants_desc: gd.join(", "),
                hardening,
            }
        })
        .collect();

    let mut notes = vec![
        "Tailscale 只管「门禁」（谁能进、以谁的身份进）；进来能干什么由这台机的 OS 决定 —— 受限账号 / cgroup / 容器要机器主人自己配。".into(),
        "身份到人：登入为各人自己的账号（autogroup:nonroot），不用共享账号 —— 否则操作追踪链会断。".into(),
        "这是 policy 片段，请 review 后并进团队 tailnet policy；app 不会替你改 tailnet。".into(),
    ];
    if any_sudo {
        notes.push("有机器对某角色开了 sudo（档 2）—— 已用 action=check（12h SSO 二次确认），请确认这是你想开的档。".into());
    }
    if view.members.is_empty() {
        notes.push("还没有成员 —— 所有 group 为空，没人能连进来。".into());
    }

    AclPlan {
        team: view.team.clone(),
        groups,
        tag_owners,
        ssh,
        machines,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{merge, new_member_file, upsert_machine, Machine};
    use std::collections::BTreeMap;

    fn root() -> crate::team::TeamRoot {
        crate::team::parse_root("team: NeuroAI Lab\n").unwrap() // 带空格大写测 slug
    }

    // alice=core 贡献 gpu(core:2, member:1, pub:0)；bob=member；dan=pub
    fn view() -> TeamView {
        let mut af = new_member_file("alice", "KA", "core");
        upsert_machine(&mut af, Machine {
            name: "gpu".into(), host: "10.0.0.1".into(), port: 22, jump: None,
            username: String::new(), transport: "tailnet".into(),
            grants: BTreeMap::from([("core".into(), 2), ("member".into(), 1), ("pub".into(), 0)]),
        });
        let bf = new_member_file("bob", "KB", "member");
        let df = new_member_file("dan", "KD", "pub");
        merge(&root(), &[af, bf, df])
    }

    #[test]
    fn slugs_names() {
        assert_eq!(tag_for("NeuroAI Lab", "gpu-01"), "tag:neuroai-lab-gpu-01");
        assert_eq!(group_for("NeuroAI Lab", "core"), "group:neuroai-lab-core");
    }

    #[test]
    fn rules_per_role_by_grant() {
        let p = compile(&view());
        // gpu 对 core=2(check) / member=1(accept) / pub=0(无规则)
        let core = p.ssh.iter().find(|r| r.role == "core").unwrap();
        assert_eq!(core.action, "check");
        assert_eq!(core.check_period.as_deref(), Some("12h"));
        assert_eq!(core.src, vec!["group:neuroai-lab-core".to_string()]);

        let member = p.ssh.iter().find(|r| r.role == "member").unwrap();
        assert_eq!(member.action, "accept");
        assert_eq!(member.check_period, None);

        // pub grant=0 → 无 ssh 规则
        assert!(p.ssh.iter().all(|r| r.role != "pub"));
    }

    #[test]
    fn groups_list_members_by_role() {
        let p = compile(&view());
        let core = p.groups.iter().find(|(g, _)| g.ends_with("-core")).unwrap();
        assert_eq!(core.1, vec!["alice".to_string()]);
        let member = p.groups.iter().find(|(g, _)| g.ends_with("-member")).unwrap();
        assert_eq!(member.1, vec!["bob".to_string()]);
    }

    #[test]
    fn machine_opens_ssh_since_some_grant_positive() {
        let p = compile(&view());
        let gpu = p.machines.iter().find(|m| m.name == "gpu").unwrap();
        assert!(gpu.command.contains("--ssh"));
        assert_eq!(gpu.owner, "alice");
        assert!(gpu.grants_desc.contains("core→2"));
    }

    #[test]
    fn pure_jump_machine_no_ssh() {
        let mut af = new_member_file("alice", "KA", "core");
        upsert_machine(&mut af, Machine {
            name: "bastion".into(), host: "1.1.1.1".into(), port: 22, jump: None,
            username: String::new(), transport: "tailnet".into(),
            grants: BTreeMap::from([("member".into(), 0), ("core".into(), 0)]),
        });
        let p = compile(&merge(&root(), &[af]));
        assert!(p.ssh.is_empty(), "全 grant=0 → 无 ssh 规则");
        assert!(!p.machines[0].command.contains("--ssh"));
    }

    #[test]
    fn warns_on_sudo() {
        let p = compile(&view());
        assert!(p.notes.iter().any(|n| n.contains("sudo")));
    }
}
