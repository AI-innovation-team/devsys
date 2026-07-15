// GitHub org → 团队花名册。消掉「每人手动登记自己 + 推送」——
// org 成员即团队成员，公钥从 github.com/<user>.keys 自动拉，角色由 GitHub team 映射。
// 新人进 org → 下次同步自动出现、公钥就位。呼应「配置即代码 / 协调器极轻」。
//
// 端点（都已实测）：
//   列成员   GET api.github.com/orgs/<org>/members            → [{login}]（私有 org 需 token）
//   列 team  GET api.github.com/orgs/<org>/teams/<slug>/members → [{login}]
//   取公钥   GET github.com/<user>.keys                        → 每行一把公钥（公开，免 token）
//
// 网络 IO 集中在 fetch_*，解析/合并是纯函数便于单测。
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// team.yaml 里的 org 绑定声明。
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct GithubBinding {
    pub org: String,
    // GitHub team slug → 我们的角色。"*" = 其余 org 成员的默认角色。
    #[serde(default)]
    pub role_map: BTreeMap<String, String>,
}

// 拉到的一个成员（org 成员 + 其公钥 + 映射后的角色）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct GhMember {
    pub login: String,
    pub pubkeys: Vec<String>,
    pub role: String,
}

fn ua() -> &'static str {
    "devsys-app"
}

// 组一个带可选 token 的请求（私有 org 列成员需要 token；拉公钥不需要）。
fn get(url: &str, token: Option<&str>) -> Result<ureq::Response, String> {
    let mut req = ureq::get(url).set("User-Agent", ua()).set("Accept", "application/vnd.github+json");
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    req.call().map_err(|e| match e {
        ureq::Error::Status(code, _) => match code {
            404 => "org 不存在或不可见（私有 org 需要 token）".into(),
            401 | 403 => "GitHub 认证失败或速率受限（检查 token）".into(),
            _ => format!("GitHub 返回 {code}"),
        },
        other => format!("请求 GitHub 失败：{other}"),
    })
}

#[derive(Deserialize)]
struct Login {
    login: String,
}

// 列 org 全部成员的 login（分页）。
fn fetch_org_logins(org: &str, token: Option<&str>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for page in 1..=10 {
        let url = format!("https://api.github.com/orgs/{org}/members?per_page=100&page={page}");
        let list: Vec<Login> = get(&url, token)?.into_json().map_err(|e| e.to_string())?;
        if list.is_empty() {
            break;
        }
        out.extend(list.into_iter().map(|l| l.login));
    }
    Ok(out)
}

// 列某 GitHub team 的成员 login（用于角色映射）。
fn fetch_team_logins(org: &str, team: &str, token: Option<&str>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for page in 1..=10 {
        let url = format!("https://api.github.com/orgs/{org}/teams/{team}/members?per_page=100&page={page}");
        let list: Vec<Login> = match get(&url, token) {
            Ok(r) => r.into_json().map_err(|e| e.to_string())?,
            Err(_) => break, // team 不存在/无权 → 跳过该映射，不致命
        };
        if list.is_empty() {
            break;
        }
        out.extend(list.into_iter().map(|l| l.login));
    }
    Ok(out)
}

// 拉某用户的公钥（公开，免 token）。解析成每行一把。
fn fetch_user_keys(login: &str) -> Result<Vec<String>, String> {
    let url = format!("https://github.com/{login}.keys");
    let text = ureq::get(&url)
        .set("User-Agent", ua())
        .call()
        .map_err(|e| format!("拉 {login} 公钥失败：{e}"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    Ok(parse_keys(&text))
}

// 纯解析：.keys 文本 → 公钥列表（去空行）。
pub fn parse_keys(text: &str) -> Vec<String> {
    text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

// 纯函数：给定 org 成员 + 各 team 成员集合，算每人的角色。
// role_map: team-slug → role；"*" 是默认。找不到映射且无默认 → "member"。
pub fn assign_roles(
    org_logins: &[String],
    team_members: &BTreeMap<String, Vec<String>>, // team-slug → logins
    role_map: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let default = role_map.get("*").cloned().unwrap_or_else(|| "member".into());
    let mut roles = BTreeMap::new();
    for login in org_logins {
        // 命中的第一个 team 映射即角色（按 role_map 声明的 team 顺序找最高优先）。
        let mut role = default.clone();
        for (team, r) in role_map {
            if team == "*" {
                continue;
            }
            if team_members.get(team).is_some_and(|ms| ms.iter().any(|m| m == login)) {
                role = r.clone();
                break;
            }
        }
        roles.insert(login.clone(), role);
    }
    roles
}

// 完整同步：拉 org 花名册 + 角色 team + 每人公钥 → GhMember 列表。网络操作。
pub fn sync(binding: &GithubBinding, token: Option<&str>) -> Result<Vec<GhMember>, String> {
    if binding.org.trim().is_empty() {
        return Err("未绑定 GitHub org".into());
    }
    let logins = fetch_org_logins(&binding.org, token)?;

    // 拉 role_map 里引用到的每个 team 的成员。
    let mut team_members: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for team in binding.role_map.keys() {
        if team == "*" {
            continue;
        }
        team_members.insert(team.clone(), fetch_team_logins(&binding.org, team, token)?);
    }

    let roles = assign_roles(&logins, &team_members, &binding.role_map);

    let mut members = Vec::new();
    for login in logins {
        let pubkeys = fetch_user_keys(&login).unwrap_or_default(); // 没配公钥的人：空，UI 提示
        let role = roles.get(&login).cloned().unwrap_or_else(|| "member".into());
        members.push(GhMember { login, pubkeys, role });
    }
    members.sort_by(|a, b| a.login.cmp(&b.login));
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keys_drops_blanks() {
        let t = "ssh-ed25519 AAA\n\n  ssh-rsa BBB  \n";
        assert_eq!(parse_keys(t), vec!["ssh-ed25519 AAA".to_string(), "ssh-rsa BBB".to_string()]);
    }

    #[test]
    fn roles_from_team_membership() {
        let org = vec!["alice".to_string(), "bob".to_string(), "carol".to_string()];
        let mut teams = BTreeMap::new();
        teams.insert("core-team".to_string(), vec!["alice".to_string()]);
        teams.insert("guests".to_string(), vec!["carol".to_string()]);
        let mut map = BTreeMap::new();
        map.insert("core-team".to_string(), "core".to_string());
        map.insert("guests".to_string(), "guest".to_string());
        map.insert("*".to_string(), "member".to_string());

        let roles = assign_roles(&org, &teams, &map);
        assert_eq!(roles.get("alice").unwrap(), "core"); // 在 core-team
        assert_eq!(roles.get("bob").unwrap(), "member"); // 无 team → 默认
        assert_eq!(roles.get("carol").unwrap(), "guest"); // 在 guests
    }

    #[test]
    fn default_role_is_member_without_star() {
        let org = vec!["dave".to_string()];
        let roles = assign_roles(&org, &BTreeMap::new(), &BTreeMap::new());
        assert_eq!(roles.get("dave").unwrap(), "member");
    }
}
