// 授权下发:把成员的公钥,按「角色 × 机器 grant」算出的档位,真正装进被共享机。
//
// 这是「共享」从"拓扑可见"变成"队友真能登进去"的那一步。
// 终态会换成 Tailscale SSH + ACL(见 acl.rs),那时 authorized_keys 这套退役。
//
// RBAC:同一台机,不同角色不同档 —— core 成员拿 sudo、member 只受限、pub 只借跳板。
// 每个成员的实际档位 = 该机对他角色开的 grant。于是**一台机的下发脚本里,不同人不同权限**。
//
// ★「档」= 开多少权 与「怎么关」**正交**,后者由 machine.sharing.isolation 选:
//
//   account（v1，裸机账号）      container（v2，一人一容器）
//   ─────────────────────────    ────────────────────────────────────────────
//   档0  仅转发账号(无 shell)     同左（不给容器，只当门）
//   档1  独立账号，无 sudo         普通容器:cpus/mem/gpus 限额 + 独立 volume + 看不见宿主 FS
//   档2  独立账号 + sudo          特权容器 + 挂宿主 /host（等价宿主 root，仅核心）
//
// 容器模式下宿主账号退化成**门房**:sshd 的 ForceCommand 把这次登入直接丢进本人容器,
// 拿不到宿主 shell。于是「身份到人」原封不动(还是各自的账号+各自的公钥),
// 而资源限额/目录隔离这层由容器兑现 —— 正是裸机账号给不了的。
//
// **安全**:成员名、公钥、镜像名、挂载路径全都拼进以 root 运行的脚本 ——
// 严格校验,拒绝一切可逃逸字符,不做转义兜底。
use std::collections::BTreeMap;

use serde::Serialize;

use crate::team::{ShareLimit, Sharing, TeamView};

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

// docker 镜像名:小写字母数字加 . _ - / : @ —— 拒绝空格/引号/$ 等。
fn valid_image(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | ':' | '@'))
}

// docker --gpus 的值:all / 2 / device=0,1
fn valid_gpus(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ',' | '=' | '.' | '-' | '_' | ':'))
}

// docker --memory 的值:数字 + 可选单位。
fn valid_mem(s: &str) -> bool {
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let (num, ok_unit) = match unit {
        "b" | "k" | "m" | "g" | "B" | "K" | "M" | "G" => (num, true),
        _ => (s, s.chars().all(|c| c.is_ascii_digit())),
    };
    ok_unit && !num.is_empty() && num.chars().all(|c| c.is_ascii_digit())
}

// 挂载路径:必须绝对,且不含冒号（会拆坏 -v 参数）与任何可逃逸字符。
fn valid_path(s: &str) -> bool {
    s.starts_with('/')
        && s.len() <= 4096
        && !s.contains("..")
        && !s.chars().any(|c| {
            c.is_control() || matches!(c, ':' | '\'' | '"' | '\\' | '`' | '$' | ' ' | ',' | '\n' | '\r')
        })
}

// 一个将被建立的账号（身份到人）及其从 RBAC 算出的档位。
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct Account {
    pub name: String,
    pub role: String,
    pub tier: u8,
    pub sudo: bool,
    // 这个人怎么被兑现:"forward"(档0 仅转发) | "account"(裸机账号) | "container"(独立容器)
    pub mode: String,
    // 容器名（容器模式才有）——一人一个,焚/停都只影响他自己。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub container: String,
    // 这个容器拿到的限额，人话（如 "8 核 · 32g · GPU all"）。空 = 不限。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub limits: String,
    // 免 root 模式下他专属的高位端口（容器里的 sshd 发布到宿主）。0 = 不适用。
    #[serde(skip_serializing_if = "is_zero")]
    pub port: u16,
}

fn is_zero(v: &u16) -> bool {
    *v == 0
}

#[derive(Serialize, Debug, PartialEq)]
pub struct ProvisionPlan {
    pub server: String,
    pub accounts: Vec<Account>, // 每个可登入成员一条（含各自档位/是否 sudo）
    pub any_sudo: bool,
    // 裸机账号档（Linux only）才有：一段要以 root 跑的 shell。
    pub script: String,
    // 容器档才有：OS 中立的命令清单（宿主 shell 不参与解释）。
    #[serde(default)]
    pub commands: Vec<Cmd>,
    pub warnings: Vec<String>,
    // 兑现方式:"account" | "container"
    pub isolation: String,
    // 借出资源池（父 cgroup）的人话描述。空 = 主人没设上限。
    pub pool: String,
    // 上限怎么兑现的:cgroup（真父池，硬顶）| divided（按人数分摊到每个容器）| 空（没设上限）
    #[serde(default)]
    pub pool_kind: String,
    // 只读挂进每个容器的共享数据集，人话。
    pub datasets: Vec<String>,
}

// 限额的人话摘要。
fn limits_text(l: &ShareLimit) -> String {
    let mut p = Vec::new();
    if let Some(c) = l.cpus {
        p.push(format!("{c} 核"));
    }
    if !l.mem.is_empty() {
        p.push(l.mem.clone());
    }
    if !l.gpus.is_empty() {
        p.push(format!("GPU {}", l.gpus));
    }
    p.join(" · ")
}

// 一个通过校验的成员在这台机上的授权（档位 + 公钥）。
struct Cand {
    name: String,
    role: String,
    tier: u8,
    key: String,
}

// 把「角色 × grant」交叉成候选名单。非法用户名/公钥直接报错 ——
// 宁可拒绝，也不把可疑输入送进 root 脚本。
fn candidates(view: &TeamView, machine: &crate::team::ViewMachine, warnings: &mut Vec<String>) -> Result<Vec<Cand>, String> {
    let mut out = Vec::new();
    for mem in &view.members {
        let Some(tier) = view.effective_tier(machine, mem) else {
            continue; // 该角色未获授权 → 什么都不建
        };
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
        out.push(Cand { name: mem.name.clone(), role: mem.role.clone(), tier, key: key.into() });
    }
    Ok(out)
}

// 档 0 = 纯跳板：账号存在（ProxyJump 需要能认证），但没有 shell、只放行端口转发。
// 没有这个账号，「借道」根本借不成 —— 档 0 就成了空话。
fn forward_block(c: &Cand) -> String {
    format!(
        r#"
# ── {name}（{role}，档 0 · 纯跳板：只借道，无 shell）──────────────
if id -u '{name}' >/dev/null 2>&1; then
  echo "  转发账号 {name} 已存在"
else
  useradd -m -s "$NOLOGIN" '{name}' && echo "  已建转发账号 {name}（无 shell）"
fi
install -d -m 700 -o '{name}' -g '{name}' "/home/{name}/.ssh"
touch "/home/{name}/.ssh/authorized_keys"
if grep -qxF '{line}' "/home/{name}/.ssh/authorized_keys"; then
  echo "  公钥已在 {name}"
else
  printf '%s\n' '{line}' >> "/home/{name}/.ssh/authorized_keys" && echo "  已装公钥 {name}（仅转发）"
fi
chmod 600 "/home/{name}/.ssh/authorized_keys"
chown -R '{name}':'{name}' "/home/{name}/.ssh"
rm -f /etc/sudoers.d/devsys-{name}
"#,
        name = c.name,
        role = c.role,
        // restrict 关掉一切，再单独放回 port-forwarding；command 挡死 shell。
        line = format!("command=\"/bin/false\",restrict,port-forwarding {}", c.key),
    )
}

// 为一台机生成下发脚本。档位对每个成员分别算 = 该机对其角色开的 grant。
pub fn plan(view: &TeamView, server: &str, observed: Option<&Observed>) -> Result<ProvisionPlan, String> {
    let machine = view
        .machines
        .iter()
        .find(|m| m.name == server)
        .ok_or_else(|| format!("团队里没有机器 {server}"))?;

    // 容器档要吃探测结果（同一份 desired 在 Linux / Docker Desktop 上生成的命令不一样），
    // 所以调用方必须先 observe。裸机账号档是 Linux-only 的备选，仍走老的脚本路径。
    if machine.sharing.is_container() {
        let o = observed.ok_or("容器模式要先探测这台机（没有探测结果算不出该发哪些命令）")?;
        return plan_containerized(view, server, o);
    }
    let mut warnings = Vec::new();
    let cands = candidates(view, machine, &mut warnings)?;
    plan_account(server, cands, warnings)
}

// 容器档:desired（纯）× observed（探测）→ 命令清单。
fn plan_containerized(view: &TeamView, server: &str, o: &Observed) -> Result<ProvisionPlan, String> {
    let (d, mut warnings) = desired(view, server)?;
    let (commands, more) = container_commands(&d, o)?;
    warnings.extend(more);

    let pool = d.limit.as_ref().map(limits_text).unwrap_or_default();
    // 真父池（Linux+root）还是按人数分摊（Docker Desktop / 无 root）—— UI 要说清区别。
    let pool_kind = if pool.is_empty() {
        String::new()
    } else if o.can_pool() {
        "cgroup".to_string()
    } else {
        "divided".to_string()
    };
    let accounts = d
        .users
        .iter()
        .map(|u| Account {
            name: u.name.clone(),
            role: u.role.clone(),
            tier: u.tier,
            sudo: u.tier >= 2,
            mode: if u.tier == 0 { "forward".into() } else { "container".into() },
            container: u.container.clone(),
            limits: pool.clone(),
            port: u.port,
        })
        .collect::<Vec<_>>();
    let any_sudo = accounts.iter().any(|a| a.sudo);
    Ok(ProvisionPlan {
        server: server.into(),
        any_sudo,
        script: String::new(),
        commands,
        warnings,
        isolation: "container".into(),
        pool,
        pool_kind,
        datasets: d
            .data
            .iter()
            .map(|(h, i, ro)| format!("{h} → {i}（{}）", if *ro { "只读" } else { "可写" }))
            .collect(),
        accounts,
    })
}

// ── 兑现方式 ①：裸机账号（v1）────────────────────────────
fn plan_account(server: &str, cands: Vec<Cand>, mut warnings: Vec<String>) -> Result<ProvisionPlan, String> {
    let mut accounts = Vec::new();
    let mut blocks = Vec::new();

    for c in &cands {
        if c.tier == 0 {
            accounts.push(Account {
                name: c.name.clone(), role: c.role.clone(), tier: 0, sudo: false,
                mode: "forward".into(), container: String::new(), limits: String::new(), port: 0,
            });
            blocks.push(forward_block(c));
            continue;
        }
        let sudo = c.tier >= 2;
        accounts.push(Account {
            name: c.name.clone(), role: c.role.clone(), tier: c.tier, sudo,
            mode: "account".into(), container: String::new(), limits: String::new(), port: 0,
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
            name = c.name,
            role = c.role,
            tier = c.tier,
            key = c.key,
            sudo_block = if sudo {
                format!(
                    "printf '%s ALL=(ALL) NOPASSWD:ALL\\n' '{n}' > /etc/sudoers.d/devsys-{n}\nchmod 440 /etc/sudoers.d/devsys-{n}\necho \"  已给 {n} sudo（档 2）\"\n",
                    n = c.name
                )
            } else {
                // 档 1:确保没有 sudo（清掉我们可能留下的旧授权）。
                format!("rm -f /etc/sudoers.d/devsys-{n}\necho \"  {n} 无 sudo（档 1）\"\n", n = c.name)
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
    if accounts.iter().any(|a| a.tier == 1) {
        warnings.push(
            "档 1 在**裸机账号**模式下只保证「无 sudo + 独立账号」:没有 CPU/内存/GPU 限额，一个人能占满整台机，也看得见宿主目录。\
             想要真限额与目录隔离,把这台机的隔离方式改成「一人一容器」。"
                .into(),
        );
    }

    let script = format!(
        r#"#!/bin/sh
# DevSys 授权下发 —— 服务器 {server}（隔离方式：裸机账号）
# 由 app 生成，需以 root 执行。幂等：可重复运行。
# 身份到人 + RBAC：每位成员一个独立账号，权限按其角色定（core=sudo / member=受限）。
set -e
[ "$(uname -s)" = Linux ] || {{ echo "这一档只支持 Linux（要 useradd / sudoers）。macOS 上请把隔离方式改成「一人一容器 · 免 root」—— 那档不碰宿主账号。" >&2; exit 1; }}
if [ "$(id -u)" -ne 0 ]; then echo "需要 root（请用 sudo 运行）" >&2; exit 1; fi
NOLOGIN=/usr/sbin/nologin
[ -x "$NOLOGIN" ] || NOLOGIN=/sbin/nologin
[ -x "$NOLOGIN" ] || NOLOGIN=/bin/false
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
        commands: vec![],
        warnings,
        isolation: "account".into(),
        pool: String::new(),
        pool_kind: String::new(),
        datasets: vec![],
    })
}
// ── 兑现方式 ②：一人一容器（三平台统一，v2）──────────────
//
// 设计要点（讨论定稿，别再重推）:
//
// 1. **交付单位是容器，不是宿主账号**。容器里永远是 Linux —— 宿主是 Linux / macOS /
//    Windows 都无所谓。所以「支持三平台」不是三套实现，是同一套。
// 2. **宿主上只发 OS 中立的 `docker` 命令**:每个 token 都是 `[A-Za-z0-9._:/=,+@-]`，
//    没有引号、管道、重定向、heredoc、`$展开`。需要往远端写脚本时走 **SSH stdin**
//    （`docker exec -i C tee /path`），宿主 shell 全程不参与解释。
//    教训:以前把编排写成一大段 `sh` 脚本 → 宿主 shell 成了硬依赖 → Windows 的
//    cmd.exe 连 heredoc 都没有，断在 docker 之前。**卡住 Linux 的从来不是 docker。**
// 3. **复杂 shell 全部下沉进镜像的 init**（那里是 Linux），靠 `-e` 传参。
// 4. **plan / apply 两段，两段都是纯函数**:
//      `desired(view, server)`         → 应该长成什么样（不联网）
//      `container_commands(d, o)`      → 要执行哪些命令（吃探测结果）
//    只有「探测」和「执行」是 I/O。于是 macOS/Windows 的行为在 Linux 上就能单测。
// 5. **父池两套实现、一个语义**:
//      Linux + 宿主有 root → 真 systemd slice（硬上限，与容器数无关，可热改）
//      其余（含 Docker Desktop）→ 上限**除以容器数**下到每个容器（`docker update` 热改）
//    **绝不去改 Docker Desktop 的 VM 大小** —— 那要重启引擎 = 把所有人踢下线。
//    VM 配额只读:`docker info` 报的 NCPU/MemTotal 就是外层盘子，超了当场告诉主人。
// 6. **档 2「可访问宿主」只在 Linux 成立**。mac/win 上挂 `/` 挂到的是 Docker Desktop
//    那个 VM 的根，不是你的 Mac —— 那两个平台只给「容器内 root + 更大配额」，UI 明说。

pub const BASE_IMAGE: &str = "devsys/base-ssh:2";
const STOCK_IMAGE: &str = "ubuntu:24.04";
const BUILD_CTR: &str = "devsys-imgbuild";
const POOL_SLICE: &str = "devsys-shared.slice";

// 容器 init：按 env 建人 + 装公钥 + 起 sshd。
// **所有需要真 shell 的活都在这儿** —— 容器里永远是 Linux，宿主那侧只传裸 token。
const INIT_SCRIPT: &str = r#"#!/bin/sh
# DevSys 容器 init（由 app 装进镜像）。每次容器启动都跑一遍 —— 幂等、自愈。
set -e
u="$DEVSYS_USER"
[ -n "$u" ] || { echo "缺 DEVSYS_USER" >&2; exit 1; }
sh_="${DEVSYS_SHELL:-/bin/bash}"
id -u "$u" >/dev/null 2>&1 || useradd -m -u "${DEVSYS_UID:-1000}" -s "$sh_" "$u"
usermod -s "$sh_" "$u"
install -d -m 700 -o "$u" -g "$u" "/home/$u/.ssh"
printf '%s' "$DEVSYS_KEY_B64" | base64 -d > "/home/$u/.ssh/authorized_keys"
chmod 600 "/home/$u/.ssh/authorized_keys"
chown "$u:$u" "/home/$u/.ssh/authorized_keys"
if [ "$DEVSYS_SUDO" = 1 ]; then
  printf '%s ALL=(ALL) NOPASSWD:ALL\n' "$u" > /etc/sudoers.d/devsys
  chmod 440 /etc/sudoers.d/devsys
else
  rm -f /etc/sudoers.d/devsys
fi
mkdir -p /run/sshd
# 首次启动时丢掉镜像里烤进来的 host key，让**每个容器有自己的身份**
# （否则同一台机上所有人的容器共用一把 host key）。之后保持不变，队友不会看到
# "REMOTE HOST IDENTIFICATION HAS CHANGED"。
if [ ! -f /etc/ssh/.devsys-hostkeys ]; then
  rm -f /etc/ssh/ssh_host_*
  touch /etc/ssh/.devsys-hostkeys
fi
ssh-keygen -A >/dev/null 2>&1 || true
exec /usr/sbin/sshd -D -e
"#;

// 装进镜像的包。tmux 是硬需求 —— 工作区的持久会话靠它。
const IMAGE_PKGS: &[&str] = &[
    "bash", "tmux", "git", "curl", "ca-certificates", "sudo",
    "openssh-server", "openssh-client", "python3", "less",
];

// ── 探测到的现实 ────────────────────────────────────────
#[derive(Serialize, Clone, Debug, Default)]
pub struct ObservedContainer {
    pub exists: bool,
    pub running: bool,
    pub stamp: String, // devsys.stamp 标签：端口+档位+公钥的指纹，变了就得重建
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Observed {
    pub os: String,      // linux | darwin | windows | unknown（来自 `docker version` 的 Client OS/Arch）
    pub engine: String,  // docker | podman（空 = 没找到容器引擎）
    pub rootless: bool,  // 引擎以 rootless 模式跑
    pub desktop: bool,   // 容器在 Docker Desktop / podman machine 的 VM 里
    pub ncpu: f64,       // 引擎能看到的核数（Desktop 上 = VM 的，就是外层盘子）
    pub mem_mib: u64,
    pub gpu: bool,
    pub host_root: bool, // 宿主有 root / 免密 sudo（建 systemd slice 用）
    pub systemd: bool,
    pub image_ready: bool,
    pub containers: BTreeMap<String, ObservedContainer>,
}

impl Observed {
    pub fn is_linux(&self) -> bool {
        self.os == "linux"
    }
    // 能不能建**真**父池:要 Linux + 宿主 root + systemd。Docker Desktop 一概不行。
    pub fn can_pool(&self) -> bool {
        self.is_linux() && self.host_root && self.systemd && !self.desktop
    }
}

// ── 应该长成什么样（纯计算，不联网）──────────────────────
#[derive(Serialize, Clone, Debug)]
pub struct DesiredUser {
    pub name: String,
    pub role: String,
    pub tier: u8,
    pub key: String, // 完整 authorized_keys 行（档 0 带 restrict 前缀）
    pub container: String,
    pub volume: String,
    pub port: u16,
    pub uid: u32,
}

#[derive(Serialize, Clone, Debug)]
pub struct Desired {
    pub server: String,
    pub image: String,
    pub custom_image: bool,
    pub users: Vec<DesiredUser>,
    pub limit: Option<ShareLimit>,
    pub data: Vec<(String, String, bool)>, // host, 容器内, 只读
}

// ── 一条要执行的命令 ────────────────────────────────────
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Cmd {
    pub label: String,      // 人话：这条要干什么（给主人 review 用）
    pub argv: Vec<String>,  // OS 中立 token；宿主 shell 不参与解释
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>, // 走 SSH 数据通道的内容（不进命令行 → 零引号问题）
    pub optional: bool,     // 失败只警告，不中止
    pub host_shell: bool,   // 需要宿主 shell（只用于 Linux 专属的 systemd 那几条）
}

impl Cmd {
    fn new(label: impl Into<String>, argv: &[&str]) -> Self {
        Cmd {
            label: label.into(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            stdin: None,
            optional: false,
            host_shell: false,
        }
    }
    fn opt(mut self) -> Self {
        self.optional = true;
        self
    }
    fn with_stdin(mut self, s: impl Into<String>) -> Self {
        self.stdin = Some(s.into());
        self
    }
    fn shell(mut self) -> Self {
        self.host_shell = true;
        self
    }
    // 命令行形态（执行与展示都用它）。
    pub fn line(&self) -> String {
        self.argv.join(" ")
    }
}

// 宿主命令行里允许出现的字符。**这是 OS 中立的守门人** ——
// 只要每个 token 都落在这个集合里，sh / cmd.exe / PowerShell 的解释就一致。
fn os_neutral_token(t: &str) -> bool {
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/' | '=' | ',' | '+' | '@'))
}

// 内容指纹:端口/档位/公钥任一变了就要重建容器（env 只在创建时生效）。
fn stamp(port: u16, tier: u8, key: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    format!("p{port}t{tier}k{h:x}")
}

fn b64(s: &str) -> String {
    // 只用来把公钥塞进 env（值必须是裸 token）。标准字母表，不换行。
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::new();
    for c in b.chunks(3) {
        let (b0, b1, b2) = (c[0] as u32, *c.get(1).unwrap_or(&0) as u32, *c.get(2).unwrap_or(&0) as u32);
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

// "32g" / "4096m" → MiB。给 mac/win 分摊上限用。
fn mem_to_mib(s: &str) -> Option<u64> {
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let (n, mul): (&str, u64) = match unit {
        "g" | "G" => (num, 1024),
        "m" | "M" => (num, 1),
        "k" | "K" => (num, 0), // 太小，忽略
        "b" | "B" => (num, 0),
        _ => (s, 0),
    };
    if mul == 0 {
        return None;
    }
    n.parse::<u64>().ok().map(|v| v * mul)
}

// team.yaml → desired。纯函数:不联网、不看宿主。
pub fn desired(view: &TeamView, server: &str) -> Result<(Desired, Vec<String>), String> {
    let machine = view
        .machines
        .iter()
        .find(|m| m.name == server)
        .ok_or_else(|| format!("团队里没有机器 {server}"))?;
    let sh = &machine.sharing;
    let mut warnings = Vec::new();
    let cands = candidates(view, machine, &mut warnings)?;
    let ports = view.container_ports(machine);

    let image = if sh.image.is_empty() { BASE_IMAGE.to_string() } else { sh.image.clone() };
    if !valid_image(&image) {
        return Err(format!("镜像名 {image:?} 不合法（只允许字母数字与 . _ - / : @）"));
    }
    if let Some(l) = &sh.limit {
        if let Some(c) = l.cpus {
            if !(c.is_finite() && c > 0.0 && c <= 4096.0) {
                return Err(format!("借出上限 cpus={c} 不合法"));
            }
        }
        if !l.mem.is_empty() && !valid_mem(&l.mem) {
            return Err(format!("借出上限 mem={:?} 不合法（应形如 32g / 4096m）", l.mem));
        }
        if !l.gpus.is_empty() && !valid_gpus(&l.gpus) {
            return Err(format!("借出上限 gpus={:?} 不合法（应形如 all / 2 / device=0,1）", l.gpus));
        }
    }

    let mut data = Vec::new();
    for d in &sh.data {
        let inner = if d.mount_as.is_empty() { &d.host } else { &d.mount_as };
        if !valid_path(&d.host) || !valid_path(inner) {
            return Err(format!(
                "共享数据集路径 {:?} → {inner:?} 不合法（必须绝对路径，且不含空格/冒号/引号/..）",
                d.host
            ));
        }
        let ro = d.mode != "rw";
        if !ro {
            warnings.push(format!("数据集 {} 是**可写**挂载 —— 借用者能改你的原始数据，确认这是你要的。", d.host));
        }
        data.push((d.host.clone(), inner.clone(), ro));
    }

    let mut users = Vec::new();
    for c in &cands {
        let Some(port) = ports.iter().find(|(n, _)| *n == c.name).map(|(_, p)| *p) else {
            continue; // 档 0 也在 ports 里；不在就是这台机没给他开档
        };
        // 档 0 = 纯跳板:容器里那个人没有 shell，公钥只放行端口转发。
        // 容器照样能当 ProxyJump 跳板 —— 桥接网络可以正常对局域网发起出站连接。
        let key = if c.tier == 0 {
            format!("command=\"/bin/false\",restrict,port-forwarding {}", c.key)
        } else {
            c.key.clone()
        };
        users.push(DesiredUser {
            name: c.name.clone(),
            role: c.role.clone(),
            tier: c.tier,
            key,
            container: format!("devsys-{}", c.name),
            volume: format!("devsys-{}-home", c.name),
            port,
            uid: 2000 + users.len() as u32,
        });
    }

    Ok((
        Desired {
            server: server.into(),
            image,
            custom_image: !sh.image.is_empty(),
            users,
            limit: sh.limit.clone(),
            data,
        },
        warnings,
    ))
}

// desired + observed → 要执行的命令清单。纯函数，所以三个平台的行为都能单测。
pub fn container_commands(d: &Desired, o: &Observed) -> Result<(Vec<Cmd>, Vec<String>), String> {
    let mut cmds = Vec::new();
    let mut warnings = Vec::new();
    let eng = if o.engine.is_empty() { "docker" } else { o.engine.as_str() };
    let n = d.users.len().max(1) as f64;

    if o.engine.is_empty() {
        return Err("这台机上没找到 docker 也没找到 podman —— 装一个再下发（面板上有该系统的安装命令）".into());
    }
    if o.os == "windows" {
        warnings.push(
            "Windows 是**按设计支持、但我们没实测过**的:命令全是 OS 中立的裸 token、脚本走 SSH stdin，\
             理论上 cmd.exe 也照跑。跑不通请把输出给我们 —— 保底方案是共享 WSL2 里的那套 Linux。"
                .into(),
        );
    }

    // ① 基础镜像：不用 `docker build`（那要 stdin 喂 Dockerfile，且 `-` 的行为跨平台不一致），
    //    改成 run → exec 装包 → tee 写 init → commit。全程裸 token + SSH stdin。
    if !o.image_ready && !d.custom_image {
        cmds.push(Cmd::new(format!("拉基础镜像 {STOCK_IMAGE}"), &[eng, "pull", STOCK_IMAGE]));
        cmds.push(Cmd::new("清掉可能残留的构建容器", &[eng, "rm", "-f", BUILD_CTR]).opt());
        cmds.push(Cmd::new("起构建容器", &[eng, "run", "-d", "--name", BUILD_CTR, STOCK_IMAGE, "sleep", "infinity"]));
        cmds.push(Cmd::new(
            "刷新包索引",
            &[eng, "exec", "-e", "DEBIAN_FRONTEND=noninteractive", BUILD_CTR, "apt-get", "update", "-qq"],
        ));
        let mut argv: Vec<&str> = vec![
            eng, "exec", "-e", "DEBIAN_FRONTEND=noninteractive", BUILD_CTR,
            "apt-get", "install", "-y", "--no-install-recommends",
        ];
        argv.extend_from_slice(IMAGE_PKGS);
        cmds.push(Cmd::new("装 sshd / tmux / 常用工具（tmux 是工作区持久化的硬需求）", &argv));
        // 唯一一处「写文件」：内容走 SSH stdin，命令行仍是裸 token。
        cmds.push(
            Cmd::new("写入容器 init 脚本", &[eng, "exec", "-i", BUILD_CTR, "tee", "/usr/local/bin/devsys-init"])
                .with_stdin(INIT_SCRIPT),
        );
        cmds.push(Cmd::new("给 init 执行权", &[eng, "exec", BUILD_CTR, "chmod", "755", "/usr/local/bin/devsys-init"]));
        cmds.push(Cmd::new(format!("固化成镜像 {}", d.image), &[eng, "commit", BUILD_CTR, &d.image]));
        cmds.push(Cmd::new("拆掉构建容器", &[eng, "rm", "-f", BUILD_CTR]).opt());
    } else if d.custom_image && !o.image_ready {
        cmds.push(Cmd::new(format!("拉主人指定的镜像 {}", d.image), &[eng, "pull", &d.image]));
        warnings.push(format!(
            "你指定了自己的镜像 {} —— 它**必须自带 sshd 并把 /usr/local/bin/devsys-init 作为 CMD**，否则队友连不进去。",
            d.image
        ));
    }

    // ② 父池:两套实现一个语义。
    let mut per_cpus: Option<f64> = None;
    let mut per_mem: Option<String> = None;
    let mut pool_arg: Option<String> = None;
    if let Some(l) = &d.limit {
        if o.can_pool() {
            // Linux + root:真 cgroup 父池。与容器数无关，开多少个都超不过；且能热改。
            let mut props = Vec::new();
            if let Some(c) = l.cpus {
                props.push(format!("CPUQuota={}%", (c * 100.0).round() as i64));
            }
            if let Some(m) = mem_to_mib(&l.mem) {
                props.push(format!("MemoryMax={m}M"));
            }
            if !props.is_empty() {
                let mut argv = vec!["systemctl".to_string(), "set-property".to_string(), "--runtime".into(), POOL_SLICE.into()];
                argv.extend(props.clone());
                cmds.push(Cmd {
                    label: format!("设借出资源池 {POOL_SLICE}（{}）—— 热改，不影响在跑的会话", limits_text(l)),
                    argv,
                    stdin: None,
                    optional: true,
                    host_shell: true,
                });
                pool_arg = Some(format!("--cgroup-parent={POOL_SLICE}"));
            }
            // 逐容器也给上限（防单人独占父池），但父池才是硬顶。
            per_cpus = l.cpus;
            per_mem = if l.mem.is_empty() { None } else { Some(l.mem.clone()) };
        } else {
            // 没有真父池（Docker Desktop / rootless / 无 root）→ 把上限**除以人数**下到每个容器。
            // 总量照样被 bound 住，代价是人员增减要重算 —— 而这条路完全靠 `docker update` 热改，
            // 绝不碰 Docker Desktop 的 VM 大小（改那个要重启引擎 = 把所有人踢下线）。
            per_cpus = l.cpus.map(|c| (c / n * 100.0).floor() / 100.0);
            per_mem = mem_to_mib(&l.mem).map(|m| format!("{}m", (m as f64 / n).floor() as u64));
            if l.cpus.is_some() || !l.mem.is_empty() {
                warnings.push(format!(
                    "这台机上建不了父 cgroup 池（{}）—— 已把上限**按 {} 人分摊**到每个容器（各 {}）。\
                     总量仍不超你设的上限，但人员增减时要重跑一次下发来重算。",
                    if o.desktop { "容器跑在 Docker Desktop 的 VM 里" }
                    else if !o.host_root { "没有宿主 root" }
                    else { "没有 systemd" },
                    d.users.len().max(1),
                    limits_of(per_cpus, per_mem.as_deref()),
                ));
            }
        }
        // 外层盘子只读校验:超了当场说，绝不替他改 VM。
        if let (Some(want), true) = (l.cpus, o.ncpu > 0.0) {
            if want > o.ncpu + 0.01 {
                warnings.push(format!(
                    "你要借出 {want} 核，但这台机的容器引擎只看得到 {:.0} 核{} —— 借不出来。{}",
                    o.ncpu,
                    if o.desktop { "（Docker Desktop 的 VM 配额）" } else { "" },
                    if o.desktop { "去 Docker Desktop 的 Resources 里把 VM 调大（那会重启引擎）。" } else { "" }
                ));
            }
        }
        if let (Some(want), true) = (mem_to_mib(&l.mem), o.mem_mib > 0) {
            if want > o.mem_mib {
                warnings.push(format!(
                    "你要借出 {} 内存，但容器引擎只看得到 {} MiB{} —— 借不出来。",
                    l.mem, o.mem_mib,
                    if o.desktop { "（Docker Desktop 的 VM 配额）" } else { "" }
                ));
            }
        }
        if !l.gpus.is_empty() {
            match o.os.as_str() {
                "darwin" => warnings.push("macOS 上没有 GPU 直通（容器跑在虚拟机里）—— GPU 设置会被忽略。这是硬件事实，不是我们的限制。".into()),
                "windows" => warnings.push("Windows 上 GPU 要 WSL2 后端 + **Windows 侧**的 NVIDIA 驱动（切记别在 WSL 里再装 Linux 驱动）。".into()),
                _ if !o.gpu => warnings.push("这台机上没探到 nvidia-smi —— GPU 参数可能会让容器起不来。".into()),
                _ => {}
            }
            if o.is_linux() {
                warnings.push("GPU 是设备直通、**不受父池约束** —— 每个容器都会拿到你声明的这组卡，他们之间自行竞争显存。".into());
            }
        }
    } else if !d.users.is_empty() {
        warnings.push("你没设**借出上限** —— 容器不限 CPU/内存，一个人仍可能占满这台机。".into());
    }

    // ③ 逐人容器。
    for u in &d.users {
        let st = stamp(u.port, u.tier, &u.key);
        let cur = o.containers.get(&u.container);
        let exists = cur.map(|c| c.exists).unwrap_or(false);
        // 端口/档位/公钥变了就必须重建 —— env 只在创建时生效，
        // 且端口漂移不重建的话，队友会连到**别人的容器**上。
        let stale = exists && cur.map(|c| c.stamp != st).unwrap_or(true);

        cmds.push(Cmd::new(format!("确保 {} 的家目录卷在", u.name), &[eng, "volume", "create", &u.volume]).opt());

        if stale {
            cmds.push(Cmd::new(
                format!("{} 的端口/档位/公钥变了 —— 重建容器（家目录卷保留）", u.name),
                &[eng, "rm", "-f", &u.container],
            ));
        }
        if !exists || stale {
            let mut argv: Vec<String> = vec![
                eng.into(), "run".into(), "-d".into(),
                "--name".into(), u.container.clone(),
                "--restart".into(), "unless-stopped".into(),
                "--hostname".into(), format!("{}-{}", u.name, d.server),
                "--label".into(), format!("devsys.stamp={st}"),
                "--label".into(), format!("devsys.owner={}", u.name),
                // **显式写 0.0.0.0**：Docker Desktop 有个「默认只绑 localhost」的开关，
                // 省掉 IP 就会被它影响，而且是静默连不上。
                "-p".into(), format!("0.0.0.0:{}:22", u.port),
                "-e".into(), format!("DEVSYS_USER={}", u.name),
                "-e".into(), format!("DEVSYS_UID={}", u.uid),
                "-e".into(), format!("DEVSYS_KEY_B64={}", b64(&u.key)),
                "-e".into(), format!("DEVSYS_SUDO={}", if u.tier >= 2 { 1 } else { 0 }),
                // 档 0 = 纯跳板:容器里也没有 shell，只借道。
                "-e".into(), format!("DEVSYS_SHELL={}", if u.tier == 0 { "/usr/sbin/nologin" } else { "/bin/bash" }),
                "-v".into(), format!("{}:/home/{}", u.volume, u.name),
            ];
            if let Some(p) = &pool_arg {
                argv.push(p.clone());
            }
            if let Some(c) = per_cpus {
                argv.push("--cpus".into());
                argv.push(format!("{c:.2}"));
            }
            if let Some(m) = &per_mem {
                argv.push("--memory".into());
                argv.push(m.clone());
            }
            // 档 2「可访问宿主」只在 Linux 成立 —— mac/win 上挂 / 挂到的是 VM 的根。
            if u.tier >= 2 {
                if o.is_linux() && !o.desktop {
                    argv.push("--privileged".into());
                    argv.push("-v".into());
                    argv.push("/:/host".into());
                } else {
                    argv.push("--pids-limit".into());
                    argv.push("8192".into());
                }
            } else {
                argv.push("--security-opt".into());
                argv.push("no-new-privileges".into());
                argv.push("--pids-limit".into());
                argv.push("4096".into());
            }
            // 档 0 是纯跳板：不给 GPU、不挂数据集 —— 那个容器里没人能跑东西。
            if u.tier >= 1 {
                if let Some(l) = &d.limit {
                    if !l.gpus.is_empty() && o.is_linux() {
                        if eng == "podman" {
                            argv.push("--device".into());
                            argv.push(format!("nvidia.com/gpu={}", l.gpus));
                        } else {
                            argv.push("--gpus".into());
                            argv.push(l.gpus.clone());
                        }
                    }
                }
                for (host, inner, ro) in &d.data {
                    argv.push("-v".into());
                    argv.push(format!("{host}:{inner}:{}", if *ro { "ro" } else { "rw" }));
                }
            }
            argv.push(d.image.clone());
            argv.push("/usr/local/bin/devsys-init".into());
            let refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
            cmds.push(Cmd::new(
                format!(
                    "建 {} 的容器 {}（端口 {}，档 {}{}）",
                    u.name, u.container, u.port, u.tier,
                    if u.tier >= 2 { "·sudo" } else { "" }
                ),
                &refs,
            ));
        } else {
            // 存在且没过期:只热改限额 + 确保在跑。绝不重建 —— 那会把人踢下线。
            if per_cpus.is_some() || per_mem.is_some() {
                let mut argv = vec![eng.to_string(), "update".to_string()];
                if let Some(c) = per_cpus {
                    argv.push("--cpus".into());
                    argv.push(format!("{c:.2}"));
                }
                if let Some(m) = &per_mem {
                    argv.push("--memory".into());
                    argv.push(m.clone());
                }
                argv.push(u.container.clone());
                let refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
                cmds.push(Cmd::new(format!("热改 {} 的限额（不打断他的会话）", u.name), &refs).opt());
            }
            cmds.push(Cmd::new(format!("确保 {} 的容器在跑", u.name), &[eng, "start", &u.container]).opt());
        }
    }

    // ④ Linux 上让容器活过登出（rootless 尤其要）。非 Linux 由 Docker Desktop 托管，不需要。
    if o.is_linux() && o.rootless {
        cmds.push(
            Cmd::new("开 linger（否则你一登出，systemd 就把容器收走）", &["loginctl", "enable-linger"])
                .opt()
                .shell(),
        );
    }

    if d.users.is_empty() {
        warnings.push("没有任何可下发的容器 —— 要么无成员获此机授权，要么成员缺公钥。队友仍登不进来。".into());
    } else {
        warnings.push(
            "队友各自连 `<主机>:<他的端口>`（**不是 22**）—— app 会按 team.yaml 自动算好填进他们的服务器列表，手工 ssh 要带 -p。"
                .into(),
        );
        warnings.push(
            "端口绑在**所有网卡**上。容器里的 sshd 只认公钥、禁密码，但这台机若有公网 IP，端口就暴露在公网 —— 建议靠防火墙或 tailnet ACL 收口。"
                .into(),
        );
        warnings.push(
            "普通容器**共享内核，不是安全边界**:适合可信队友。不可信的人 / agent 生成的代码要强沙箱（microVM / gVisor），那是另一层。"
                .into(),
        );
    }
    if d.users.iter().any(|u| u.tier >= 2) {
        if o.is_linux() && !o.desktop {
            warnings.push("有成员是**档 2**：特权容器 + 挂载宿主 `/host` —— 等价于宿主 root。仅限核心成员。".into());
        } else {
            warnings.push(
                "有成员是**档 2**，但这台机上它**给不了宿主访问** —— 容器跑在 Docker Desktop 的 VM 里，挂 `/` 挂到的是 VM 的根，不是你的机器。这里的档 2 = 容器内 root + 更大配额。"
                    .into(),
            );
        }
    }

    // 守门:任何一个 token 越界就是 bug —— 宁可拒发，也别送一条会被宿主 shell 曲解的命令。
    for c in &cmds {
        if c.host_shell {
            continue;
        }
        for t in &c.argv {
            if !os_neutral_token(t) {
                return Err(format!("内部错误:命令 token {t:?} 不是 OS 中立的（{}）", c.label));
            }
        }
    }
    Ok((cmds, warnings))
}

fn limits_of(cpus: Option<f64>, mem: Option<&str>) -> String {
    let mut p = Vec::new();
    if let Some(c) = cpus {
        p.push(format!("{c:.2} 核"));
    }
    if let Some(m) = mem {
        p.push(m.to_string());
    }
    if p.is_empty() { "不限".into() } else { p.join(" · ") }
}#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::{merge, new_member_file, upsert_machine, Machine, ShareData, TeamRoot};

    const KEY_A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample alice@mac";

    fn root() -> TeamRoot {
        crate::team::parse_root("team: neuroai\n").unwrap()
    }

    fn view(grants: &[(&str, u8)], members: &[(&str, &str, &str)]) -> TeamView {
        view_sh(grants, members, Sharing::default())
    }

    fn view_sh(grants: &[(&str, u8)], members: &[(&str, &str, &str)], sharing: Sharing) -> TeamView {
        let g: BTreeMap<String, u8> = grants.iter().map(|(r, t)| (r.to_string(), *t)).collect();
        let mut files = Vec::new();
        for (i, (n, k, role)) in members.iter().enumerate() {
            let mut f = new_member_file(n, k, role);
            if i == 0 {
                upsert_machine(&mut f, Machine {
                    name: "gpu".into(), host: "10.0.0.1".into(), port: 22, jump: None,
                    username: String::new(), transport: "direct".into(), grants: g.clone(),
                    advertises: vec![], sharing: sharing.clone(),
                });
            }
            files.push(f);
        }
        merge(&root(), &files)
    }

    // 容器档 + 8 核 / 32g / GPU all + 一个只读数据集。
    fn ctr() -> Sharing {
        Sharing {
            isolation: "container".into(),
            image: String::new(),
            limit: Some(ShareLimit { cpus: Some(8.0), mem: "32g".into(), gpus: "all".into() }),
            data: vec![ShareData { host: "/data/imagenet".into(), mount_as: String::new(), mode: "ro".into() }],
            port_base: None,
        }
    }

    // ── 三种宿主现实（这就是拆纯函数的回报:在 Linux 上单测 macOS/Windows 的行为）──
    fn linux_root() -> Observed {
        Observed {
            os: "linux".into(), engine: "docker".into(), host_root: true, systemd: true,
            gpu: true, ncpu: 64.0, mem_mib: 256 * 1024, ..Default::default()
        }
    }
    fn linux_rootless() -> Observed {
        Observed { engine: "podman".into(), rootless: true, host_root: false, ..linux_root() }
    }
    fn mac_desktop() -> Observed {
        Observed {
            os: "darwin".into(), engine: "docker".into(), desktop: true, host_root: false,
            systemd: false, gpu: false, ncpu: 10.0, mem_mib: 8092, ..Default::default()
        }
    }
    fn windows_desktop() -> Observed {
        Observed { os: "windows".into(), gpu: true, ..mac_desktop() }
    }

    fn cmds(v: &TeamView, o: &Observed) -> Vec<Cmd> {
        let (d, _) = desired(v, "gpu").unwrap();
        container_commands(&d, o).unwrap().0
    }
    fn warns(v: &TeamView, o: &Observed) -> Vec<String> {
        let (d, w) = desired(v, "gpu").unwrap();
        let (_, w2) = container_commands(&d, o).unwrap();
        [w, w2].concat()
    }
    fn joined(c: &[Cmd]) -> String {
        c.iter().map(|x| x.line()).collect::<Vec<_>>().join("\n")
    }

    // ★★ 三平台的核心不变量:宿主上发的每个 token 都是 OS 中立的。
    // 这条挂了就意味着某个平台的 shell 会曲解命令 —— 而且多半是静默的。
    #[test]
    fn every_host_token_is_os_neutral_on_all_platforms() {
        let v = view_sh(&[("core", 2), ("member", 1), ("pub", 0)],
                        &[("alice", KEY_A, "core"), ("bob", KEY_A, "member"), ("dan", KEY_A, "pub")], ctr());
        for o in [linux_root(), linux_rootless(), mac_desktop(), windows_desktop()] {
            for c in cmds(&v, &o) {
                if c.host_shell {
                    continue; // systemd 那两条是 Linux 专属，本来就要宿主 shell
                }
                for t in &c.argv {
                    assert!(
                        os_neutral_token(t),
                        "{} 上的 token {t:?} 不是 OS 中立的（{}）—— cmd.exe 会曲解它",
                        o.os, c.label
                    );
                }
            }
        }
    }

    // ★ 要写文件的那条必须走 SSH stdin，绝不能拼进命令行（那就又依赖宿主 shell 了）。
    #[test]
    fn file_content_goes_through_stdin_not_command_line() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], ctr());
        let c = cmds(&v, &mac_desktop());
        let w = c.iter().find(|x| x.stdin.is_some()).expect("应有一条靠 stdin 写 init");
        assert!(w.line().ends_with("tee /usr/local/bin/devsys-init"));
        assert!(w.stdin.as_ref().unwrap().contains("exec /usr/sbin/sshd"));
        // 全流程只该有这一条带 stdin
        assert_eq!(c.iter().filter(|x| x.stdin.is_some()).count(), 1);
    }

    // ★ 一人一容器 + 每人一个高位端口 + 显式绑 0.0.0.0。
    #[test]
    fn one_container_and_one_port_per_person() {
        let v = view_sh(&[("core", 2), ("member", 1)],
                        &[("alice", KEY_A, "core"), ("bob", KEY_A, "member")], ctr());
        let s = joined(&cmds(&v, &linux_root()));
        assert!(s.contains("--name devsys-alice"));
        assert!(s.contains("--name devsys-bob"));
        // 显式 0.0.0.0:Docker Desktop 有「默认只绑 localhost」的开关，省掉 IP 会静默连不上
        assert!(s.contains("-p 0.0.0.0:2200:22"));
        assert!(s.contains("-p 0.0.0.0:2201:22"));
        assert!(s.contains("-v devsys-alice-home:/home/alice"));
    }

    // ★ 父池两套实现一个语义:Linux+root 用真 cgroup 池；Docker Desktop 按人数分摊。
    #[test]
    fn pool_is_real_cgroup_on_linux_root() {
        let v = view_sh(&[("core", 2), ("member", 1)],
                        &[("alice", KEY_A, "core"), ("bob", KEY_A, "member")], ctr());
        let c = cmds(&v, &linux_root());
        let s = joined(&c);
        assert!(s.contains("systemctl set-property --runtime devsys-shared.slice CPUQuota=800% MemoryMax=32768M"));
        assert!(s.contains("--cgroup-parent=devsys-shared.slice"));
        // 父池是硬顶，所以逐容器给的是**完整**上限（不分摊）
        assert!(s.contains("--cpus 8.00"));
        assert!(c.iter().any(|x| x.host_shell), "systemctl 要标成需要宿主 shell");
    }

    #[test]
    fn pool_is_divided_per_container_on_docker_desktop() {
        let v = view_sh(&[("core", 2), ("member", 1)],
                        &[("alice", KEY_A, "core"), ("bob", KEY_A, "member")], ctr());
        let s = joined(&cmds(&v, &mac_desktop()));
        assert!(!s.contains("cgroup-parent"), "Desktop 上建不了父池");
        assert!(!s.contains("systemctl"));
        // 8 核 / 2 人 = 每人 4；32g / 2 = 16384m
        assert!(s.contains("--cpus 4.00"), "应按人数分摊:{s}");
        assert!(s.contains("--memory 16384m"));
        assert!(warns(&v, &mac_desktop()).iter().any(|w| w.contains("分摊")));
    }

    // ★ 绝不去改 Docker Desktop 的 VM 大小 —— 那要重启引擎 = 把所有人踢下线。
    // 外层盘子只读:超了就说，不动它。
    #[test]
    fn never_touches_the_vm_size_only_warns_when_over() {
        let big = Sharing { limit: Some(ShareLimit { cpus: Some(16.0), mem: "64g".into(), gpus: String::new() }), ..ctr() };
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], big);
        let s = joined(&cmds(&v, &mac_desktop()));
        for forbidden in ["settings-store", "wslconfig", "docker desktop restart", "wsl"] {
            assert!(!s.to_lowercase().contains(forbidden), "不该动 VM 设置:{forbidden}");
        }
        let w = warns(&v, &mac_desktop());
        assert!(w.iter().any(|x| x.contains("16 核") && x.contains("10 核")), "要说清借不出来:{w:?}");
        assert!(w.iter().any(|x| x.contains("64g")));
    }

    // ★ 档 2「可访问宿主」只在 Linux 成立 —— mac/win 上挂 / 挂到的是 VM 的根。
    #[test]
    fn tier2_host_access_is_linux_only_and_says_so() {
        let v = view_sh(&[("core", 2)], &[("alice", KEY_A, "core")], ctr());

        let s = joined(&cmds(&v, &linux_root()));
        assert!(s.contains("--privileged"));
        assert!(s.contains("-v /:/host"));
        assert!(s.contains("DEVSYS_SUDO=1"));

        let s = joined(&cmds(&v, &mac_desktop()));
        assert!(!s.contains("--privileged"), "Desktop 上不给特权");
        assert!(!s.contains("/:/host"), "挂的是 VM 的根，没意义");
        assert!(s.contains("DEVSYS_SUDO=1"), "容器内 root 仍然给");
        assert!(warns(&v, &mac_desktop()).iter().any(|w| w.contains("给不了宿主访问")));
    }

    // ★ 档 0 也容器化了 —— 于是「借道」在 macOS 上第一次能用。
    #[test]
    fn tier0_is_a_container_too_so_it_works_on_mac() {
        let v = view_sh(&[("pub", 0)], &[("dan", KEY_A, "pub")], ctr());
        let s = joined(&cmds(&v, &mac_desktop()));
        assert!(s.contains("--name devsys-dan"));
        assert!(s.contains("DEVSYS_SHELL=/usr/sbin/nologin"), "档0 容器里也没有 shell");
        assert!(s.contains("DEVSYS_SUDO=0"));
        assert!(!s.contains("--gpus") && !s.contains("/data/imagenet"), "纯跳板不给 GPU、不挂数据集");
        // 公钥带 restrict，只放行端口转发（base64 后进 env，所以查 desired 那一侧）
        let (d, _) = desired(&v, "gpu").unwrap();
        assert!(d.users[0].key.starts_with("command=\"/bin/false\",restrict,port-forwarding "));
    }

    // ★ 端口/档位/公钥变了才重建；否则只热改限额 —— 重建会把人踢下线。
    #[test]
    fn only_recreates_when_stamp_changes() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], ctr());
        let (d, _) = desired(&v, "gpu").unwrap();
        let good = stamp(d.users[0].port, d.users[0].tier, &d.users[0].key);

        // 已存在且指纹一致 → 不重建
        let mut o = linux_root();
        o.image_ready = true;
        o.containers.insert("devsys-alice".into(), ObservedContainer { exists: true, running: true, stamp: good });
        let s = joined(&container_commands(&d, &o).unwrap().0);
        assert!(!s.contains("rm -f devsys-alice"), "指纹没变不该重建:{s}");
        assert!(!s.contains("docker run"), "不该重跑");
        assert!(s.contains("docker update"), "应热改限额");

        // 指纹变了 → 重建
        let mut o2 = o.clone();
        o2.containers.insert("devsys-alice".into(), ObservedContainer { exists: true, running: true, stamp: "p9999t1kdead".into() });
        let s2 = joined(&container_commands(&d, &o2).unwrap().0);
        assert!(s2.contains("rm -f devsys-alice"));
        assert!(s2.contains("--name devsys-alice"));
    }

    // 镜像已在就别重建（省掉一整轮 apt-get）。
    #[test]
    fn skips_image_build_when_present() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], ctr());
        let mut o = linux_root();
        o.image_ready = true;
        let s = joined(&cmds(&v, &o));
        assert!(!s.contains("apt-get"));
        assert!(!s.contains("commit"));
    }

    // GPU:Linux 按引擎分写法；mac 没有直通要明说；Windows 要提醒驱动装在 Windows 侧。
    #[test]
    fn gpu_per_platform() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], ctr());
        assert!(joined(&cmds(&v, &linux_root())).contains("--gpus all"));
        assert!(joined(&cmds(&v, &linux_rootless())).contains("--device nvidia.com/gpu=all"));

        let s = joined(&cmds(&v, &mac_desktop()));
        assert!(!s.contains("--gpus") && !s.contains("nvidia"), "mac 上不该发 GPU 参数");
        assert!(warns(&v, &mac_desktop()).iter().any(|w| w.contains("没有 GPU 直通")));
        assert!(warns(&v, &windows_desktop()).iter().any(|w| w.contains("WSL2")));
    }

    // 数据分层:点名的只读挂进来，没点名的宿主目录不可见。
    #[test]
    fn only_named_datasets_are_mounted_readonly() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], ctr());
        let s = joined(&cmds(&v, &linux_root()));
        assert!(s.contains("-v /data/imagenet:/data/imagenet:ro"));
        assert!(!s.contains("-v /:/host"), "档1 看不见宿主根");
    }

    #[test]
    fn container_without_limit_warns() {
        let sh = Sharing { isolation: "container".into(), ..Default::default() };
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], sh);
        let s = joined(&cmds(&v, &linux_root()));
        assert!(!s.contains("--cpus") && !s.contains("--memory"));
        assert!(warns(&v, &linux_root()).iter().any(|w| w.contains("借出上限")));
    }

    #[test]
    fn rootless_gets_linger_but_rootful_does_not() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], ctr());
        assert!(joined(&cmds(&v, &linux_rootless())).contains("loginctl enable-linger"));
        assert!(!joined(&cmds(&v, &linux_root())).contains("loginctl"));
        // Docker Desktop 托管容器，不跟 SSH 会话走 —— 不需要 linger
        assert!(!joined(&cmds(&v, &mac_desktop())).contains("loginctl"));
    }

    #[test]
    fn no_engine_is_a_clear_error() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], ctr());
        let (d, _) = desired(&v, "gpu").unwrap();
        let e = container_commands(&d, &Observed { os: "linux".into(), ..Default::default() }).unwrap_err();
        assert!(e.contains("docker") && e.contains("podman"));
    }

    #[test]
    fn windows_is_marked_untested() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], ctr());
        assert!(warns(&v, &windows_desktop()).iter().any(|w| w.contains("没实测")));
    }

    // 端口分配稳定，两端（下发/消费）同一个纯函数算。
    #[test]
    fn ports_are_stable_and_configurable() {
        let v = view_sh(&[("core", 2), ("member", 1)],
                        &[("alice", KEY_A, "core"), ("bob", KEY_A, "member")], ctr());
        let m = v.machines.iter().find(|m| m.name == "gpu").unwrap();
        assert_eq!(v.container_port(m, "alice"), Some(2200));
        assert_eq!(v.container_port(m, "bob"), Some(2201));

        let sh = Sharing { port_base: Some(9000), ..ctr() };
        let v2 = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], sh);
        let m2 = v2.machines.iter().find(|m| m.name == "gpu").unwrap();
        assert_eq!(v2.container_port(m2, "alice"), Some(9000));
    }

    // ── 注入防御:主人写的镜像/限额/路径也进宿主命令行 ──────
    #[test]
    fn rejects_injection_in_sharing_fields() {
        let bad = [
            Sharing { isolation: "container".into(), image: "ubuntu; rm -rf /".into(), ..Default::default() },
            Sharing { isolation: "container".into(), image: "$(id)".into(), ..Default::default() },
            Sharing { isolation: "container".into(), limit: Some(ShareLimit { cpus: None, mem: "32g; id".into(), gpus: String::new() }), ..Default::default() },
            Sharing { isolation: "container".into(), limit: Some(ShareLimit { cpus: None, mem: String::new(), gpus: "all`id`".into() }), ..Default::default() },
            Sharing { isolation: "container".into(), limit: Some(ShareLimit { cpus: Some(-1.0), mem: String::new(), gpus: String::new() }), ..Default::default() },
            Sharing { isolation: "container".into(), data: vec![ShareData { host: "/d'; id; '".into(), mount_as: String::new(), mode: "ro".into() }], ..Default::default() },
            Sharing { isolation: "container".into(), data: vec![ShareData { host: "relative/path".into(), mount_as: String::new(), mode: "ro".into() }], ..Default::default() },
            Sharing { isolation: "container".into(), data: vec![ShareData { host: "/a:/b".into(), mount_as: String::new(), mode: "ro".into() }], ..Default::default() },
        ];
        for sh in bad {
            let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], sh.clone());
            assert!(desired(&v, "gpu").is_err(), "应拒绝: {sh:?}");
        }
    }

    #[test]
    fn rejects_shell_injection_in_username() {
        for bad in ["alice'; rm -rf /;'", "root ALL", "a b", "Alice", "1alice", "a".repeat(33).as_str()] {
            let v = view_sh(&[("member", 1)], &[(bad, KEY_A, "member")], ctr());
            assert!(desired(&v, "gpu").is_err(), "应拒绝: {bad}");
        }
    }

    #[test]
    fn rejects_injection_in_pubkey() {
        for bad in [
            "ssh-ed25519 AAAA'; rm -rf / ;'",
            "ssh-ed25519 AAAA\nroot ALL=(ALL) NOPASSWD",
            "ssh-ed25519 AAAA`whoami`",
            "ssh-ed25519 AAAA$(id)",
            "not-a-key AAAA",
            "ssh-ed25519",
        ] {
            let v = view_sh(&[("member", 1)], &[("alice", bad, "member")], ctr());
            assert!(desired(&v, "gpu").is_err(), "应拒绝: {bad:?}");
        }
    }

    #[test]
    fn member_without_pubkey_is_skipped_with_warning() {
        let v = view_sh(&[("member", 1)], &[("alice", "", "member"), ("bob", KEY_A, "member")], ctr());
        let (d, w) = desired(&v, "gpu").unwrap();
        assert_eq!(d.users.len(), 1);
        assert_eq!(d.users[0].name, "bob");
        assert!(w.iter().any(|x| x.contains("alice") && x.contains("没有公钥")));
    }

    // base64 得对 —— 公钥是靠它进 env 的，错了队友就登不进去。
    #[test]
    fn b64_roundtrips() {
        for s in ["a", "ab", "abc", "abcd", KEY_A, "command=\"/bin/false\",restrict,port-forwarding x"] {
            let e = b64(s);
            assert!(e.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')));
            let out = std::process::Command::new("base64")
                .arg("-d")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut c| {
                    use std::io::Write;
                    c.stdin.as_mut().unwrap().write_all(e.as_bytes())?;
                    c.wait_with_output()
                })
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), s, "base64 解回来不一样: {s}");
        }
    }

    // ── 裸机账号档（Linux only 备选）────────────────────────
    #[test]
    fn account_mode_still_works_as_linux_fallback() {
        let v = view(&[("core", 2), ("member", 1), ("pub", 0)],
                     &[("alice", KEY_A, "core"), ("bob", KEY_A, "member"), ("dan", KEY_A, "pub")]);
        let p = plan(&v, "gpu", None).unwrap();
        assert_eq!(p.isolation, "account");
        assert!(p.commands.is_empty());
        let acct = |n: &str| p.accounts.iter().find(|a| a.name == n).unwrap();
        assert!(acct("alice").sudo);
        assert!(!acct("bob").sudo);
        assert_eq!(acct("dan").mode, "forward");
        assert!(p.script.contains("useradd -m -s /bin/bash 'alice'"));
        assert!(p.script.contains("restrict,port-forwarding"));
        assert!(p.script.contains(r#"[ "$(uname -s)" = Linux ]"#), "非 Linux 要在开头挡住");
    }

    #[test]
    fn container_mode_requires_probing_first() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], ctr());
        assert!(plan(&v, "gpu", None).unwrap_err().contains("探测"));
    }

    // 裸机账号那段仍是要以 root 跑的 shell —— 让 shell 自己判语法，别靠眼睛。
    #[cfg(unix)]
    #[test]
    fn account_script_is_valid_shell() {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let v = view(&[("core", 2), ("member", 1), ("pub", 0)],
                     &[("alice", KEY_A, "core"), ("bob", KEY_A, "member"), ("dan", KEY_A, "pub")]);
        let script = plan(&v, "gpu", None).unwrap().script;
        let mut c = Command::new("sh").arg("-n").stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        c.stdin.as_mut().unwrap().write_all(script.as_bytes()).unwrap();
        let out = c.wait_with_output().unwrap();
        assert!(out.status.success(), "语法不过:\n{}", String::from_utf8_lossy(&out.stderr));
    }

    // 眼睛看一遍要发的命令:`cargo test dump_commands -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump_commands() {
        let v = view_sh(&[("core", 2), ("member", 1), ("pub", 0)],
                        &[("alice", KEY_A, "core"), ("bob", KEY_A, "member"), ("dan", KEY_A, "pub")], ctr());
        for (name, o) in [("Linux+root", linux_root()), ("Linux rootless", linux_rootless()), ("macOS Docker Desktop", mac_desktop())] {
            println!("\n########## {name} ##########");
            for c in cmds(&v, &o) {
                println!("# {}{}", c.label, if c.optional { "（可失败）" } else { "" });
                println!("$ {}", c.line());
                if let Some(s) = &c.stdin {
                    println!("  <stdin: {} 字节>", s.len());
                }
            }
            for w in warns(&v, &o) {
                println!("⚠ {w}");
            }
        }
    }
}
