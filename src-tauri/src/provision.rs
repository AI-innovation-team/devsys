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
use serde::Serialize;

use crate::team::{ShareData, ShareLimit, Sharing, TeamView};

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
    pub script: String,
    pub warnings: Vec<String>,
    // 兑现方式:"account" | "container"
    pub isolation: String,
    // 借出资源池（父 cgroup）的人话描述。空 = 主人没设上限。
    pub pool: String,
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
pub fn plan(view: &TeamView, server: &str) -> Result<ProvisionPlan, String> {
    let machine = view
        .machines
        .iter()
        .find(|m| m.name == server)
        .ok_or_else(|| format!("团队里没有机器 {server}"))?;

    let mut warnings = Vec::new();
    let cands = candidates(view, machine, &mut warnings)?;
    if machine.sharing.is_rootless() {
        let ports = view.rootless_ports(machine);
        plan_rootless(server, &machine.sharing, cands, ports, warnings)
    } else if machine.sharing.is_container() {
        plan_container(server, &machine.sharing, cands, warnings)
    } else {
        plan_account(server, cands, warnings)
    }
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
        warnings,
        isolation: "account".into(),
        pool: String::new(),
        datasets: vec![],
    })
}

// ── 兑现方式 ②：一人一容器（v2）──────────────────────────
//
// 为什么是 per-user 而不是「共用一个容器内分权限」:容器的特权/GPU/配额都是**容器级**的,
// 共用容器给不了「张三档2、李四档1」。逐人容器才能逐人设能力,顺带拿到:
// per-user cgroup 天然配额（一人占不满饿死别人）· 爆炸半径小（焚自己的不波及别人）
// · 身份到人 = 容器到人（担责链清晰）。成本低:镜像层共享，空闲容器≈免费。
const BASE_IMAGE: &str = "devsys/base:1";

// 主人写进 team.yaml 的那几个值，校验后拼成 docker/podman 参数。
// 它们都会进到在别人机器上执行的脚本里 —— 一律**拒绝**可疑输入，不做转义兜底。
struct Flags {
    image: String,
    limits: String,      // --cpus/--memory/--gpus（docker 写法）
    limits_nogpu: String, // 同上但不含 GPU（rootless 下 GPU 要按引擎换写法）
    gpus: String,        // 主人声明的 GPU 值（原样）
    update: String,      // docker update 能改的那部分
    pool: String,        // 限额的人话（也是父池描述）
    data: String,        // -v 挂载参数
    datasets: Vec<String>,
}

fn sharing_flags(sh: &Sharing, default_image: &str, warnings: &mut Vec<String>) -> Result<Flags, String> {
    let image = if sh.image.is_empty() { default_image.to_string() } else { sh.image.clone() };
    if !valid_image(&image) {
        return Err(format!("镜像名 {image:?} 不合法（只允许字母数字与 . _ - / : @）"));
    }
    let (mut limits, mut update, mut gpus, mut pool) = (String::new(), String::new(), String::new(), String::new());
    if let Some(l) = &sh.limit {
        if let Some(c) = l.cpus {
            if !(c.is_finite() && c > 0.0 && c <= 4096.0) {
                return Err(format!("借出上限 cpus={c} 不合法"));
            }
            limits.push_str(&format!(" --cpus {c:.2}"));
            update.push_str(&format!(" --cpus {c:.2}"));
        }
        if !l.mem.is_empty() {
            if !valid_mem(&l.mem) {
                return Err(format!("借出上限 mem={:?} 不合法（应形如 32g / 4096m）", l.mem));
            }
            limits.push_str(&format!(" --memory {}", l.mem));
            update.push_str(&format!(" --memory {}", l.mem));
        }
        if !l.gpus.is_empty() {
            if !valid_gpus(&l.gpus) {
                return Err(format!("借出上限 gpus={:?} 不合法（应形如 all / 2 / device=0,1）", l.gpus));
            }
            gpus = l.gpus.clone();
        }
        pool = limits_text(l);
    }
    let limits_nogpu = limits.clone();
    if !gpus.is_empty() {
        limits.push_str(&format!(" --gpus {gpus}"));
    }

    // 点名共享的数据集（只读挂进每个容器）。
    let mut data = String::new();
    let mut datasets = Vec::new();
    for d in &sh.data {
        let ShareData { host, mount_as, mode } = d;
        let inner = if mount_as.is_empty() { host } else { mount_as };
        if !valid_path(host) || !valid_path(inner) {
            return Err(format!("共享数据集路径 {host:?} → {inner:?} 不合法（必须绝对路径，且不含空格/冒号/引号/..）"));
        }
        let ro = mode != "rw";
        data.push_str(&format!(" -v '{host}':'{inner}':{}", if ro { "ro" } else { "rw" }));
        datasets.push(format!("{host} → {inner}（{}）", if ro { "只读" } else { "可写" }));
        if !ro {
            warnings.push(format!("数据集 {host} 是**可写**挂载 —— 借用者能改你的原始数据，确认这是你要的。"));
        }
    }
    Ok(Flags { image, limits, limits_nogpu, gpus, update, pool, data, datasets })
}

fn plan_container(
    server: &str,
    sh: &Sharing,
    cands: Vec<Cand>,
    mut warnings: Vec<String>,
) -> Result<ProvisionPlan, String> {
    let f = sharing_flags(sh, BASE_IMAGE, &mut warnings)?;
    let (image, limit_flags, update_flags, pool, data_flags, datasets) =
        (f.image, f.limits, f.update, f.pool, f.data, f.datasets);

    let mut accounts = Vec::new();
    let mut blocks = Vec::new();
    let mut shelled: Vec<String> = Vec::new(); // 进容器的人（用于 sshd 加固段）

    for c in &cands {
        if c.tier == 0 {
            accounts.push(Account {
                name: c.name.clone(), role: c.role.clone(), tier: 0, sudo: false,
                mode: "forward".into(), container: String::new(), limits: String::new(), port: 0,
            });
            blocks.push(forward_block(c));
            continue;
        }
        let cname = format!("devsys-{}", c.name);
        let vol = format!("devsys-{}-home", c.name);
        let sudo = c.tier >= 2;
        // 档 2 = 完全信任:特权容器 + 挂宿主根 → 等价宿主 root。档 1 = 关死。
        let sec = if sudo {
            " --privileged -v '/':'/host'".to_string()
        } else {
            " --security-opt no-new-privileges --pids-limit 4096".to_string()
        };
        accounts.push(Account {
            name: c.name.clone(), role: c.role.clone(), tier: c.tier, sudo,
            mode: "container".into(), container: cname.clone(), limits: pool.clone(), port: 0,
        });
        shelled.push(c.name.clone());

        blocks.push(format!(
            r#"
# ── {name}（{role}，档 {tier} · 容器 {cname}）──────────────
if id -u '{name}' >/dev/null 2>&1; then
  echo "  门房账号 {name} 已存在"
else
  useradd -m '{name}' && echo "  已建门房账号 {name}"
fi
# 登录 shell = 门房脚本:SSH 进来直接落进本人容器，拿不到宿主 shell。
usermod -s /usr/local/bin/devsys-enter '{name}'
install -d -m 700 -o '{name}' -g '{name}' "/home/{name}/.ssh"
touch "/home/{name}/.ssh/authorized_keys"
if grep -qxF '{key}' "/home/{name}/.ssh/authorized_keys"; then
  echo "  公钥已在 {name}"
else
  printf '%s\n' '{key}' >> "/home/{name}/.ssh/authorized_keys" && echo "  已装公钥 {name}"
fi
chmod 600 "/home/{name}/.ssh/authorized_keys"
chown -R '{name}':'{name}' "/home/{name}/.ssh"
rm -f /etc/sudoers.d/devsys-{name}   # 容器模式:宿主一律无 sudo（档 2 的 sudo 只在容器内）
docker volume inspect '{vol}' >/dev/null 2>&1 || docker volume create '{vol}' >/dev/null
if docker inspect '{cname}' >/dev/null 2>&1; then
  echo "  容器 {cname} 已在 —— 更新限额"
  docker update{update_flags} '{cname}' >/dev/null 2>&1 || echo "  ⚠ 限额更新失败（换镜像/换挂载需先 docker rm -f {cname} 再重跑）"
  docker start '{cname}' >/dev/null 2>&1 || true
else
  docker run -d --name '{cname}' --restart unless-stopped --hostname '{name}-{server}' \
    {pool_flag}{limit_flags}{sec}{data_flags} \
    -v '{vol}':'/home/{name}' \
    '{image}' sleep infinity >/dev/null && echo "  已建容器 {cname}"
fi
# 容器内建同名同 uid 的人：uid 对齐宿主，volume 里的文件属主才不错乱。
_uid=$(id -u '{name}')
docker exec -u 0 '{cname}' sh -c "id -u '{name}' >/dev/null 2>&1 || useradd -m -u $_uid -s /bin/bash '{name}'; chown '{name}' '/home/{name}'" >/dev/null 2>&1 \
  || echo "  ⚠ 容器内建用户失败（镜像里没有 useradd?）"
{sudo_in}"#,
            name = c.name,
            role = c.role,
            tier = c.tier,
            key = c.key,
            cname = cname,
            vol = vol,
            server = server,
            image = image,
            // 挂进父池的开关：非 systemd 的机器上 $POOLARG 会被置空，
            // 不能写死 --cgroup-parent=（空值参数 docker 会拒）。
            pool_flag = if pool.is_empty() { String::new() } else { "$POOLARG ".to_string() },
            limit_flags = limit_flags,
            update_flags = update_flags,
            sec = sec,
            data_flags = data_flags,
            sudo_in = if sudo {
                format!(
                    "docker exec -u 0 '{c}' sh -c \"command -v sudo >/dev/null 2>&1 && printf '%s ALL=(ALL) NOPASSWD:ALL\\n' '{n}' > /etc/sudoers.d/devsys && chmod 440 /etc/sudoers.d/devsys\" >/dev/null 2>&1 || true\necho \"  {n}：档 2 —— 容器内 sudo + 挂载宿主 /host（等价宿主 root）\"\n",
                    c = cname, n = c.name
                )
            } else {
                format!("echo \"  {n}：档 1 —— 容器内无特权、限额内跑、看不见宿主目录\"\n", n = c.name)
            },
        ));
    }

    let any_sudo = accounts.iter().any(|a| a.sudo);
    if accounts.is_empty() {
        warnings.push("没有任何可下发的账号 —— 要么无成员获此机授权，要么成员缺公钥。队友仍登不进来。".into());
    }
    if any_sudo {
        warnings.push("有成员是**档 2**：特权容器 + 挂载宿主 `/host` —— 等价于宿主 root。仅限核心成员。".into());
    }
    if pool.is_empty() && !shelled.is_empty() {
        warnings.push("你没设**借出上限**（share_limit）—— 容器不限 CPU/内存，一个人仍可能占满整台机。建议设一个父池上限。".into());
    }
    if sh.limit.as_ref().map(|l| !l.gpus.is_empty()).unwrap_or(false) {
        warnings.push("GPU **不受 cgroup 父池约束**（设备直通不是可分配资源）：每个容器都会拿到你声明的这组 GPU，他们之间自行竞争显存。要独占请按人分配 device=。".into());
    }
    if !shelled.is_empty() {
        warnings.push("普通容器**共享内核，不是安全边界**：适合可信队友。不可信的人 / agent 生成的代码需要强沙箱（microVM / gVisor），那是另一层，我们还没接。".into());
        warnings.push("容器用桥接网络：队友的 `ssh -L` 端口转发会打到**宿主**而不是他的容器（我们已尽量在 sshd 里关掉转发）。终态是每个容器挂一个 tailnet 节点。".into());
    }

    let script = format!(
        r#"#!/bin/sh
# DevSys 授权下发 —— 服务器 {server}（隔离方式：一人一容器）
# 由 app 生成，需以 root 执行。幂等：可重复运行。
#
# 「档」= 开多少权（team.yaml 的 grants，一字未改），「容器」= 怎么关。
# 每人一个容器：档位逐人兑现、配额逐人生效、爆炸半径只有他自己。
set -e
if [ "$(id -u)" -ne 0 ]; then echo "需要 root（请用 sudo 运行）" >&2; exit 1; fi
command -v docker >/dev/null 2>&1 || {{ echo "这台机没装 docker —— 装好再下发，或把隔离方式改回「裸机账号」" >&2; exit 1; }}
docker info >/dev/null 2>&1 || {{ echo "docker 守护进程没在跑（systemctl start docker）" >&2; exit 1; }}
NOLOGIN=/usr/sbin/nologin
[ -x "$NOLOGIN" ] || NOLOGIN=/sbin/nologin
[ -x "$NOLOGIN" ] || NOLOGIN=/bin/false
POOL=devsys-shared.slice
echo "下发到 {server}："
{pool_block}{image_block}
# ── 门房脚本：SSH 落进本人容器 ────────────────────────────
# 装成借用者的**登录 shell**（不依赖 sshd 配置，任何发行版都生效）。
cat > /usr/local/bin/devsys-enter <<'DEVSYS_ENTER'
#!/bin/sh
u=$(id -un)
c="devsys-$u"
docker start "$c" >/dev/null 2>&1 || {{ echo "你的容器 $c 不在 —— 请机器主人重跑一次授权下发。" >&2; exit 1; }}
t=""
[ -t 0 ] && t="-t"
if [ "$1" = "-c" ]; then
  exec docker exec -i $t -u "$u" -w "/home/$u" "$c" bash -lc "$2"
fi
exec docker exec -i $t -u "$u" -w "/home/$u" "$c" bash -l
DEVSYS_ENTER
chmod 755 /usr/local/bin/devsys-enter
grep -qxF /usr/local/bin/devsys-enter /etc/shells 2>/dev/null || echo /usr/local/bin/devsys-enter >> /etc/shells
echo "  门房脚本已装 /usr/local/bin/devsys-enter"
{blocks}{sshd_block}
echo "完成。"
"#,
        server = server,
        // 借出资源池：所有借用容器挂它下面，cgroup 层级保证加起来永不超这个上限。
        // 主人随时可改 / 设 0 收回 —— 责任为门。
        pool_block = if pool.is_empty() {
            String::new()
        } else {
            format!(
                r#"
# ── 借出资源池（父 cgroup）：{pool} ────────────────────
# 所有借用容器都挂在它下面 —— 开多少个容器，加起来都突破不了这个上限。
POOLARG="--cgroup-parent=$POOL"
if [ -d /run/systemd/system ]; then
  cat > "/etc/systemd/system/$POOL" <<EOF
[Unit]
Description=DevSys 借出算力池（团队借用容器的总上限）

[Slice]
{slice_props}
EOF
  systemctl daemon-reload
  systemctl start "$POOL" || echo "  ⚠ 资源池启动失败 —— 容器仍会建，但没有总上限"
  echo "  借出资源池 $POOL：{pool}"
else
  echo "  ⚠ 这台机没有 systemd —— 跳过父资源池，只有逐容器限额"
  POOLARG=""
fi
"#,
                pool = pool,
                slice_props = {
                    let l = sh.limit.as_ref().unwrap();
                    let mut p = Vec::new();
                    if let Some(c) = l.cpus {
                        p.push(format!("CPUQuota={}%", (c * 100.0).round() as i64));
                    }
                    if !l.mem.is_empty() {
                        p.push(format!("MemoryMax={}", l.mem.to_uppercase()));
                    }
                    if p.is_empty() { "# （未设 CPU/内存上限）".into() } else { p.join("\n") }
                },
            )
        },
        image_block = if sh.image.is_empty() {
            format!(
                r#"
# ── 基础镜像（带 bash/tmux/git —— 工作区的持久会话靠容器里的 tmux）──
if docker image inspect '{image}' >/dev/null 2>&1; then
  echo "  基础镜像 {image} 已在"
else
  echo "  构建基础镜像 {image}（首次较慢）…"
  docker build -t '{image}' - <<'DEVSYS_DOCKERFILE'
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      bash tmux git curl ca-certificates sudo openssh-client python3 less vim-tiny \
    && rm -rf /var/lib/apt/lists/*
DEVSYS_DOCKERFILE
fi
"#,
                image = image
            )
        } else {
            format!("\necho \"  用主人指定的镜像 {image}（若里面没有 tmux，工作区就不持久）\"\ndocker image inspect '{image}' >/dev/null 2>&1 || docker pull '{image}'\n")
        },
        blocks = blocks.join(""),
        // 加固（尽力而为）:容器用户不该能往宿主打端口转发。装不上就明说，不假装。
        sshd_block = if shelled.is_empty() {
            String::new()
        } else {
            format!(
                r#"
# ── 加固（尽力而为）:挡掉借用者向**宿主**的端口转发/X11 ──────
if [ -d /etc/ssh/sshd_config.d ] && grep -qE '^[[:space:]]*Include[[:space:]]+/etc/ssh/sshd_config\.d/' /etc/ssh/sshd_config 2>/dev/null; then
  cat > /etc/ssh/sshd_config.d/50-devsys-container.conf <<'EOF'
Match User {users}
  AllowTcpForwarding no
  X11Forwarding no
  AllowAgentForwarding no
Match all
EOF
  if sshd -t 2>/dev/null; then
    systemctl reload ssh 2>/dev/null || systemctl reload sshd 2>/dev/null || true
    echo "  已挡掉借用者到宿主的端口转发"
  else
    rm -f /etc/ssh/sshd_config.d/50-devsys-container.conf
    echo "  ⚠ sshd 配置校验未过 —— 已回滚加固段（容器隔离本身不受影响）"
  fi
else
  echo "  ⚠ 这台机的 sshd 不吃 sshd_config.d —— 未加固端口转发（容器隔离本身不受影响）"
fi
"#,
                users = shelled.join(",")
            )
        },
    );

    Ok(ProvisionPlan {
        server: server.into(),
        accounts,
        any_sudo,
        script,
        warnings,
        isolation: "container".into(),
        pool,
        datasets,
    })
}

// ── 兑现方式 ③：一人一容器 · 免 root（v2）─────────────────
//
// 为什么要有这一档:①②都要被共享机的 root。可校园里绝大多数人对实验室 GPU 服务器
// **只有一个普通账号** —— 「自助贡献机器」如果非管理员不可,就等于只服务得了机器主人。
//
// 免 root 的关键换法:**队友不登宿主账号,直连他自己容器里的 sshd**（各占一个高位端口）。
// 于是 useradd / 改 sshd / 写 /etc 这些要 root 的事一件都不用做,全程在贡献者
// 自己的普通账号里(rootless podman 优先,回退 rootless docker)。
//
// 换来的额外性质:rootless 下容器 root 经 userns 映射到**你这个普通用户**,
// 所以即便档 2 也拿不到宿主 root —— 比要 root 的 ② 反而更安全。
const BASE_SSH_IMAGE: &str = "devsys/base-ssh:1";

fn plan_rootless(
    server: &str,
    sh: &Sharing,
    cands: Vec<Cand>,
    ports: Vec<(String, u16)>,
    mut warnings: Vec<String>,
) -> Result<ProvisionPlan, String> {
    let f = sharing_flags(sh, BASE_SSH_IMAGE, &mut warnings)?;
    let port_of = |n: &str| ports.iter().find(|(m, _)| m == n).map(|(_, p)| *p);

    let mut accounts = Vec::new();
    let mut blocks = Vec::new();
    let mut skipped_forward = 0usize;

    for c in &cands {
        if c.tier == 0 {
            // 免 root 起不了宿主账号 → 档 0「纯跳板」在这一档无法兑现，明说而不是假装。
            skipped_forward += 1;
            continue;
        }
        let Some(port) = port_of(&c.name) else { continue };
        let cname = format!("devsys-{}", c.name);
        let sudo = c.tier >= 2;
        // 档 2 在 rootless 下仍只等于「贡献者本人的权限」（userns 兜底），不是宿主 root。
        let sec = if sudo {
            " --privileged -v '/':'/host'".to_string()
        } else {
            " --security-opt no-new-privileges --pids-limit 4096".to_string()
        };
        accounts.push(Account {
            name: c.name.clone(),
            role: c.role.clone(),
            tier: c.tier,
            sudo,
            mode: "container".into(),
            container: cname.clone(),
            limits: f.pool.clone(),
            port,
        });

        blocks.push(format!(
            r#"
# ── {name}（{role}，档 {tier} · 容器 {cname} · 端口 {port}）──────────────
C={cname}
PORT={port}
# 端口变了（有人离队/加入会让排位前移）就重建 —— 否则队友会连到别人的容器上。
if $ENG inspect "$C" >/dev/null 2>&1 && [ -z "$($ENG ps -a --filter "name=^$C$" --filter "label=devsys.port=$PORT" -q)" ]; then
  echo "  $C 的端口已变 —— 重建（家目录卷保留）"
  $ENG rm -f "$C" >/dev/null 2>&1 || true
fi
$ENG volume inspect '{vol}' >/dev/null 2>&1 || $ENG volume create '{vol}' >/dev/null
$ENG volume inspect '{sshvol}' >/dev/null 2>&1 || $ENG volume create '{sshvol}' >/dev/null
if $ENG inspect "$C" >/dev/null 2>&1; then
  echo "  容器 $C 已在 —— 更新限额"
  $ENG update{update} "$C" >/dev/null 2>&1 || echo "  ⚠ 限额更新失败（换镜像/换挂载需先 $ENG rm -f $C 再重跑）"
  $ENG start "$C" >/dev/null 2>&1 || true
elif $ENG run -d --name "$C" --restart unless-stopped --hostname '{name}-{server}' \
       --label "devsys.port=$PORT" -p "$PORT":22 \
       -v '{vol}':'/home/{name}' -v '{sshvol}':'/etc/ssh'{limits} $GPUARG{sec}{data} \
       '{image}' >/dev/null 2>&1; then
  echo "  已建容器 $C（端口 $PORT）"
else
  # rootless 下 CPU/内存 cgroup 控制器默认没委派给普通用户 —— 带限额会直接起不来。
  # 宁可起来但不限额，也别让贡献者卡死在这里；但必须**说清楚限额没生效**。
  echo "  ⚠ 带限额启动失败 —— 改为**不限额**启动（rootless 的 cgroup 控制器没委派给你）"
  echo "     让机器管理员做一次即可开启限额："
  echo "     mkdir -p /etc/systemd/system/user@.service.d && printf '[Service]\nDelegate=cpu cpuset io memory pids\n' > /etc/systemd/system/user@.service.d/delegate.conf && systemctl daemon-reload"
  $ENG rm -f "$C" >/dev/null 2>&1 || true
  $ENG run -d --name "$C" --restart unless-stopped --hostname '{name}-{server}' \
    --label "devsys.port=$PORT" -p "$PORT":22 \
    -v '{vol}':'/home/{name}' -v '{sshvol}':'/etc/ssh' $GPUARG{sec}{data} \
    '{image}' >/dev/null && echo "  已建容器 $C（端口 $PORT，⚠ 无限额）"
fi
# 容器里建这个人 + 装他的公钥（每次覆盖 → 队友换钥匙重跑一次就生效）
$ENG exec -u 0 "$C" sh -c "id -u '{name}' >/dev/null 2>&1 || useradd -m -s /bin/bash '{name}'" >/dev/null 2>&1 \
  || echo "  ⚠ 容器内建用户失败（镜像里没有 useradd?）"
$ENG exec -u 0 "$C" sh -c "install -d -m 700 -o '{name}' -g '{name}' '/home/{name}/.ssh' && printf '%s\n' '{key}' > '/home/{name}/.ssh/authorized_keys' && chmod 600 '/home/{name}/.ssh/authorized_keys' && chown '{name}':'{name}' '/home/{name}/.ssh/authorized_keys'" >/dev/null \
  && echo "  已装公钥 {name}"
{sudo_in}"#,
            name = c.name,
            role = c.role,
            tier = c.tier,
            key = c.key,
            cname = cname,
            port = port,
            server = server,
            vol = format!("devsys-{}-home", c.name),
            sshvol = format!("devsys-{}-sshd", c.name),
            image = f.image,
            limits = f.limits_nogpu,
            update = f.update,
            sec = sec,
            data = f.data,
            sudo_in = if sudo {
                format!(
                    "$ENG exec -u 0 \"$C\" sh -c \"command -v sudo >/dev/null 2>&1 && printf '%s ALL=(ALL) NOPASSWD:ALL\\n' '{n}' > /etc/sudoers.d/devsys && chmod 440 /etc/sudoers.d/devsys\" >/dev/null 2>&1 || true\necho \"  {n}：档 2 —— 容器内 sudo + 挂载宿主 /host（**仍受你这个账号的权限约束**，不是宿主 root）\"\n",
                    n = c.name
                )
            } else {
                format!("echo \"  {n}：档 1 —— 容器内无特权、限额内跑、看不见宿主目录\"\n", n = c.name)
            },
        ));
    }

    let any_sudo = accounts.iter().any(|a| a.sudo);
    if accounts.is_empty() {
        warnings.push("没有任何可下发的容器 —— 要么无成员获此机授权，要么成员缺公钥。队友仍登不进来。".into());
    }
    if skipped_forward > 0 {
        warnings.push(format!(
            "有 {skipped_forward} 位成员是**档 0（纯跳板）** —— 免 root 模式建不了宿主账号，做不到「借道」，已跳过。\
             这台机要当跳板给队友用，得用「裸机账号」或「一人一容器」（都需要 root）。"
        ));
    }
    warnings.push(
        "免 root 模式下队友连的是 `<主机>:<各自端口>`，**不是 22 端口** —— app 会自动按 team.yaml 算出端口填进他们的服务器列表。手工 ssh 要自己带 -p。"
            .into(),
    );
    warnings.push(
        "容器归你的用户会话所有:脚本会试着开 linger（`loginctl enable-linger`），否则你一登出容器就被杀。开不了的话让管理员执行一次。"
            .into(),
    );
    if !accounts.is_empty() {
        warnings.push(
            "这些端口绑在**所有网卡**上（rootless 拿不到 tailnet 网卡做定向绑定）—— 这台机若有公网 IP，端口就暴露在公网。\
             容器里的 sshd 只认公钥、禁密码，但仍建议靠防火墙或 tailnet ACL 收口。"
                .into(),
        );
    }
    if f.pool.is_empty() {
        warnings.push("你没设**借出上限** —— 容器不限 CPU/内存，一个人仍可能占满整台机。".into());
    } else {
        warnings.push(
            "免 root 下**没有父 cgroup 池**（建 systemd slice 要 root）：限额是逐容器的，N 个人最多能占到 N 倍。要硬上限就得用要 root 的模式。"
                .into(),
        );
    }
    if any_sudo {
        warnings.push("有成员是**档 2**：特权容器 + 挂载宿主 `/host`。rootless 下这仍被 userns 兜住（等于你这个账号的权限，不是宿主 root），但你自己的文件对他全开。".into());
    }
    if !f.gpus.is_empty() {
        warnings.push("GPU 在 rootless 下要 podman + CDI（`nvidia-ctk cdi generate`）或配好 rootless 的 nvidia-container-toolkit；脚本会自动按引擎选写法，起不来会退回无 GPU 并提示。".into());
    }

    let script = format!(
        r#"#!/bin/sh
# DevSys 授权下发 —— 服务器 {server}（隔离方式：一人一容器 · 免 root）
# 由 app 生成，**用你自己的普通账号执行** —— 不需要 sudo，不碰 /etc，不建宿主账号。
#
# 换法:队友不登宿主账号，而是直连他自己容器里的 sshd（每人一个高位端口）。
# 于是「贡献一台机」的门槛从「我是这台机的管理员」降到「我在这台机上有个账号」。
set -e
if [ "$(id -u)" -eq 0 ]; then
  echo "别用 root 跑这个 —— 免 root 模式的意义就是跑在你自己的账号里" >&2; exit 1
fi
ENG=podman
command -v podman >/dev/null 2>&1 || ENG=docker
command -v "$ENG" >/dev/null 2>&1 || {{ echo "没找到 podman 也没找到 docker —— 装一个（推荐 podman，天生 rootless）" >&2; exit 1; }}
$ENG info >/dev/null 2>&1 || {{ echo "$ENG 跑不起来（rootless 没初始化?试 '$ENG system migrate' 或让管理员配 subuid/subgid）" >&2; exit 1; }}
echo "下发到 {server}（引擎 $ENG，免 root）："
# 登出后容器要继续活着 —— 没有 linger，你一断开 SSH 容器就被 systemd 收走。
loginctl enable-linger "$(id -un)" >/dev/null 2>&1 \
  && echo "  已开 linger（登出后容器继续跑）" \
  || echo "  ⚠ 开 linger 失败 —— 你登出后容器可能被杀，让管理员跑 loginctl enable-linger $(id -un)"
# GPU：podman 走 CDI，docker 走 --gpus
GPUARG=""
{gpu_block}{image_block}{blocks}
echo "完成。队友各自连 {server}:<他的端口>，落在自己的容器里。"
"#,
        server = server,
        gpu_block = if f.gpus.is_empty() {
            String::new()
        } else {
            format!(
                r#"if [ "$ENG" = podman ]; then
  GPUARG="--device nvidia.com/gpu={gpus}"
else
  GPUARG="--gpus {gpus}"
fi
"#,
                gpus = f.gpus
            )
        },
        image_block = if sh.image.is_empty() {
            format!(
                r#"# ── 基础镜像（sshd + tmux —— 队友直连它，工作区的持久会话也靠里面的 tmux）──
if $ENG image inspect '{image}' >/dev/null 2>&1; then
  echo "  基础镜像 {image} 已在"
else
  echo "  构建基础镜像 {image}（首次较慢）…"
  $ENG build -t '{image}' - <<'DEVSYS_DOCKERFILE'
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
      bash tmux git curl ca-certificates sudo openssh-server openssh-client python3 less vim-tiny \
    && rm -rf /var/lib/apt/lists/* /etc/ssh/ssh_host_* \
    && mkdir -p /run/sshd /etc/ssh/sshd_config.d \
    && printf 'PasswordAuthentication no\nPermitRootLogin no\nX11Forwarding no\n' > /etc/ssh/sshd_config.d/10-devsys.conf
CMD ["sh","-c","ssh-keygen -A >/dev/null 2>&1; exec /usr/sbin/sshd -D -e"]
DEVSYS_DOCKERFILE
fi
"#,
                image = f.image
            )
        } else {
            format!(
                "echo \"  用主人指定的镜像 {image}（**必须自带 sshd 并以它为 CMD**，否则队友连不进去）\"\n$ENG image inspect '{image}' >/dev/null 2>&1 || $ENG pull '{image}'\n",
                image = f.image
            )
        },
        blocks = blocks.join(""),
    );

    Ok(ProvisionPlan {
        server: server.into(),
        accounts,
        any_sudo,
        script,
        warnings,
        isolation: "rootless".into(),
        pool: f.pool,
        datasets: f.datasets,
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

    // 容器模式的 sharing：8 核 / 32g / GPU all，外加一个只读数据集。
    fn container_sharing() -> Sharing {
        Sharing {
            isolation: "container".into(),
            image: String::new(),
            limit: Some(ShareLimit { cpus: Some(8.0), mem: "32g".into(), gpus: "all".into() }),
            data: vec![ShareData { host: "/data/imagenet".into(), mount_as: String::new(), mode: "ro".into() }],
            port_base: None,
        }
    }

    // 档 0 = 纯跳板：要有账号（不然 ProxyJump 认证不了），但没 shell、只放行转发。
    #[test]
    fn pub_grant0_gets_forward_only_account() {
        let v = view(&[("pub", 0)], &[("dan", KEY_A, "pub")]);
        let p = plan(&v, "gpu").unwrap();
        assert_eq!(p.accounts.len(), 1);
        assert_eq!(p.accounts[0].mode, "forward");
        assert_eq!(p.accounts[0].tier, 0);
        assert!(!p.accounts[0].sudo);
        assert!(p.script.contains(r#"useradd -m -s "$NOLOGIN" 'dan'"#), "档0 用 nologin");
        assert!(p.script.contains("restrict,port-forwarding"), "只放行端口转发");
        assert!(!p.script.contains("useradd -m -s /bin/bash 'dan'"), "档0 不给 shell");
    }

    // 无 grant（角色根本没被授权）才是真的什么都不建。
    #[test]
    fn ungranted_role_gets_nothing() {
        let v = view(&[("core", 2)], &[("dan", KEY_A, "pub")]);
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
            &[("core", 2), ("member", 1), ("pub", 0)],
            &[("alice", KEY_A, "core"), ("bob", KEY_A, "member"), ("dan", KEY_A, "pub")],
        );
        let p = plan(&v, "gpu").unwrap();
        // alice(core)→sudo, bob(member)→无sudo, dan(pub)→仅转发
        let acct = |n: &str| p.accounts.iter().find(|a| a.name == n);
        assert!(acct("alice").unwrap().sudo, "core 拿 sudo");
        assert!(!acct("bob").unwrap().sudo, "member 无 sudo");
        assert_eq!(acct("dan").unwrap().mode, "forward", "pub(档0) 只借道");
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

    // ── 容器模式（v2：一人一容器）─────────────────────────

    // ★ 核心不变量:grants 一字不改，只是兑现从 useradd 换成 docker run。
    #[test]
    fn container_mode_gives_each_person_own_container() {
        let v = view_sh(
            &[("core", 2), ("member", 1)],
            &[("alice", KEY_A, "core"), ("bob", KEY_A, "member")],
            container_sharing(),
        );
        let p = plan(&v, "gpu").unwrap();
        assert_eq!(p.isolation, "container");
        let acct = |n: &str| p.accounts.iter().find(|a| a.name == n).unwrap();
        // 一人一个容器 —— 不是共用一个容器内分权限
        assert_eq!(acct("alice").container, "devsys-alice");
        assert_eq!(acct("bob").container, "devsys-bob");
        assert!(p.script.contains("--name 'devsys-alice'"));
        assert!(p.script.contains("--name 'devsys-bob'"));
        // 各自独立 volume（焚容器也留住自己的活）
        assert!(p.script.contains("-v 'devsys-alice-home':'/home/alice'"));
        assert!(p.script.contains("-v 'devsys-bob-home':'/home/bob'"));
    }

    // ★ 档位 → 容器能力:档2 特权+挂宿主，档1 关死。这正是共用容器给不了的。
    #[test]
    fn container_tier_maps_to_capabilities() {
        let v = view_sh(
            &[("core", 2), ("member", 1)],
            &[("alice", KEY_A, "core"), ("bob", KEY_A, "member")],
            container_sharing(),
        );
        let p = plan(&v, "gpu").unwrap();
        let alice = p.script.split("devsys-alice").nth(1).unwrap_or("");
        assert!(p.script.contains("--privileged -v '/':'/host'"), "档2 = 特权 + 宿主可见");
        assert!(p.script.contains("--security-opt no-new-privileges"), "档1 = 关死");
        let _ = alice;
        assert!(p.any_sudo);
        // 宿主一律无 sudo —— 档2 的 sudo 只在容器里
        assert!(!p.script.contains("NOPASSWD:ALL\\n' 'alice' > /etc/sudoers.d/devsys-alice"));
        assert!(p.script.contains("rm -f /etc/sudoers.d/devsys-alice"));
    }

    // ★ 主人的借出上限 = 父 cgroup 池：开多少容器都突破不了。
    #[test]
    fn share_limit_becomes_parent_cgroup_pool() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], container_sharing());
        let p = plan(&v, "gpu").unwrap();
        assert_eq!(p.pool, "8 核 · 32g · GPU all");
        assert!(p.script.contains("CPUQuota=800%"));
        assert!(p.script.contains("MemoryMax=32G"));
        assert!(p.script.contains("POOLARG=\"--cgroup-parent=$POOL\""));
        assert!(p.script.contains("docker run -d --name 'devsys-alice' --restart unless-stopped --hostname 'alice-gpu' \\\n    $POOLARG "), "容器挂进父池");
        assert!(p.script.contains("--cpus 8.00"));
        assert!(p.script.contains("--memory 32g"));
        assert!(p.script.contains("--gpus all"));
        // GPU 不受父池约束 —— 必须说清楚，别让主人以为 cgroup 管得住显卡
        assert!(p.warnings.iter().any(|w| w.contains("GPU") && w.contains("不受")));
    }

    // 没设上限 = 容器不限量 —— 要明说，不能让主人误以为容器自带保护。
    #[test]
    fn container_without_limit_warns() {
        let sh = Sharing { isolation: "container".into(), ..Default::default() };
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], sh);
        let p = plan(&v, "gpu").unwrap();
        assert!(p.pool.is_empty());
        assert!(!p.script.contains("cgroup-parent"));
        assert!(p.warnings.iter().any(|w| w.contains("借出上限")));
    }

    // ★ 数据分层：点名的数据集只读挂进来；没点名的宿主目录一律不可见（档1 不挂宿主 FS）。
    #[test]
    fn shared_dataset_mounts_readonly_and_host_stays_invisible() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], container_sharing());
        let p = plan(&v, "gpu").unwrap();
        assert_eq!(p.datasets, vec!["/data/imagenet → /data/imagenet（只读）"]);
        assert!(p.script.contains("-v '/data/imagenet':'/data/imagenet':ro"));
        assert!(!p.script.contains("-v '/':'/host'"), "档1 看不见宿主根");
    }

    // 门房:SSH 进来直接落进本人容器，拿不到宿主 shell（不依赖 sshd 配置，靠登录 shell）。
    #[test]
    fn container_users_get_doorman_shell_not_host_shell() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], container_sharing());
        let p = plan(&v, "gpu").unwrap();
        assert!(p.script.contains("usermod -s /usr/local/bin/devsys-enter 'alice'"));
        assert!(!p.script.contains("useradd -m -s /bin/bash 'alice'"), "宿主不给 bash");
        // 门房要透传 SSH 带来的命令 —— 否则工作区的 tmux 起不到容器里
        assert!(p.script.contains("bash -lc \"$2\""));
        assert!(p.script.contains("tmux"), "基础镜像要带 tmux，工作区才持久");
    }

    // 容器模式下档 0 仍然只是门（不给容器）。
    #[test]
    fn container_mode_tier0_still_forward_only() {
        let v = view_sh(&[("pub", 0)], &[("dan", KEY_A, "pub")], container_sharing());
        let p = plan(&v, "gpu").unwrap();
        assert_eq!(p.accounts[0].mode, "forward");
        assert!(p.accounts[0].container.is_empty());
        assert!(!p.script.contains("--name 'devsys-dan'"), "档0 不给容器");
        assert!(!p.script.contains("usermod -s /usr/local/bin/devsys-enter 'dan'"));
    }

    // ── 免 root 模式（一人一容器 · 不需要被共享机的 root）────

    fn rootless_sharing() -> Sharing {
        Sharing { isolation: "rootless".into(), ..container_sharing() }
    }

    // ★ 全程不碰 root:不 useradd、不写 /etc、不改 sshd。队友直连容器里的 sshd。
    #[test]
    fn rootless_never_touches_host_root() {
        let v = view_sh(
            &[("core", 2), ("member", 1)],
            &[("alice", KEY_A, "core"), ("bob", KEY_A, "member")],
            rootless_sharing(),
        );
        let p = plan(&v, "gpu").unwrap();
        assert_eq!(p.isolation, "rootless");
        // 宿主上一个 root 动作都不该有。注意「容器里的 /etc」是合法的 ——
        // 它在 `$ENG exec` 里，属于容器自己的文件系统，跟宿主无关。
        let mut in_dockerfile = false;
        for (i, line) in p.script.lines().enumerate() {
            let l = line.trim_start();
            // Dockerfile 是**镜像**的构建指令，不是在宿主上执行的命令（里面的 sudo 只是个包名）。
            if l.contains("DEVSYS_DOCKERFILE") {
                in_dockerfile = !in_dockerfile;
                continue;
            }
            if in_dockerfile || l.starts_with('#') || l.starts_with("echo ") || l.contains("$ENG exec") {
                continue;
            }
            for forbidden in ["useradd ", "usermod ", "/etc/sudoers", "/etc/ssh/", "systemctl ", "sudo "] {
                assert!(
                    !l.contains(forbidden),
                    "免 root 脚本第 {} 行动了宿主({forbidden:?}): {line}",
                    i + 1
                );
            }
        }
        // 反过来:必须明确拒绝被 root 执行
        assert!(p.script.contains(r#"if [ "$(id -u)" -eq 0 ]"#));
    }

    // ★ 每人一个高位端口,下发侧与消费侧用同一个纯函数算 —— 端口不一致会静默连不上。
    #[test]
    fn rootless_assigns_stable_per_member_ports() {
        let v = view_sh(
            &[("core", 2), ("member", 1)],
            &[("alice", KEY_A, "core"), ("bob", KEY_A, "member")],
            rootless_sharing(),
        );
        let m = v.machines.iter().find(|m| m.name == "gpu").unwrap();
        // 成员按名排序 → alice 先、bob 后
        assert_eq!(v.rootless_port(m, "alice"), Some(2200));
        assert_eq!(v.rootless_port(m, "bob"), Some(2201));

        let p = plan(&v, "gpu").unwrap();
        let acct = |n: &str| p.accounts.iter().find(|a| a.name == n).unwrap();
        assert_eq!(acct("alice").port, 2200);
        assert_eq!(acct("bob").port, 2201);
        assert!(p.script.contains("PORT=2200"));
        assert!(p.script.contains("PORT=2201"));
        assert!(p.script.contains(r#"-p "$PORT":22"#));
        // 端口漂移要能自愈,否则队友会连到别人的容器
        assert!(p.script.contains("label=devsys.port=$PORT"));
    }

    #[test]
    fn rootless_port_base_is_configurable() {
        let sh = Sharing { port_base: Some(9000), ..rootless_sharing() };
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], sh);
        let m = v.machines.iter().find(|m| m.name == "gpu").unwrap();
        assert_eq!(v.rootless_port(m, "alice"), Some(9000));
    }

    // 免 root 建不了宿主账号 → 档 0「借道」做不到。必须明说,不能假装成功。
    #[test]
    fn rootless_cannot_do_tier0_and_says_so() {
        let v = view_sh(
            &[("member", 1), ("pub", 0)],
            &[("alice", KEY_A, "member"), ("dan", KEY_A, "pub")],
            rootless_sharing(),
        );
        let p = plan(&v, "gpu").unwrap();
        assert!(p.accounts.iter().all(|a| a.name != "dan"));
        assert!(p.warnings.iter().any(|w| w.contains("档 0") && w.contains("跳过")));
    }

    // 没有父 cgroup 池（建 slice 要 root）—— 限额只到逐容器，得说清 N 人 N 倍。
    #[test]
    fn rootless_has_no_parent_pool_and_says_so() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], rootless_sharing());
        let p = plan(&v, "gpu").unwrap();
        assert!(!p.script.contains("cgroup-parent"));
        assert!(!p.script.contains("devsys-shared.slice"));
        assert!(p.warnings.iter().any(|w| w.contains("没有父 cgroup 池")));
        // 限额仍逐容器下,且带限额失败要退回不限额并说明
        assert!(p.script.contains("--cpus 8.00"));
        assert!(p.script.contains("⚠ 无限额"));
    }

    // GPU 在 rootless 下写法按引擎分（podman 用 CDI）。
    #[test]
    fn rootless_gpu_uses_cdi_for_podman() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], rootless_sharing());
        let p = plan(&v, "gpu").unwrap();
        assert!(p.script.contains("--device nvidia.com/gpu=all"));
        assert!(p.script.contains("--gpus all"));
    }

    // 镜像必须自带 sshd —— 队友是直连它的。
    #[test]
    fn rootless_image_ships_sshd_and_tmux() {
        let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], rootless_sharing());
        let p = plan(&v, "gpu").unwrap();
        assert!(p.script.contains("openssh-server"));
        assert!(p.script.contains("tmux"));
        assert!(p.script.contains("/usr/sbin/sshd -D -e"));
        // 登出后容器要活着
        assert!(p.script.contains("loginctl enable-linger"));
    }

    // ── 注入防御：主人写的镜像/限额/路径也进 root 脚本 ──────
    #[test]
    fn rejects_injection_in_sharing_fields() {
        let bad_shares = [
            Sharing { isolation: "container".into(), image: "ubuntu; rm -rf /".into(), ..Default::default() },
            Sharing { isolation: "container".into(), image: "$(id)".into(), ..Default::default() },
            Sharing { isolation: "container".into(), limit: Some(ShareLimit { cpus: None, mem: "32g; id".into(), gpus: String::new() }), ..Default::default() },
            Sharing { isolation: "container".into(), limit: Some(ShareLimit { cpus: None, mem: String::new(), gpus: "all`id`".into() }), ..Default::default() },
            Sharing { isolation: "container".into(), limit: Some(ShareLimit { cpus: Some(-1.0), mem: String::new(), gpus: String::new() }), ..Default::default() },
            Sharing { isolation: "container".into(), data: vec![ShareData { host: "/d'; id; '".into(), mount_as: String::new(), mode: "ro".into() }], ..Default::default() },
            Sharing { isolation: "container".into(), data: vec![ShareData { host: "relative/path".into(), mount_as: String::new(), mode: "ro".into() }], ..Default::default() },
            Sharing { isolation: "container".into(), data: vec![ShareData { host: "/a:/b".into(), mount_as: String::new(), mode: "ro".into() }], ..Default::default() },
        ];
        for sh in bad_shares {
            let v = view_sh(&[("member", 1)], &[("alice", KEY_A, "member")], sh.clone());
            assert!(plan(&v, "gpu").is_err(), "应拒绝: {sh:?}");
        }
    }

    // ★ 生成的脚本要以 **root** 在别人的机器上跑 —— 语法错=半途炸在中间，
    // 账号建了一半、容器没建。让 shell 自己来判语法，别靠眼睛看。
    #[cfg(unix)]
    fn assert_valid_sh(script: &str, what: &str) {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut c = Command::new("sh")
            .arg("-n")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("起不了 sh");
        c.stdin.as_mut().unwrap().write_all(script.as_bytes()).unwrap();
        let out = c.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{what} 的脚本语法不过:\n{}\n──── 脚本 ────\n{script}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[cfg(unix)]
    #[test]
    fn generated_scripts_are_valid_shell() {
        let members = [("alice", KEY_A, "core"), ("bob", KEY_A, "member"), ("dan", KEY_A, "pub")];
        let grants = [("core", 2), ("member", 1), ("pub", 0)];
        assert_valid_sh(&plan(&view(&grants, &members), "gpu").unwrap().script, "裸机账号");
        assert_valid_sh(
            &plan(&view_sh(&grants, &members, container_sharing()), "gpu").unwrap().script,
            "一人一容器",
        );
        // 不设上限 / 自带镜像的分支也要过
        let bare = Sharing { isolation: "container".into(), image: "nvcr.io/nvidia/pytorch:24.05-py3".into(), ..Default::default() };
        assert_valid_sh(&plan(&view_sh(&grants, &members, bare), "gpu").unwrap().script, "自带镜像");
        // 免 root 的三个分支
        assert_valid_sh(&plan(&view_sh(&grants, &members, rootless_sharing()), "gpu").unwrap().script, "免 root");
        let rl_bare = Sharing { isolation: "rootless".into(), ..Default::default() };
        assert_valid_sh(&plan(&view_sh(&grants, &members, rl_bare), "gpu").unwrap().script, "免 root · 无上限");
        let rl_img = Sharing { isolation: "rootless".into(), image: "myorg/withsshd:1".into(), ..Default::default() };
        assert_valid_sh(&plan(&view_sh(&grants, &members, rl_img), "gpu").unwrap().script, "免 root · 自带镜像");
    }

    // 眼睛看一遍生成的容器脚本:`cargo test dump_container_script -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump_container_script() {
        let v = view_sh(
            &[("core", 2), ("member", 1), ("pub", 0)],
            &[("alice", KEY_A, "core"), ("bob", KEY_A, "member"), ("dan", KEY_A, "pub")],
            container_sharing(),
        );
        println!("{}", plan(&v, "gpu").unwrap().script);
    }

    // `cargo test dump_rootless_script -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump_rootless_script() {
        let v = view_sh(
            &[("core", 2), ("member", 1), ("pub", 0)],
            &[("alice", KEY_A, "core"), ("bob", KEY_A, "member"), ("dan", KEY_A, "pub")],
            rootless_sharing(),
        );
        println!("{}", plan(&v, "gpu").unwrap().script);
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
