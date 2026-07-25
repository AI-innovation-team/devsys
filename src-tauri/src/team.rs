// 团队配置：两层 + 合并成图。呼应「配置即代码」与「责任为门」。
//
//   团队级  team.yaml            —— 角色名（core/member/pub）+ 可选 GitHub org 绑定。管理员维护。
//   个人级  members/<name>.yaml  —— 我是谁 + 我贡献的机器 + 每台机对哪个角色开哪个档（grants）。
//                                   只有本人改，git 永不冲突，git blame 即担责链。
//
// 加载时把 team.yaml + 所有 members/*.yaml 合并成 TeamView（统一视图）——
// 下游（provision / acl / 拓扑图）只认 TeamView。权限 = 成员角色 × 机器 grant 的交叉，算出来。
//
// 核心是纯函数（parse / merge / 交叉查表 / graph），便于单测；lib.rs 只做文件 IO 编排。
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ── 磁盘结构 ──────────────────────────────────────────────

// 团队级 team.yaml。对齐指南针后：
//   · 角色只是**名字**（人的分组），不带 tier —— tier 只属于「设备×角色」（machine.grants）。
//   · 成员从身份源**导出**（GitHub org），不手维护名单。
//   · 三层信任：org=私有 / federation=联邦（别人的 org）/ pub=公开公地。
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TeamRoot {
    pub team: String,
    // 角色名（纯标签，无 tier）。core=org 核心 / member=org 成员 / pub=org 外公开。
    #[serde(default = "default_roles")]
    pub roles: Vec<String>,
    // 绑定 GitHub org：成员/公钥/角色从 org 自动导出（github.role_map = GitHub team→角色）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<crate::github::GithubBinding>,
    // 团队 tailnet(可达性地基):管理员声明团队用哪张 tailnet(名/组织域),成员据此加入
    // 同一张网 —— 不在同一 tailnet,直连和经门都无从谈起。空=未声明(仅显示引导)。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tailnet: String,
    // 联邦：我信任的别的 org，其成员算「联邦成员」而非 pub。v2 实现，先占位。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub federation: Vec<String>,
}

// core=org 核心（从 GitHub core-team 导出）/ member=org 成员 / pub=org 外公开借用。
fn default_roles() -> Vec<String> {
    vec!["core".into(), "member".into(), "pub".into()]
}

// 个人级 members/<name>.yaml：我 + 我贡献的机器。
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct MemberFile {
    pub member: Member,
    #[serde(default)]
    pub machines: Vec<Machine>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Member {
    pub name: String, // unix 账号句柄（下发时建的账号名）
    // 验证过的身份 = 团队 IdP 的 SSO 登录名（邮箱/类邮箱）。不可伪造，是防冒名的锚。
    // 空 = 未验证（旧数据 / 未连 tailnet 时登记）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub identity: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pubkey: String,
    // 我在团队里的角色（引用 team.yaml 的 roles 键）。缺省 member。
    #[serde(default = "default_role")]
    pub role: String,
    // ★ 切面模型：我共享的「自己这台设备」= 人节点的算力切面（人本身就是一台算力）。
    // 与 machines（我贡献但「不是我」的服务器）区分。有 = 这个人在图里带算力环。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<Device>,
}

// 自身设备:人的算力切面。没有 name(名字就是这个人),其余同机器的「怎么到达 + 开档」。
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Device {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default = "default_grants")]
    pub grants: BTreeMap<String, u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advertises: Vec<String>,
    #[serde(default, skip_serializing_if = "Sharing::is_default")]
    pub sharing: Sharing,
}

fn default_role() -> String {
    "member".into()
}

// SSO 登录名（如 guohao2045@gmail.com）→ 合法 unix 账号句柄。
pub fn slug_login(login: &str) -> String {
    let local = login.split('@').next().unwrap_or(login);
    let mut out: String = local
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' })
        .collect();
    out = out.trim_matches('-').to_string();
    // 首字符必须是字母或下划线
    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() {
        out = format!("u{out}");
    }
    out.chars().take(32).collect()
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Machine {
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
    // ★ RBAC 落点：这台机对每个角色开的档位。空 = 未授权任何角色。
    // 缺省给 member=1（贡献时若没细分，至少让普通成员能用）。
    #[serde(default = "default_grants")]
    pub grants: BTreeMap<String, u8>,
    // 它广播的子网 CIDR（非空 = 这台是 subnet router / 网关，替整个网段当门）。
    // 校园那种「只能经它进内网」的场景:门是一等节点,网段内的机靠它可达。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advertises: Vec<String>,
    // 「档位怎么兑现」——裸机账号 or 一人一容器,以及主人的借出上限与点名共享的数据集。
    #[serde(default, skip_serializing_if = "Sharing::is_default")]
    pub sharing: Sharing,
}

// ★ 档位的**兑现方式**（v2）。「档」= 开多少权，「容器」= 怎么关，两者正交:
// grants 一字不改，只是从 `useradd` 换成 `docker run`。
//
// 主人的借出上限 `limit` 落成一个**父 cgroup 池**，所有借用容器挂它下面 ——
// cgroup 层级保证子容器加起来永不超父上限，开多少容器都突破不了（责任为门:随时可改/设 0 收回）。
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
pub struct Sharing {
    // account（默认，v1 裸机账号）| container（v2 一人一容器）
    #[serde(default = "default_isolation", skip_serializing_if = "is_account")]
    pub isolation: String,
    // 容器基础镜像。空 = 用 app 现建的 devsys/base（带 bash/tmux/git，保证工作区持久化可用）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    // 最多借出多少（父池上限）。None = 不限（不建父池）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<ShareLimit>,
    // 主人**点名**只读挂进来的数据集。没点名的宿主目录一律不可见（数据分层第②③层）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data: Vec<ShareData>,
}

impl Sharing {
    pub fn is_default(&self) -> bool {
        *self == Sharing::default() || (self.is_account() && self.image.is_empty() && self.limit.is_none() && self.data.is_empty())
    }
    pub fn is_account(&self) -> bool {
        self.isolation.is_empty() || self.isolation == "account"
    }
    pub fn is_container(&self) -> bool {
        self.isolation == "container"
    }
}

fn default_isolation() -> String {
    "account".into()
}
fn is_account(s: &String) -> bool {
    s.is_empty() || s == "account"
}

// 主人愿意借出的资源上限。字段全可选:只写你在意的那几项。
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
pub struct ShareLimit {
    // CPU 核数（可小数，如 7.5）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpus: Option<f64>,
    // 内存，docker 写法:"32g" / "4096m"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mem: String,
    // GPU，docker --gpus 写法:"all" / "2" / "device=0,1"。
    // 注意:GPU **不受 cgroup 父池约束**（是设备直通，不是可分配资源）——只能逐容器点名。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gpus: String,
}

// 主人点名共享的数据集:大数据集共用一份、不各拷，只读挂进每个借用容器。
// 「把计算带到数据旁」—— 数据不出本机，容器就在这台机上跑。
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
pub struct ShareData {
    pub host: String, // 宿主上的路径
    #[serde(rename = "as", default, skip_serializing_if = "String::is_empty")]
    pub mount_as: String, // 容器内路径（空 = 同 host）
    #[serde(default = "default_mode", skip_serializing_if = "is_ro")]
    pub mode: String, // ro（默认）| rw
}

fn default_mode() -> String {
    "ro".into()
}
fn is_ro(s: &String) -> bool {
    s.is_empty() || s == "ro"
}

fn default_port() -> u16 {
    22
}
fn default_transport() -> String {
    "direct".into()
}
fn default_grants() -> BTreeMap<String, u8> {
    BTreeMap::from([("member".into(), 1)])
}

// ── 合并视图（下游只认这个）──────────────────────────────

// 合并后的一台机：带上「谁贡献的」。
#[derive(Serialize, Clone, Debug)]
pub struct ViewMachine {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub jump: Option<String>,
    pub username: String,
    pub transport: String,
    pub grants: BTreeMap<String, u8>,
    pub owner: String, // 贡献者（来自哪个 members/*.yaml）
    // 这台是不是 owner 本人的设备（来自 member.device）。图里折进人节点;
    // acl/provision 一视同仁（自身设备也要下发/编 ACL，它就是一台算力）。
    pub is_self: bool,
    // 广播的子网 CIDR（非空 = subnet router / 网关）。图里标成「门」。
    pub advertises: Vec<String>,
    // 档位兑现方式 + 借出上限 + 点名共享的数据集（provision 据此建账号还是建容器）。
    pub sharing: Sharing,
}

#[derive(Serialize, Clone, Debug)]
pub struct ViewMember {
    pub name: String,
    pub identity: String, // 验证过的 SSO 身份（空=未验证）
    pub pubkey: String,
    pub role: String,
}

// team.yaml + 所有 members/*.yaml 合并出的统一视图。
#[derive(Serialize, Clone, Debug)]
pub struct TeamView {
    pub team: String,
    pub roles: Vec<String>, // 纯角色名（无 tier）
    #[serde(default)]
    pub tailnet: String,    // 团队声明的 tailnet(可达性地基)
    pub members: Vec<ViewMember>,
    pub machines: Vec<ViewMachine>,
}

impl TeamView {
    // 某成员对某台机的**实际档位** = 该机对成员角色开的 grant（无则 None = 无权）。
    pub fn effective_tier(&self, machine: &ViewMachine, member: &ViewMember) -> Option<u8> {
        machine.grants.get(&member.role).copied()
    }

    // 某台机授权了哪些角色（grant 存在即算，含档 0）。稳定排序。
    pub fn machine_roles(&self, machine: &ViewMachine) -> Vec<String> {
        let mut r: Vec<String> = machine.grants.keys().cloned().collect();
        r.sort();
        r
    }

    pub fn members_of_role(&self, role: &str) -> Vec<&ViewMember> {
        self.members.iter().filter(|m| m.role == role).collect()
    }
}

// ── 解析 / 合并 ──────────────────────────────────────────

pub fn parse_root(text: &str) -> Result<TeamRoot, String> {
    let root: TeamRoot = serde_yaml::from_str(text).map_err(|e| format!("team.yaml 解析失败: {e}"))?;
    if root.team.trim().is_empty() {
        return Err("team.yaml 缺少 team 字段（团队名）".into());
    }
    Ok(root)
}

pub fn parse_member(text: &str) -> Result<MemberFile, String> {
    let mf: MemberFile = serde_yaml::from_str(text).map_err(|e| format!("member 文件解析失败: {e}"))?;
    if mf.member.name.trim().is_empty() {
        return Err("member 文件缺少 member.name".into());
    }
    Ok(mf)
}

pub fn root_to_yaml(root: &TeamRoot) -> Result<String, String> {
    serde_yaml::to_string(root).map_err(|e| format!("序列化失败: {e}"))
}

pub fn member_to_yaml(mf: &MemberFile) -> Result<String, String> {
    serde_yaml::to_string(mf).map_err(|e| format!("序列化失败: {e}"))
}

// 合并：team 根 + 若干成员文件 → 统一视图。
// 未知角色（member.role 不在 roles 里）保留原样，交给校验层提示，不在此静默丢弃。
pub fn merge(root: &TeamRoot, members: &[MemberFile]) -> TeamView {
    let mut view_members = Vec::new();
    let mut view_machines = Vec::new();

    for mf in members {
        view_members.push(ViewMember {
            name: mf.member.name.clone(),
            identity: mf.member.identity.clone(),
            pubkey: mf.member.pubkey.clone(),
            role: mf.member.role.clone(),
        });
        // 自身设备（算力切面）→ 一台 is_self 机器,名字即人名。
        if let Some(d) = &mf.member.device {
            view_machines.push(ViewMachine {
                name: mf.member.name.clone(),
                host: d.host.clone(),
                port: d.port,
                jump: d.jump.clone(),
                username: d.username.clone(),
                transport: d.transport.clone(),
                grants: d.grants.clone(),
                owner: mf.member.name.clone(),
                is_self: true,
                advertises: d.advertises.clone(),
                sharing: d.sharing.clone(),
            });
        }
        // 贡献的服务器（管但不是本人）。
        for m in &mf.machines {
            view_machines.push(ViewMachine {
                name: m.name.clone(),
                host: m.host.clone(),
                port: m.port,
                jump: m.jump.clone(),
                username: m.username.clone(),
                transport: m.transport.clone(),
                grants: m.grants.clone(),
                owner: mf.member.name.clone(),
                is_self: false,
                advertises: m.advertises.clone(),
                sharing: m.sharing.clone(),
            });
        }
    }
    // 稳定排序（成员/机器按名），让图与授权计划输出确定。
    view_members.sort_by(|a, b| a.name.cmp(&b.name));
    view_machines.sort_by(|a, b| a.name.cmp(&b.name));

    TeamView {
        team: root.team.clone(),
        roles: root.roles.clone(),
        tailnet: root.tailnet.clone(),
        members: view_members,
        machines: view_machines,
    }
}

// 把 GitHub org 拉来的花名册折叠进视图：org 成员即团队成员（公钥/角色自动）。
// 与手写 members/*.yaml 合并：手写档若已存在同名成员，保留其（可能更细的）本地设置，
// 但补上从 GitHub 拉到的公钥（本地缺时）。GitHub 独有的成员按 login 派生账号名加入。
pub fn fold_github(view: &mut TeamView, gh: &[crate::github::GhMember]) {
    for m in gh {
        let name = slug_login(&m.login);
        let first_key = m.pubkeys.first().cloned().unwrap_or_default();
        if let Some(existing) = view.members.iter_mut().find(|v| v.name == name || v.identity == m.login) {
            // 手写档优先，但补公钥/身份。
            if existing.pubkey.is_empty() {
                existing.pubkey = first_key;
            }
            if existing.identity.is_empty() {
                existing.identity = m.login.clone();
            }
        } else {
            view.members.push(ViewMember {
                name,
                identity: m.login.clone(), // GitHub login 即验证身份锚
                pubkey: first_key,
                role: m.role.clone(),
            });
        }
    }
    view.members.sort_by(|a, b| a.name.cmp(&b.name));
    view.members.dedup_by(|a, b| a.name == b.name);
}

// ── 兼容旧的平表 team.yaml（迁移一版）─────────────────────
// 旧格式：{ team, members:[{name,pubkey}], machines:[{...,tier}] }。
// 迁移为：TeamRoot(默认角色) + 一个「合并的」成员文件（machines 的 tier → grants{member:tier}）。

#[derive(Deserialize)]
struct FlatOld {
    team: String,
    #[serde(default)]
    members: Vec<FlatMember>,
    #[serde(default)]
    machines: Vec<FlatMachine>,
}
#[derive(Deserialize)]
struct FlatMember {
    name: String,
    #[serde(default)]
    pubkey: String,
}
#[derive(Deserialize)]
struct FlatMachine {
    name: String,
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default)]
    jump: Option<String>,
    #[serde(default)]
    username: String,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    tier: u8,
}

// 探测并迁移旧平表；不是旧格式则返回 None。
pub fn migrate_flat(text: &str) -> Option<(TeamRoot, Vec<MemberFile>)> {
    // 新格式的成员文件顶层有 `member:`，团队根有 `roles:`；旧平表是顶层 `members:`+`machines:`。
    let old: FlatOld = serde_yaml::from_str(text).ok()?;
    if old.team.trim().is_empty() {
        return None;
    }
    let root = TeamRoot {
        team: old.team,
        roles: default_roles(),
        github: None,
        tailnet: String::new(),
        federation: vec![],
    };
    // 旧平表没有"谁贡献了哪台机"的归属信息 —— 全部归到第一个成员名下（迁移的近似）。
    let owner = old.members.first().map(|m| m.name.clone()).unwrap_or_else(|| "owner".into());
    let machines = old
        .machines
        .into_iter()
        .map(|m| Machine {
            name: m.name,
            host: m.host,
            port: m.port,
            jump: m.jump,
            username: m.username,
            transport: m.transport,
            grants: BTreeMap::from([("member".into(), m.tier.max(1).min(2))]),
            advertises: vec![],
            sharing: Default::default(),
        })
        .collect();
    // 每个旧成员成一份文件；机器挂在 owner 那份下。
    let mut files = Vec::new();
    for (i, mem) in old.members.iter().enumerate() {
        let _ = i;
        files.push(MemberFile {
            member: Member {
                name: mem.name.clone(),
                identity: String::new(), // 旧数据无验证身份
                pubkey: mem.pubkey.clone(),
                role: default_role(),
                device: None,
            },
            machines: vec![],
        });
    }
    if let Some(f) = files.iter_mut().find(|f| f.member.name == owner) {
        f.machines = machines;
    } else {
        files.push(MemberFile {
            member: Member { name: owner, identity: String::new(), pubkey: String::new(), role: default_role(), device: None },
            machines,
        });
    }
    Some((root, files))
}

// ── 纯操作（个人文件的增改；只动自己那份）───────────────

pub fn new_root(team: &str) -> TeamRoot {
    TeamRoot {
        team: team.trim().to_string(),
        roles: default_roles(),
        github: None,
        tailnet: String::new(),
        federation: vec![],
    }
}

pub fn new_member_file(name: &str, pubkey: &str, role: &str) -> MemberFile {
    MemberFile {
        member: Member {
            name: name.trim().into(),
            identity: String::new(),
            pubkey: pubkey.trim().into(),
            role: if role.is_empty() { default_role() } else { role.into() },
            device: None,
        },
        machines: vec![],
    }
}

pub fn upsert_machine(mf: &mut MemberFile, m: Machine) {
    match mf.machines.iter_mut().find(|x| x.name == m.name) {
        Some(e) => *e = m,
        None => mf.machines.push(m),
    }
}

pub fn remove_machine(mf: &mut MemberFile, name: &str) -> bool {
    let before = mf.machines.len();
    mf.machines.retain(|x| x.name != name);
    mf.machines.len() != before
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> TeamRoot {
        parse_root("team: neuroai\nroles: [core, member, pub]\n").unwrap()
    }

    #[test]
    fn roles_are_plain_names_no_tier() {
        let r = root();
        assert_eq!(r.roles, vec!["core", "member", "pub"]); // 无 tier，纯名字
    }

    #[test]
    fn fold_github_adds_and_enriches() {
        use crate::github::GhMember;
        // 本地手写档：alice(core, 无公钥)
        let af = new_member_file("alice", "", "core");
        let mut view = merge(&root(), &[af]);
        let gh = vec![
            GhMember { login: "alice".into(), pubkeys: vec!["ssh-ed25519 KA".into()], role: "member".into() },
            GhMember { login: "bob".into(), pubkeys: vec!["ssh-ed25519 KB".into()], role: "pub".into() },
        ];
        fold_github(&mut view, &gh);
        // alice：本地档保留（角色仍 core），但补上 GitHub 公钥
        let alice = view.members.iter().find(|m| m.name == "alice").unwrap();
        assert_eq!(alice.role, "core", "本地手写角色优先");
        assert_eq!(alice.pubkey, "ssh-ed25519 KA", "补上 GitHub 公钥");
        // bob：GitHub 独有 → 自动加入，角色来自 GitHub 映射（org 外 → pub）
        let bob = view.members.iter().find(|m| m.name == "bob").unwrap();
        assert_eq!(bob.role, "pub");
        assert_eq!(bob.identity, "bob");
        assert_eq!(view.members.len(), 2);
    }

    #[test]
    fn slug_login_makes_unix_name() {
        assert_eq!(slug_login("guohao2045@gmail.com"), "guohao2045");
        assert_eq!(slug_login("Alice.Smith@github"), "alice-smith");
        assert_eq!(slug_login("123@x.com"), "u123"); // 数字开头 → 前缀 u
        assert_eq!(slug_login("bob"), "bob");
    }

    #[test]
    fn default_roles_when_omitted() {
        let r = parse_root("team: neuroai\n").unwrap();
        assert_eq!(r.roles, vec!["core", "member", "pub"]); // 无 tier，纯名字
    }

    #[test]
    fn member_file_parses_with_grants() {
        let mf = parse_member(
            r#"
member:
  name: alice
  pubkey: ssh-ed25519 AAAA
  role: core
machines:
  - name: alice-gpu
    host: 100.0.0.5
    transport: tailnet
    grants:
      core: 2
      member: 1
      pub: 0
"#,
        )
        .unwrap();
        assert_eq!(mf.member.role, "core");
        assert_eq!(mf.machines[0].grants.get("core"), Some(&2));
        assert_eq!(mf.machines[0].grants.get("pub"), Some(&0));
    }

    #[test]
    fn member_role_defaults_to_member() {
        let mf = parse_member("member:\n  name: bob\n").unwrap();
        assert_eq!(mf.member.role, "member");
    }

    fn view3() -> TeamView {
        // alice=core 贡献 gpu(core:2,member:1,pub:0)；bob=member；dan=pub
        let mut af = new_member_file("alice", "KA", "core");
        upsert_machine(&mut af, Machine {
            name: "gpu".into(), host: "10.0.0.1".into(), port: 22, jump: None,
            username: String::new(), transport: "direct".into(),
            grants: BTreeMap::from([("core".into(), 2), ("member".into(), 1), ("pub".into(), 0)]),
            advertises: vec![],
            sharing: Default::default(),
        });
        let bf = new_member_file("bob", "KB", "member");
        let df = new_member_file("dan", "KD", "pub");
        merge(&root(), &[af, bf, df])
    }

    #[test]
    fn effective_tier_is_role_times_grant() {
        let v = view3();
        let gpu = v.machines.iter().find(|m| m.name == "gpu").unwrap().clone();
        let by = |n: &str| v.members.iter().find(|m| m.name == n).unwrap().clone();
        assert_eq!(v.effective_tier(&gpu, &by("alice")), Some(2)); // core
        assert_eq!(v.effective_tier(&gpu, &by("bob")), Some(1));   // member
        assert_eq!(v.effective_tier(&gpu, &by("dan")), Some(0));   // pub
    }

    #[test]
    fn device_folds_into_self_machine() {
        // alice 共享自己的设备(切面),另贡献一台服务器 gpu。
        let mut af = new_member_file("alice", "KA", "core");
        af.member.device = Some(Device {
            host: "100.64.0.11".into(), port: 22, jump: None,
            username: "alice".into(), transport: "tailnet".into(),
            grants: BTreeMap::from([("core".into(), 2), ("member".into(), 1)]),
            advertises: vec![],
            sharing: Default::default(),
        });
        upsert_machine(&mut af, Machine {
            name: "gpu".into(), host: "10.0.0.1".into(), port: 22, jump: None,
            username: String::new(), transport: "direct".into(),
            grants: BTreeMap::from([("core".into(), 2), ("member".into(), 1)]),
            advertises: vec![],
            sharing: Default::default(),
        });
        let v = merge(&root(), &[af]);
        // 两台算力节点:自身设备(is_self,名=alice)+ 服务器 gpu。
        assert_eq!(v.machines.len(), 2);
        let dev = v.machines.iter().find(|m| m.name == "alice").unwrap();
        assert!(dev.is_self, "自身设备标 is_self");
        assert_eq!(dev.owner, "alice");
        assert_eq!(dev.host, "100.64.0.11");
        let gpu = v.machines.iter().find(|m| m.name == "gpu").unwrap();
        assert!(!gpu.is_self, "贡献的服务器不是自身设备");
        // device 走 YAML 往返不丢。
        let back = parse_member(&member_to_yaml(&{
            let mut f = new_member_file("alice", "KA", "core");
            f.member.device = Some(Device { host: "1.2.3.4".into(), port: 22, jump: None, username: String::new(), transport: "tailnet".into(), grants: BTreeMap::from([("member".into(), 1)]), advertises: vec![], sharing: Default::default() });
            f
        }).unwrap()).unwrap();
        assert_eq!(back.member.device.unwrap().host, "1.2.3.4");
    }

    #[test]
    fn merge_records_owner() {
        let v = view3();
        let gpu = v.machines.iter().find(|m| m.name == "gpu").unwrap();
        assert_eq!(gpu.owner, "alice");
    }

    // 双命名空间对齐:成员档 name=slug_login(gh login)、roster 成员=gh login(可能带大写)。
    // fold 后必须还是同一个人(一个节点),不能裂成两个。身份锚统一为 GitHub 的关键回归。
    #[test]
    fn fold_github_aligns_slug_and_login_case() {
        let root = new_root("t");
        let mf = new_member_file(&slug_login("ColeHank"), "K-mine", "core"); // 本地档:colehank
        let mut view = merge(&root, &[mf]);
        assert_eq!(view.members.len(), 1);

        let gh = vec![crate::github::GhMember {
            login: "ColeHank".into(), // roster 保留大写
            pubkeys: vec!["K-gh".into()],
            role: "member".into(),
        }];
        fold_github(&mut view, &gh);
        assert_eq!(view.members.len(), 1, "同一个人不能裂成两个节点");
        let m = &view.members[0];
        assert_eq!(m.name, "colehank");
        assert_eq!(m.identity, "ColeHank"); // 空身份被补上 gh login
        assert_eq!(m.pubkey, "K-mine"); // 手写档的公钥优先
    }

    #[test]
    fn merge_sorts_stable() {
        let v = view3();
        let names: Vec<&str> = v.members.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["alice", "bob", "dan"]); // 排序确定
    }

    #[test]
    fn machine_ops_dedupe() {
        let mut f = new_member_file("alice", "K", "core");
        let m = |h: &str| Machine {
            name: "gpu".into(), host: h.into(), port: 22, jump: None,
            username: String::new(), transport: "direct".into(),
            grants: BTreeMap::from([("member".into(), 1)]),
            advertises: vec![],
            sharing: Default::default(),
        };
        upsert_machine(&mut f, m("1.1.1.1"));
        upsert_machine(&mut f, m("2.2.2.2"));
        assert_eq!(f.machines.len(), 1);
        assert_eq!(f.machines[0].host, "2.2.2.2");
        assert!(remove_machine(&mut f, "gpu"));
        assert!(!remove_machine(&mut f, "gpu"));
    }

    #[test]
    fn round_trips() {
        let mut f = new_member_file("alice", "KA", "core");
        upsert_machine(&mut f, Machine {
            name: "gpu".into(), host: "10.0.0.5".into(), port: 2222,
            jump: Some("bastion".into()), username: "alice".into(), transport: "jump".into(),
            grants: BTreeMap::from([("core".into(), 2), ("member".into(), 1)]),
            advertises: vec![],
            sharing: Default::default(),
        });
        let back = parse_member(&member_to_yaml(&f).unwrap()).unwrap();
        assert_eq!(back.machines[0].port, 2222);
        assert_eq!(back.machines[0].jump.as_deref(), Some("bastion"));
        assert_eq!(back.machines[0].grants.get("core"), Some(&2));

        let r = root();
        let rback = parse_root(&root_to_yaml(&r).unwrap()).unwrap();
        assert_eq!(rback.roles, vec!["core", "member", "pub"]);
    }

    #[test]
    fn migrate_flat_old_format() {
        let old = r#"
team: neuroai
members:
  - name: alice
    pubkey: KA
  - name: bob
    pubkey: KB
machines:
  - name: gpu
    host: 10.0.0.1
    tier: 2
"#;
        let (root, files) = migrate_flat(old).unwrap();
        assert_eq!(root.team, "neuroai");
        assert!(root.roles.contains(&"member".to_string()));
        // 两个成员各一份文件
        assert_eq!(files.len(), 2);
        // 机器归到第一个成员（owner），tier=2 → grants{member:2}
        let owner = files.iter().find(|f| f.member.name == "alice").unwrap();
        assert_eq!(owner.machines.len(), 1);
        assert_eq!(owner.machines[0].grants.get("member"), Some(&2));
        // 合并后视图可用
        let v = merge(&root, &files);
        assert_eq!(v.machines.len(), 1);
        assert_eq!(v.machines[0].owner, "alice");
    }
}
