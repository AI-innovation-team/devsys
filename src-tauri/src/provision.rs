// 授权下发:把成员的公钥,按「角色 × 机器 grant」算出的档位,真正装进被共享机。
//
// 这是「共享」从"拓扑可见"变成"队友真能登进去"的那一步(v1 过渡方案,不等 tailnet sidecar)。
// 终态会换成 Tailscale SSH + ACL(见 acl.rs),那时 authorized_keys 这套退役。
//
// RBAC:同一台机,不同角色不同档 —— core 成员拿 sudo、member 只受限、guest 只借跳板。
// 每个成员的实际档位 = 该机对他角色开的 grant。于是**一台机的下发脚本里,不同人不同权限**。
//   档 0 → 不建账号(纯跳板) · 档 1 → 独立账号无 sudo · 档 2 → 独立账号 + sudo
//
// **安全**:成员名与公钥拼进以 root 运行的脚本 —— 严格校验,拒绝一切可逃逸字符,不做转义兜底。
use serde::Serialize;

use crate::team::TeamView;

// 合法 unix 用户名:字母/下划线开头,后跟字母数字/下划线/连字符,≤32。
fn valid_user(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 32 {
        return false;
    }
    let first = b[0];
    if !(first.is_ascii_lowercase() || first == b'_') {
        return false;
    }
    b.iter()
        .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

// 合法 SSH 公钥:<type> <base64> [comment]。拒绝换行/引号/反斜杠等一切可逃逸字符。
fn valid_pubkey(s: &str) -> bool {
    if s.is_empty() || s.len() > 4096 {
        return false;
    }
    // 任何控制字符、单双引号、反斜杠、反引号、$ 都不该出现在公钥里。
    if s.chars().any(|c| {
        c.is_control() || matches!(c, '\'' | '"' | '\\' | '`' | '$' | '\n' | '\r')
    }) {
        return false;
    }
    let mut it = s.split_whitespace();
    let (Some(kind), Some(body)) = (it.next(), it.next()) else {
        return false;
    };
    let known = [
        "ssh-ed25519",
        "ssh-rsa",
        "ssh-dss",
        "ecdsa-sha2-nistp256",
        "ecdsa-sha2-nistp384",
        "ecdsa-sha2-nistp521",
        "sk-ssh-ed25519@openssh.com",
        "sk-ecdsa-sha2-nistp256@openssh.com",
    ];
    if !known.contains(&kind) {
        return false;
    }
    // base64 主体
    !body.is_empty()
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
}

// 一个将被建立的账号（身份到人）及其从 RBAC 算出的档位。
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct Account {
    pub name: String,
    pub role: String,
    pub tier: u8,
    pub sudo: bool,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ProvisionPlan {
    pub server: String,
    pub accounts: Vec<Account>, // 每个可登入成员一条（含各自档位/是否 sudo）
    pub any_sudo: bool,
    pub script: String,
    pub warnings: Vec<String>,
}

// 为一台机生成下发脚本。档位对每个成员分别算 = 该机对其角色开的 grant。
// 校验失败(非法用户名/公钥)直接报错 —— 宁可拒绝，也不把可疑输入送进 root 脚本。
pub fn plan(view: &TeamView, server: &str) -> Result<ProvisionPlan, String> {
    let machine = view
        .machines
        .iter()
        .find(|m| m.name == server)
        .ok_or_else(|| format!("团队里没有机器 {server}"))?;

    let mut warnings = Vec::new();
    let mut accounts = Vec::new();
    let mut blocks = Vec::new();

    for mem in &view.members {
        let tier = view.effective_tier(machine, mem); // = grants[mem.role]
        let Some(tier) = tier else {
            continue; // 该角色未获授权 → 不建账号
        };
        if tier == 0 {
            continue; // 纯跳板：不给 shell
        }
        if !valid_user(&mem.name) {
            return Err(format!(
                "成员名 {:?} 不是合法的 unix 用户名（小写字母/数字/_/-，字母或_开头，≤32）",
                mem.name
            ));
        }
        if mem.pubkey.trim().is_empty() {
            warnings.push(format!("成员 {} 没有公钥 —— 已跳过，他将无法登入。", mem.name));
            continue;
        }
        let key = mem.pubkey.trim();
        if !valid_pubkey(key) {
            return Err(format!("成员 {} 的公钥格式不合法（或含危险字符），已拒绝下发。", mem.name));
        }
        let sudo = tier >= 2;
        accounts.push(Account {
            name: mem.name.clone(),
            role: mem.role.clone(),
            tier,
            sudo,
        });

        // 幂等:账号已存在则不动;公钥已在则不重复追加。
        blocks.push(format!(
            r#"
# ── {name}（{role}，档 {tier}）──────────────
if id -u '{name}' >/dev/null 2>&1; then
  echo "  账号 {name} 已存在"
else
  useradd -m -s /bin/bash '{name}' && echo "  已建账号 {name}"
fi
install -d -m 700 -o '{name}' -g '{name}' "/home/{name}/.ssh"
touch "/home/{name}/.ssh/authorized_keys"
if grep -qxF '{key}' "/home/{name}/.ssh/authorized_keys"; then
  echo "  公钥已在 {name}"
else
  printf '%s\n' '{key}' >> "/home/{name}/.ssh/authorized_keys" && echo "  已装公钥 {name}"
fi
chmod 600 "/home/{name}/.ssh/authorized_keys"
chown -R '{name}':'{name}' "/home/{name}/.ssh"
{sudo_block}"#,
            name = mem.name,
            role = mem.role,
            tier = tier,
            key = key,
            sudo_block = if sudo {
                format!(
                    "printf '%s ALL=(ALL) NOPASSWD:ALL\\n' '{n}' > /etc/sudoers.d/devsys-{n}\nchmod 440 /etc/sudoers.d/devsys-{n}\necho \"  已给 {n} sudo（档 2）\"\n",
                    n = mem.name
                )
            } else {
                // 档 1:确保没有 sudo（清掉我们可能留下的旧授权）。
                format!(
                    "rm -f /etc/sudoers.d/devsys-{n}\necho \"  {n} 无 sudo（档 1）\"\n",
                    n = mem.name
                )
            },
        ));
    }

    let any_sudo = accounts.iter().any(|a| a.sudo);
    if accounts.is_empty() {
        warnings.push("没有任何可下发的账号 —— 要么无成员获此机授权，要么成员缺公钥。队友仍登不进来。".into());
    }
    if any_sudo {
        warnings.push("有成员获 **sudo**（档 2，完全信任）—— 仅限核心成员，请确认。".into());
    }
    warnings.push(
        "档 1（受限）只保证「无 sudo + 独立账号」。资源限额（GPU/CPU/内存）与目录隔离仍需你自己配（cgroup / 容器）—— 这层我们替不了。"
            .into(),
    );

    let script = format!(
        r#"#!/bin/sh
# DevSys 授权下发 —— 服务器 {server}
# 由 app 生成，需以 root 执行。幂等：可重复运行。
# 身份到人 + RBAC：每位成员一个独立账号，权限按其角色定（core=sudo / member=受限）。
set -e
if [ "$(id -u)" -ne 0 ]; then echo "需要 root（请用 sudo 运行）" >&2; exit 1; fi
echo "下发到 {server}："
{blocks}
echo "完成。"
"#,
        server = server,
        blocks = blocks.join("")
    );

    Ok(ProvisionPlan {
        server: server.into(),
        accounts,
        any_sudo,
        script,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{merge, new_member_file, upsert_machine, Machine, TeamRoot};
    use std::collections::BTreeMap;

    const KEY_A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample alice@mac";

    fn root() -> TeamRoot {
        crate::team::parse_root("team: neuroai\n").unwrap()
    }

    // 用一台机 gpu(grants) + 一组成员(name,pubkey,role) 构造视图。
    fn view(grants: &[(&str, u8)], members: &[(&str, &str, &str)]) -> TeamView {
        let g: BTreeMap<String, u8> = grants.iter().map(|(r, t)| (r.to_string(), *t)).collect();
        // 机器挂在第一个成员名下
        let owner = members.first().map(|m| m.0).unwrap_or("owner");
        let mut files = Vec::new();
        for (i, (n, k, role)) in members.iter().enumerate() {
            let mut f = new_member_file(n, k, role);
            if i == 0 {
                upsert_machine(&mut f, Machine {
                    name: "gpu".into(), host: "10.0.0.1".into(), port: 22, jump: None,
                    username: String::new(), transport: "direct".into(), grants: g.clone(),
                });
            }
            let _ = owner;
            files.push(f);
        }
        merge(&root(), &files)
    }

    #[test]
    fn guest_grant0_gets_no_account() {
        // gpu 对 guest 开 0；dan 是 guest → 不建账号
        let v = view(&[("guest", 0)], &[("dan", KEY_A, "guest")]);
        let p = plan(&v, "gpu").unwrap();
        assert!(p.accounts.is_empty());
        assert!(!p.script.contains("useradd"));
    }

    #[test]
    fn member_grant1_account_without_sudo() {
        let v = view(&[("member", 1)], &[("alice", KEY_A, "member")]);
        let p = plan(&v, "gpu").unwrap();
        assert_eq!(p.accounts.len(), 1);
        assert_eq!(p.accounts[0].tier, 1);
        assert!(!p.accounts[0].sudo);
        assert!(p.script.contains("useradd -m -s /bin/bash 'alice'"));
        assert!(p.script.contains(KEY_A));
        assert!(p.script.contains("rm -f /etc/sudoers.d/devsys-alice"));
        assert!(!p.script.contains("NOPASSWD"));
    }

    #[test]
    fn core_grant2_gets_sudo() {
        let v = view(&[("core", 2)], &[("bob", KEY_A, "core")]);
        let p = plan(&v, "gpu").unwrap();
        assert!(p.accounts[0].sudo);
        assert!(p.any_sudo);
        assert!(p.script.contains("NOPASSWD:ALL"));
        assert!(p.warnings.iter().any(|w| w.contains("sudo")));
    }

    // ★ RBAC 核心：同一台机，不同角色不同权限
    #[test]
    fn same_machine_different_roles_different_tiers() {
        let v = view(
            &[("core", 2), ("member", 1), ("guest", 0)],
            &[("alice", KEY_A, "core"), ("bob", KEY_A, "member"), ("dan", KEY_A, "guest")],
        );
        let p = plan(&v, "gpu").unwrap();
        // alice(core)→sudo, bob(member)→无sudo, dan(guest)→无账号
        let acct = |n: &str| p.accounts.iter().find(|a| a.name == n);
        assert!(acct("alice").unwrap().sudo, "core 拿 sudo");
        assert!(!acct("bob").unwrap().sudo, "member 无 sudo");
        assert!(acct("dan").is_none(), "guest(档0) 不建账号");
        assert!(p.script.contains("useradd -m -s /bin/bash 'alice'"));
        assert!(p.script.contains("useradd -m -s /bin/bash 'bob'"));
    }

    #[test]
    fn script_is_idempotent() {
        let v = view(&[("member", 1)], &[("alice", KEY_A, "member")]);
        let p = plan(&v, "gpu").unwrap();
        assert!(p.script.contains("id -u 'alice'"));
        assert!(p.script.contains("grep -qxF"));
    }

    // ── 注入防御（安全关键）──────────────────────────────
    #[test]
    fn rejects_shell_injection_in_username() {
        for bad in ["alice'; rm -rf /;'", "root ALL", "a b", "Alice", "1alice", "a".repeat(33).as_str()] {
            let v = view(&[("member", 1)], &[(bad, KEY_A, "member")]);
            assert!(plan(&v, "gpu").is_err(), "应拒绝: {bad}");
        }
    }

    #[test]
    fn rejects_injection_in_pubkey() {
        let bads = [
            "ssh-ed25519 AAAA'; rm -rf / ;'",
            "ssh-ed25519 AAAA\nroot ALL=(ALL) NOPASSWD",
            "ssh-ed25519 AAAA`whoami`",
            "ssh-ed25519 AAAA$(id)",
            "not-a-key AAAA",
            "ssh-ed25519",
        ];
        for bad in bads {
            let v = view(&[("member", 1)], &[("alice", bad, "member")]);
            assert!(plan(&v, "gpu").is_err(), "应拒绝: {bad:?}");
        }
    }

    #[test]
    fn member_without_pubkey_is_skipped_with_warning() {
        let v = view(&[("member", 1)], &[("alice", "", "member"), ("bob", KEY_A, "member")]);
        let p = plan(&v, "gpu").unwrap();
        assert_eq!(p.accounts.len(), 1);
        assert_eq!(p.accounts[0].name, "bob");
        assert!(p.warnings.iter().any(|w| w.contains("alice") && w.contains("没有公钥")));
    }
}
