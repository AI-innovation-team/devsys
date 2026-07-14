// 授权下发:把 team.yaml 里成员的公钥,按 tier 档位真正装进被共享机。
//
// 这是「共享」从"拓扑可见"变成"队友真能登进去"的那一步(v1 过渡方案,不等 tailnet sidecar)。
// 终态会换成 Tailscale SSH + ACL(见 acl.rs),那时 authorized_keys 这套退役。
//
// 档位如何兑现(门禁 + 屋内 一起做):
//   tier 0 纯跳板   → 不建任何账号、不装任何公钥(只借道转发)
//   tier 1 受限计算 → 每位成员一个**独立账号**(身份到人)、**无 sudo**、装其公钥
//   tier 2 完全信任 → 同上 + sudo(仅核心成员)
//
// **安全**:成员名与公钥来自 team.yaml,会被拼进以 root 运行的脚本 —— 必须严格校验,
// 拒绝一切可能逃逸出 shell 单引号的输入。校验不通过 = 拒绝生成脚本(不做转义兜底)。
use serde::Serialize;

use crate::team::TeamConfig;

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

#[derive(Serialize, Debug, PartialEq)]
pub struct ProvisionPlan {
    pub server: String,
    pub tier: u8,
    pub accounts: Vec<String>, // 将建立/更新的账号(= 团队成员，身份到人)
    pub sudo: bool,            // tier 2 才给
    pub script: String,        // 要在被共享机上以 root 执行的脚本
    pub warnings: Vec<String>,
}

// 为一台机生成下发脚本。cfg 提供成员与公钥;tier 决定开多大权。
// 校验失败(非法用户名/公钥)直接报错 —— 宁可拒绝，也不把可疑输入送进 root 脚本。
pub fn plan(cfg: &TeamConfig, server: &str, tier: u8) -> Result<ProvisionPlan, String> {
    let mut warnings = Vec::new();

    // tier 0 = 纯跳板:不给 shell，不建账号。
    if tier == 0 {
        return Ok(ProvisionPlan {
            server: server.into(),
            tier,
            accounts: vec![],
            sudo: false,
            script: "# 档 0（纯跳板）：不建账号、不装公钥，只借道转发。无需下发。\n".into(),
            warnings: vec!["档 0 只借道：队友无法在这台机上取得 shell。".into()],
        });
    }

    let sudo = tier >= 2;
    let mut accounts = Vec::new();
    let mut blocks = Vec::new();

    for m in &cfg.members {
        if !valid_user(&m.name) {
            return Err(format!(
                "成员名 {:?} 不是合法的 unix 用户名（小写字母/数字/_/-，字母或_开头，≤32）",
                m.name
            ));
        }
        if m.pubkey.trim().is_empty() {
            warnings.push(format!("成员 {} 没有公钥 —— 已跳过，他将无法登入。", m.name));
            continue;
        }
        let key = m.pubkey.trim();
        if !valid_pubkey(key) {
            return Err(format!("成员 {} 的公钥格式不合法（或含危险字符），已拒绝下发。", m.name));
        }
        accounts.push(m.name.clone());

        // 幂等:账号已存在则不动;公钥已在则不重复追加。
        blocks.push(format!(
            r#"
# ── {name} ──────────────────────────────
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
            name = m.name,
            key = key,
            sudo_block = if sudo {
                format!(
                    "printf '%s ALL=(ALL) NOPASSWD:ALL\\n' '{n}' > /etc/sudoers.d/devsys-{n}\nchmod 440 /etc/sudoers.d/devsys-{n}\necho \"  已给 {n} sudo（档 2）\"\n",
                    n = m.name
                )
            } else {
                // 档 1:确保没有 sudo（清掉我们可能留下的旧授权）。
                format!(
                    "rm -f /etc/sudoers.d/devsys-{n}\necho \"  {n} 无 sudo（档 1）\"\n",
                    n = m.name
                )
            },
        ));
    }

    if accounts.is_empty() {
        warnings.push("没有任何可下发的成员公钥 —— 队友仍然登不进来。".into());
    }
    if sudo {
        warnings.push("档 2 会给这些账号 **sudo**（完全信任）—— 仅限核心成员，请确认。".into());
    } else {
        warnings.push(
            "档 1 只保证「无 sudo + 独立账号」。资源限额（GPU/CPU/内存）与目录隔离仍需你自己配（cgroup / 容器）—— 这层我们替不了。"
                .into(),
        );
    }

    let script = format!(
        r#"#!/bin/sh
# DevSys 授权下发 —— 服务器 {server}（档 {tier}）
# 由 app 生成，需以 root 执行。幂等：可重复运行。
# 身份到人：每位成员一个独立账号，不用共享账号（否则操作追踪链会断）。
set -e
if [ "$(id -u)" -ne 0 ]; then echo "需要 root（请用 sudo 运行）" >&2; exit 1; fi
echo "下发到 {server}（档 {tier}）："
{blocks}
echo "完成。"
"#,
        server = server,
        tier = tier,
        blocks = blocks.join("")
    );

    Ok(ProvisionPlan {
        server: server.into(),
        tier,
        accounts,
        sudo,
        script,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{TeamMachine, TeamMember};

    const KEY_A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample alice@mac";

    fn cfg(members: &[(&str, &str)]) -> TeamConfig {
        TeamConfig {
            team: "neuroai".into(),
            members: members
                .iter()
                .map(|(n, k)| TeamMember { name: (*n).into(), pubkey: (*k).into() })
                .collect(),
            machines: vec![TeamMachine {
                name: "gpu".into(), host: "10.0.0.1".into(), port: 22,
                jump: None, username: String::new(), transport: "direct".into(), tier: 1,
            }],
        }
    }

    #[test]
    fn tier0_provisions_nothing() {
        let p = plan(&cfg(&[("alice", KEY_A)]), "gpu", 0).unwrap();
        assert!(p.accounts.is_empty());
        assert!(!p.script.contains("useradd"), "纯跳板不该建账号");
        assert!(!p.sudo);
    }

    #[test]
    fn tier1_creates_account_without_sudo() {
        let p = plan(&cfg(&[("alice", KEY_A)]), "gpu", 1).unwrap();
        assert_eq!(p.accounts, vec!["alice".to_string()]);
        assert!(!p.sudo);
        assert!(p.script.contains("useradd -m -s /bin/bash 'alice'"));
        assert!(p.script.contains(KEY_A));
        assert!(p.script.contains("rm -f /etc/sudoers.d/devsys-alice"), "档1 必须确保无 sudo");
        assert!(!p.script.contains("NOPASSWD"), "档1 绝不能给 sudo");
        assert!(p.warnings.iter().any(|w| w.contains("cgroup")), "须提示屋内层责任");
    }

    #[test]
    fn tier2_grants_sudo_and_warns() {
        let p = plan(&cfg(&[("bob", KEY_A)]), "gpu", 2).unwrap();
        assert!(p.sudo);
        assert!(p.script.contains("NOPASSWD:ALL"));
        assert!(p.warnings.iter().any(|w| w.contains("sudo")));
    }

    #[test]
    fn script_is_idempotent() {
        let p = plan(&cfg(&[("alice", KEY_A)]), "gpu", 1).unwrap();
        assert!(p.script.contains("id -u 'alice'"), "账号已存在则不重建");
        assert!(p.script.contains("grep -qxF"), "公钥已在则不重复追加");
    }

    #[test]
    fn per_member_accounts_not_shared() {
        let p = plan(&cfg(&[("alice", KEY_A), ("bob", KEY_A)]), "gpu", 1).unwrap();
        assert_eq!(p.accounts, vec!["alice".to_string(), "bob".to_string()]);
        // 身份到人：两个独立账号，不是一个共享号。
        assert!(p.script.contains("useradd -m -s /bin/bash 'alice'"));
        assert!(p.script.contains("useradd -m -s /bin/bash 'bob'"));
    }

    // ── 注入防御（安全关键）──────────────────────────────
    #[test]
    fn rejects_shell_injection_in_username() {
        for bad in ["alice'; rm -rf /;'", "root ALL", "a b", "Alice", "1alice", "a".repeat(33).as_str()] {
            assert!(plan(&cfg(&[(bad, KEY_A)]), "gpu", 1).is_err(), "应拒绝: {bad}");
        }
    }

    #[test]
    fn rejects_injection_in_pubkey() {
        let bads = [
            "ssh-ed25519 AAAA'; rm -rf / ;'",           // 单引号逃逸
            "ssh-ed25519 AAAA\nroot ALL=(ALL) NOPASSWD", // 换行注入第二行
            "ssh-ed25519 AAAA`whoami`",                  // 命令替换
            "ssh-ed25519 AAAA$(id)",                     // 命令替换
            "not-a-key AAAA",                            // 未知类型
            "ssh-ed25519",                               // 缺主体
        ];
        for bad in bads {
            assert!(plan(&cfg(&[("alice", bad)]), "gpu", 1).is_err(), "应拒绝: {bad:?}");
        }
    }

    #[test]
    fn member_without_pubkey_is_skipped_with_warning() {
        let p = plan(&cfg(&[("alice", ""), ("bob", KEY_A)]), "gpu", 1).unwrap();
        assert_eq!(p.accounts, vec!["bob".to_string()]);
        assert!(p.warnings.iter().any(|w| w.contains("alice") && w.contains("没有公钥")));
    }
}
