// 共享闭环的全链路集成测试（不含真 SSH 那一跳 —— 那需要真机）。
//
// 新模型（分文件 + RBAC）下走一遍真实剧情：
//   alice 建团队（team.yaml 角色）→ 写 members/alice.yaml，贡献 GPU（grants: core→2, member→1）
//   git 推 → bob 克隆 → 加载合并视图 → alice 的机器作为只读节点进列表
//   bob 写 members/bob.yaml（role=member）→ alice 拉取
//   alice 为 GPU 生成授权脚本：alice(core)→sudo、bob(member)→无 sudo —— 同机不同权
//
// 守住的不变量：凭据不进任何 yaml；身份到人；团队机只读；RBAC 按角色算权限。
#![cfg(test)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{acl, github, provision, store, team};

const ALICE_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAlice alice@mac";
const BOB_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBob bob@thinkpad";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(dir).args(args).output().expect("git 不可用");
    assert!(out.status.success(), "git {:?} 失败: {}", args, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn fresh(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("devsys-e2e-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

// 读该团队目录下全部 members/*.yaml。
fn load_members(dir: &Path) -> Vec<team::MemberFile> {
    let mdir = dir.join("members");
    let Ok(rd) = std::fs::read_dir(&mdir) else { return vec![] };
    let mut v: Vec<team::MemberFile> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "yaml"))
        .filter_map(|e| team::parse_member(&std::fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    v.sort_by(|a, b| a.member.name.cmp(&b.member.name));
    v
}

fn write_member(dir: &Path, mf: &team::MemberFile) {
    let mdir = dir.join("members");
    std::fs::create_dir_all(&mdir).unwrap();
    std::fs::write(mdir.join(format!("{}.yaml", mf.member.name)), team::member_to_yaml(mf).unwrap()).unwrap();
}

fn view_of(dir: &Path) -> team::TeamView {
    let root = team::parse_root(&std::fs::read_to_string(dir.join("team.yaml")).unwrap()).unwrap();
    team::merge(&root, &load_members(dir))
}

#[test]
fn full_sharing_loop() {
    let rootdir = fresh("loop");

    // 团队 git 真源（裸仓库）
    let bare = rootdir.join("team.git");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "--bare", "-q"]);

    // ── alice ──
    let arepo = rootdir.join("alice/team");
    std::fs::create_dir_all(arepo.parent().unwrap()).unwrap();
    git(&rootdir, &["clone", "-q", bare.to_str().unwrap(), arepo.to_str().unwrap()]);
    git(&arepo, &["config", "user.email", "a@t.t"]);
    git(&arepo, &["config", "user.name", "alice"]);

    // 1) 建团队：team.yaml（角色）+ members/alice.yaml
    let root = team::new_root("neuroai");
    std::fs::write(arepo.join("team.yaml"), team::root_to_yaml(&root).unwrap()).unwrap();
    let mut alice = team::new_member_file("alice", ALICE_KEY, "core");

    // 2) 贡献 GPU：core→2, member→1, pub→0（RBAC）
    team::upsert_machine(&mut alice, team::Machine {
        name: "gpu-01".into(), host: "192.168.1.10".into(), port: 22, jump: None,
        username: String::new(), transport: "direct".into(),
        grants: BTreeMap::from([("core".into(), 2), ("member".into(), 1), ("pub".into(), 0)]),
        advertises: vec![],
    });
    write_member(&arepo, &alice);

    // ★ 铁律：任何 yaml 都不含凭据
    for f in ["team.yaml", "members/alice.yaml"] {
        let text = std::fs::read_to_string(arepo.join(f)).unwrap();
        for bad in ["PRIVATE KEY", "password:", "hunter2", "secret:"] {
            assert!(!text.contains(bad), "{f} 疑似含凭据: {bad}");
        }
    }

    // 3) 推给团队
    git(&arepo, &["add", "-A"]);
    git(&arepo, &["commit", "-qm", "team + alice shares gpu"]);
    git(&arepo, &["push", "-q", "origin", "HEAD:master"]);

    // ── bob ──
    let brepo = rootdir.join("bob/team");
    std::fs::create_dir_all(brepo.parent().unwrap()).unwrap();
    git(&rootdir, &["clone", "-q", bare.to_str().unwrap(), brepo.to_str().unwrap()]);

    // 4) bob 加载合并视图 → alice 的机器只读进列表
    let bview = view_of(&brepo);
    assert_eq!(bview.team, "neuroai");
    assert_eq!(bview.machines.len(), 1);
    assert_eq!(bview.machines[0].owner, "alice", "记录了贡献者");

    let bstore = rootdir.join("bob/appdata");
    let src = format!("team:{}", bview.team);
    let m = &bview.machines[0];
    store::upsert(&bstore, store::Server {
        name: m.name.clone(), host: m.host.clone(), port: m.port, jump: m.jump.clone(),
        username: String::new(), auth: "password".into(), transport: m.transport.clone(),
        source: src.clone(), shared_to: vec![], has_secret: false,
    }).unwrap();
    let gpu = store::load(&bstore).into_iter().find(|s| s.name == "gpu-01").unwrap();
    assert_eq!(gpu.source, src, "团队机来源 = team:<名>（UI 据此只读）");
    assert!(!gpu.has_secret, "凭据是 bob 自己的事");

    // 5) bob 写自己的成员档（role=member），推回
    let bob = team::new_member_file("bob", BOB_KEY, "member");
    write_member(&brepo, &bob);
    git(&brepo, &["config", "user.email", "b@t.t"]);
    git(&brepo, &["config", "user.name", "bob"]);
    git(&brepo, &["add", "-A"]);
    git(&brepo, &["commit", "-qm", "bob joins as member"]);
    git(&brepo, &["push", "-q", "origin", "HEAD:master"]);

    // 6) alice 拉取 → 两人
    git(&arepo, &["pull", "-q", "--ff-only", "origin", "master"]);
    let view = view_of(&arepo);
    let names: Vec<&str> = view.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["alice", "bob"]);

    // 7) 授权脚本：alice(core)→sudo, bob(member)→无 sudo —— ★ 同机不同权
    let plan = provision::plan(&view, "gpu-01").unwrap();
    let acct = |n: &str| plan.accounts.iter().find(|a| a.name == n);
    assert!(acct("alice").unwrap().sudo, "core 拿 sudo");
    assert!(!acct("bob").unwrap().sudo, "member 无 sudo");
    assert!(plan.script.contains("useradd -m -s /bin/bash 'alice'"));
    assert!(plan.script.contains("useradd -m -s /bin/bash 'bob'"));
    assert!(plan.script.contains(BOB_KEY) && plan.script.contains(ALICE_KEY));

    // 8) 同一视图编译 ACL：core 用 check、member 用 accept
    let a = acl::compile(&view);
    let core = a.ssh.iter().find(|r| r.role == "core").unwrap();
    let member = a.ssh.iter().find(|r| r.role == "member").unwrap();
    assert_eq!(core.action, "check");
    assert_eq!(member.action, "accept");
}

// 跳板链：共享走跳板的机器时，跳板本身也得在配置里（否则队友连不上）。
#[test]
fn shared_machine_with_jump_needs_its_jump() {
    let root = team::new_root("neuroai");
    let mut alice = team::new_member_file("alice", ALICE_KEY, "core");
    // 跳板（对所有角色档 0：只借道）
    team::upsert_machine(&mut alice, team::Machine {
        name: "alice-mac".into(), host: "100.64.0.5".into(), port: 22, jump: None,
        username: String::new(), transport: "tailnet".into(),
        grants: BTreeMap::from([("core".into(), 0), ("member".into(), 0), ("pub".into(), 0)]),
        advertises: vec![],
    });
    // 内网 GPU 经跳板到达
    team::upsert_machine(&mut alice, team::Machine {
        name: "gpu-inner".into(), host: "192.168.1.50".into(), port: 22,
        jump: Some("alice-mac".into()), username: String::new(), transport: "jump".into(),
        grants: BTreeMap::from([("core".into(), 2), ("member".into(), 1)]),
        advertises: vec![],
    });
    let view = team::merge(&root, &[alice]);

    // 每台机引用的跳板必须也在 machines 里
    let names: Vec<&str> = view.machines.iter().map(|m| m.name.as_str()).collect();
    for m in &view.machines {
        if let Some(j) = &m.jump {
            assert!(names.contains(&j.as_str()), "{} 的跳板 {} 缺失", m.name, j);
        }
    }
    // 跳板全 grant=0 → 授权脚本不建账号，ACL 不开 --ssh
    let p = provision::plan(&view, "alice-mac").unwrap();
    assert!(p.accounts.is_empty());
    let a = acl::compile(&view);
    let jm = a.machines.iter().find(|m| m.name == "alice-mac").unwrap();
    assert!(!jm.command.contains("--ssh"));
}

// GitHub 花名册端到端：绑定 org 声明 → 缓存花名册 → merge+fold → 视图被 org 成员填满。
// 这是 UI「绑定并同步」背后的完整胶水（不含网络那一跳：网络已单独实测）。
#[test]
fn github_roster_folds_into_view() {
    let dir = fresh("gh-fold");
    // team.yaml 绑定 org（core-team→core，其余→member）。角色现为纯名字列表。
    let team_yaml = "team: neuroai\nroles: [core, member, pub]\ngithub:\n  org: neuroai-lab\n  role_map:\n    core-team: core\n    '*': member\n".to_string();
    std::fs::write(dir.join("team.yaml"), &team_yaml).unwrap();
    let root = team::parse_root(&team_yaml).unwrap();
    assert!(root.github.is_some(), "绑定被解析出来");

    // 模拟同步：把拉到的花名册缓存到本地（sync_github 干的事）
    let roster = vec![
        github::GhMember { login: "alice".into(), pubkeys: vec!["ssh-ed25519 KA".into()], role: "core".into() },
        github::GhMember { login: "bob".into(), pubkeys: vec!["ssh-ed25519 KB".into()], role: "member".into() },
        github::GhMember { login: "carol".into(), pubkeys: vec![], role: "member".into() }, // 没配公钥
    ];
    std::fs::write(dir.join(".github-roster.json"), serde_json::to_string(&roster).unwrap()).unwrap();

    // load_view 等价：merge（无手写成员）+ fold 缓存花名册
    let mut view = team::merge(&root, &[]);
    let cache: Vec<github::GhMember> =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".github-roster.json")).unwrap()).unwrap();
    team::fold_github(&mut view, &cache);

    // org 三人全进来，角色/公钥/身份到位
    assert_eq!(view.members.len(), 3);
    let by = |n: &str| view.members.iter().find(|m| m.name == n).unwrap();
    assert_eq!(by("alice").role, "core");
    assert_eq!(by("alice").identity, "alice"); // GitHub login = 身份锚
    assert_eq!(by("alice").pubkey, "ssh-ed25519 KA");
    assert_eq!(by("bob").role, "member");
    assert_eq!(by("carol").pubkey, "", "没配公钥的成员：空，UI 会提示");

    // 授权下发：有公钥的 org 成员按角色建账号（carol 无公钥 → 跳过并警告）
    let mut alice = team::new_member_file("alice", ALICE_KEY, "core");
    team::upsert_machine(&mut alice, team::Machine {
        name: "gpu".into(), host: "10.0.0.1".into(), port: 22, jump: None,
        username: String::new(), transport: "direct".into(),
        grants: BTreeMap::from([("core".into(), 2), ("member".into(), 1)]),
        advertises: vec![],
    });
    let mut v2 = team::merge(&root, &[alice]);
    team::fold_github(&mut v2, &cache);
    let plan = provision::plan(&v2, "gpu").unwrap();
    // alice(core)→sudo, bob(member)→无sudo, carol 无公钥→跳过
    assert!(plan.accounts.iter().find(|a| a.name == "alice").unwrap().sudo);
    assert!(!plan.accounts.iter().find(|a| a.name == "bob").unwrap().sudo);
    assert!(plan.accounts.iter().all(|a| a.name != "carol"));
    assert!(plan.warnings.iter().any(|w| w.contains("carol") && w.contains("没有公钥")));
}
