// 共享闭环的全链路集成测试(不含真 SSH 那一跳 —— 那需要真机)。
//
// 走一遍真实剧情:
//   alice 建团队 → 贡献自己的 GPU 机(档 1)→ 推进 git 仓库
//   bob   克隆 → 加载 → alice 的机器作为**只读节点**进他的列表
//   bob   登记自己的公钥 → alice 为这台机生成授权脚本 → 脚本里有 bob 的独立账号与公钥
//
// 守住的不变量:
//   · 凭据(密码/私钥)一个字节都不进 team.yaml
//   · 身份到人:每个成员独立账号,没有共享号
//   · 团队机在消费者那侧是只读(source=team:<名>)
#![cfg(test)]

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{acl, provision, store, team};

const ALICE_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAlice alice@mac";
const BOB_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBob bob@thinkpad";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git 不可用");
    assert!(
        out.status.success(),
        "git {:?} 失败: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn fresh(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("devsys-e2e-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn full_sharing_loop() {
    let root = fresh("loop");

    // ── 团队的 git 真源(裸仓库,模拟 GitHub)────────────────
    let bare = root.join("team-config.git");
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "--bare", "-q"]);

    // ── alice 那侧 ────────────────────────────────────────
    let alice_repo = root.join("alice/team-config");
    std::fs::create_dir_all(alice_repo.parent().unwrap()).unwrap();
    git(
        root.as_path(),
        &["clone", "-q", bare.to_str().unwrap(), alice_repo.to_str().unwrap()],
    );
    git(&alice_repo, &["config", "user.email", "a@t.t"]);
    git(&alice_repo, &["config", "user.name", "alice"]);

    let alice_yaml = alice_repo.join("team.yaml");
    let ay = alice_yaml.to_str().unwrap();

    // 1) alice 建团队，自己是首个成员
    let mut cfg = team::new_config(
        "neuroai",
        team::TeamMember { name: "alice".into(), pubkey: ALICE_KEY.into() },
    );

    // 2) alice 贡献自己的 GPU 机（档 1 = 受限计算）—— 只给拓扑
    team::upsert_machine(
        &mut cfg,
        team::TeamMachine {
            name: "gpu-01".into(),
            host: "192.168.1.10".into(),
            port: 22,
            jump: None,
            username: String::new(),
            transport: "direct".into(),
            tier: 1,
        },
    );
    std::fs::write(ay, team::to_yaml(&cfg).unwrap()).unwrap();

    // ★ 铁律：凭据绝不进 team.yaml
    let yaml_text = std::fs::read_to_string(ay).unwrap();
    for forbidden in ["PRIVATE KEY", "password", "hunter2", "secret"] {
        assert!(
            !yaml_text.contains(forbidden),
            "team.yaml 里出现了疑似凭据的内容: {forbidden}"
        );
    }

    // 3) 推给团队
    git(&alice_repo, &["add", "team.yaml"]);
    git(&alice_repo, &["commit", "-qm", "share gpu-01"]);
    git(&alice_repo, &["push", "-q", "origin", "HEAD:master"]);

    // ── bob 那侧 ──────────────────────────────────────────
    let bob_repo = root.join("bob/team-config");
    std::fs::create_dir_all(bob_repo.parent().unwrap()).unwrap();
    git(
        root.as_path(),
        &["clone", "-q", bare.to_str().unwrap(), bob_repo.to_str().unwrap()],
    );
    let bob_yaml = bob_repo.join("team.yaml");
    let by = bob_yaml.to_str().unwrap();

    // 4) bob 加载 → alice 的机器进他的拓扑，且是**只读**（source=team:neuroai）
    let bob_cfg = team::parse(&std::fs::read_to_string(by).unwrap()).unwrap();
    assert_eq!(bob_cfg.team, "neuroai");
    assert_eq!(bob_cfg.machines.len(), 1);

    let bob_store = root.join("bob/appdata");
    let src = format!("team:{}", bob_cfg.team);
    let m = &bob_cfg.machines[0];
    store::upsert(
        &bob_store,
        store::Server {
            name: m.name.clone(),
            host: m.host.clone(),
            port: m.port,
            jump: m.jump.clone(),
            username: String::new(),
            auth: "password".into(),
            transport: m.transport.clone(),
            source: src.clone(),
            shared_to: vec![],
            has_secret: false,
        },
    )
    .unwrap();

    let bob_list = store::load(&bob_store);
    let gpu = bob_list.iter().find(|s| s.name == "gpu-01").expect("bob 应看到 gpu-01");
    assert_eq!(gpu.source, src, "团队机来源必须是 team:<名>（UI 据此只读渲染）");
    assert_eq!(gpu.host, "192.168.1.10", "拓扑（怎么到达）确实传过来了");
    assert!(!gpu.has_secret, "凭据是 bob 自己的事，不随共享而来");

    // 5) bob 登记自己的公钥，推回去
    let mut bob_cfg = bob_cfg;
    team::upsert_member(
        &mut bob_cfg,
        team::TeamMember { name: "bob".into(), pubkey: BOB_KEY.into() },
    );
    std::fs::write(by, team::to_yaml(&bob_cfg).unwrap()).unwrap();
    git(&bob_repo, &["config", "user.email", "b@t.t"]);
    git(&bob_repo, &["config", "user.name", "bob"]);
    git(&bob_repo, &["add", "team.yaml"]);
    git(&bob_repo, &["commit", "-qm", "join as bob"]);
    git(&bob_repo, &["push", "-q", "origin", "HEAD:master"]);

    // 6) alice 拉取 → 看到 bob
    git(&alice_repo, &["pull", "-q", "--ff-only", "origin", "master"]);
    let cfg = team::parse(&std::fs::read_to_string(ay).unwrap()).unwrap();
    let names: Vec<&str> = cfg.members.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["alice", "bob"], "团队现在有两人");

    // 7) alice 为 gpu-01 生成授权脚本 —— 这一步让 bob **真能登进去**
    let plan = provision::plan(&cfg, "gpu-01", 1).unwrap();

    // 身份到人：每人一个独立账号，不是共享号
    assert_eq!(plan.accounts, vec!["alice".to_string(), "bob".to_string()]);
    assert!(plan.script.contains("useradd -m -s /bin/bash 'bob'"));
    assert!(plan.script.contains(BOB_KEY), "bob 的公钥要装进去");
    assert!(plan.script.contains(ALICE_KEY));
    // 档 1：无 sudo
    assert!(!plan.sudo);
    assert!(!plan.script.contains("NOPASSWD"), "档 1 绝不给 sudo");

    // 8) 同一份 team.yaml 也能编译成 Tailscale ACL（终态方案）
    let acl = acl::compile(&cfg);
    assert_eq!(acl.group, "group:neuroai");
    assert_eq!(acl.ssh.len(), 1);
    assert_eq!(acl.ssh[0].action, "accept"); // 档 1
    assert_eq!(acl.ssh[0].users, vec!["autogroup:nonroot".to_string()]); // 身份到人
    assert_eq!(acl.machines[0].name, "gpu-01");
}

// 跳板链必须完整：共享一台走跳板的机器时，跳板本身也得在 team.yaml 里，
// 否则队友拿到的条目指向一个他没有的跳板 —— 连不上。
// （这里直接测 team.rs 层的不变量；lib.rs 的 share_server 命令按同一规则补链。）
#[test]
fn shared_machine_with_jump_needs_its_jump_in_config() {
    let mut cfg = team::new_config(
        "neuroai",
        team::TeamMember { name: "alice".into(), pubkey: ALICE_KEY.into() },
    );

    // alice 的笔记本是跳板（档 0：只借道），内网 GPU 经它到达
    team::upsert_machine(
        &mut cfg,
        team::TeamMachine {
            name: "alice-mac".into(), host: "100.64.0.5".into(), port: 22,
            jump: None, username: String::new(), transport: "tailnet".into(), tier: 0,
        },
    );
    team::upsert_machine(
        &mut cfg,
        team::TeamMachine {
            name: "gpu-inner".into(), host: "192.168.1.50".into(), port: 22,
            jump: Some("alice-mac".into()), username: String::new(),
            transport: "jump".into(), tier: 1,
        },
    );

    // 每台机引用的跳板，必须也在 machines 里 —— 否则队友解析不出这一跳
    let names: Vec<&str> = cfg.machines.iter().map(|m| m.name.as_str()).collect();
    for m in &cfg.machines {
        if let Some(j) = &m.jump {
            assert!(
                names.contains(&j.as_str()),
                "{} 的跳板 {} 不在 team.yaml 里 —— 队友连不上",
                m.name, j
            );
        }
    }

    // 跳板是档 0：只借道，不给 shell
    let jump = cfg.machines.iter().find(|m| m.name == "alice-mac").unwrap();
    assert_eq!(jump.tier, 0);
    let plan = provision::plan(&cfg, "alice-mac", 0).unwrap();
    assert!(plan.accounts.is_empty(), "纯跳板不该建账号");
    assert!(!plan.script.contains("useradd"));

    // ACL：档 0 不生成 ssh 规则，也不开 --ssh
    let a = acl::compile(&cfg);
    assert!(a.ssh.iter().all(|r| !r.dst[0].ends_with("tier0")), "档 0 不该有 ssh 规则");
    let jm = a.machines.iter().find(|m| m.name == "alice-mac").unwrap();
    assert!(!jm.command.contains("--ssh"));

    // 目标机档 1：队友能登进来跑计算
    let plan = provision::plan(&cfg, "gpu-inner", 1).unwrap();
    assert_eq!(plan.accounts, vec!["alice".to_string()]);
    assert!(!plan.sudo);
}

#[test]
fn tier2_machine_grants_sudo_end_to_end() {
    let dir = fresh("tier2");
    let mut cfg = team::new_config(
        "core",
        team::TeamMember { name: "alice".into(), pubkey: ALICE_KEY.into() },
    );
    team::upsert_member(&mut cfg, team::TeamMember { name: "bob".into(), pubkey: BOB_KEY.into() });
    team::upsert_machine(
        &mut cfg,
        team::TeamMachine {
            name: "trusted".into(), host: "10.1.1.1".into(), port: 22,
            jump: None, username: String::new(), transport: "direct".into(), tier: 2,
        },
    );
    let p = dir.join("team.yaml");
    std::fs::write(&p, team::to_yaml(&cfg).unwrap()).unwrap();

    // round-trip 后档位不丢
    let back = team::parse(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(back.machines[0].tier, 2);

    // 档 2 → 给 sudo，且必须警告
    let plan = provision::plan(&back, "trusted", 2).unwrap();
    assert!(plan.sudo);
    assert!(plan.script.contains("NOPASSWD:ALL"));
    assert!(plan.warnings.iter().any(|w| w.contains("sudo")));

    // 档 2 → ACL 用 check（SSO 二次确认 = 轻量审批）
    let acl = acl::compile(&back);
    assert_eq!(acl.ssh[0].action, "check");
    assert_eq!(acl.ssh[0].check_period.as_deref(), Some("12h"));
}
