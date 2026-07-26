// GitHub 登录（OAuth Device Flow）—— 桌面标准:app 显示一个短码,用户在浏览器
// github.com/login/device 授权,我们轮询换 token。无需 client secret。
//
// 用途:登录 = GitHub 身份(取代本地密码);登录后列出用户所属 org 供「自动选择组织」,
// 选中的 org 即团队(喂 github.rs 的花名册)。token 存 OS 钥匙串,会话(login/org)存
// app 目录小 json。保险库另有设备密钥自动解锁,与这里解耦。
//
// client_id 是 OAuth App 的公开值(非机密):从环境变量 DEVSYS_GH_CLIENT_ID 或
// app 目录 gh-client-id 文件读。没配 → 明确报错让用户先注册一个 OAuth App。
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::keychain;

const TOKEN_ACCOUNT: &str = "gh-token";
const SESSION_FILE: &str = "gh-session.json";
const CLIENT_FILE: &str = "gh-client-id";
// read:org = 列组织/成员/公钥;repo = 建/读写团队仓库(ait-team 及后续 org repo 操作)。
const SCOPE: &str = "read:org repo";

// 应用级 client_id（不是每人一个！团队注册一次 OAuth App，烤进分发二进制，用户零配置）。
// client_id 非机密 —— device flow 专为「存不住 secret 的公共客户端」设计，公开分发是预期用法
// （GitHub 官方 gh CLI 即如此）。分发构建时 `DEVSYS_GH_CLIENT_ID=xxx tauri build` 注入即可，
// 或直接把下面常量填成你的值。运行时 env / 文件仍可覆盖（开发期方便切换）。
const FALLBACK_CLIENT_ID: &str = "Ov23lidtpuzaZa6jGU9q"; // AIT.dev 团队 OAuth App（公开值，非机密）

// ★ **构建期把这个变量设成空串，等同于没设**。
// 踩过的坑：release workflow 里写了 `DEVSYS_GH_CLIENT_ID: ${{ vars.XXX }}`，
// 而仓库变量没配 → 展开成空串 → 变量确实被设了 → `option_env!` 返回 `Some("")`
// → 烤进二进制的 client_id 是空的 → **分发版一打开就说「还没配 OAuth App」，用户根本没法登录**。
// dev 构建不设这个变量，所以本地永远发现不了 —— 只有发行版会中招。
fn baked_client_id() -> &'static str {
    match option_env!("DEVSYS_GH_CLIENT_ID") {
        Some(v) if !v.trim().is_empty() => v.trim(),
        _ => FALLBACK_CLIENT_ID,
    }
}

fn ua() -> &'static str {
    "devsys-app"
}

// 已登录的 GitHub 会话（非机密:login/org 都是公开信息，token 在钥匙串）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Session {
    pub login: String,
    #[serde(default)]
    pub org: String,
}

// device_start 的返回：给前端展示的短码 + 授权 URL + 轮询节奏。
// verification_uri_complete = 码已预填进 URL，浏览器打开后用户只需点 Authorize。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: String,
    pub interval: u64,
    pub expires_in: u64,
}

// device_poll 的返回：pending=继续等；ok=登录成功（带 login + 可选 org）；error=终止。
#[derive(Serialize, Clone, Debug)]
pub struct PollResult {
    pub status: String, // pending | slow_down | ok | error
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(default)]
    pub orgs: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// OAuth App 的 client_id：运行时 env → app 目录文件 → 编译期烤进的默认值。
// 分发版靠 BAKED_CLIENT_ID（用户零配置）；env/文件仅供开发期覆盖。空=未配置。
pub fn client_id(dir: &Path) -> String {
    if let Ok(v) = std::env::var("DEVSYS_GH_CLIENT_ID") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return v;
        }
    }
    if let Ok(s) = std::fs::read_to_string(dir.join(CLIENT_FILE)) {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    baked_client_id().to_string()
}

// 会话读/写。
pub fn session(dir: &Path) -> Session {
    std::fs::read_to_string(dir.join(SESSION_FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_session(dir: &Path, s: &Session) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(
        dir.join(SESSION_FILE),
        serde_json::to_string(s).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

pub fn token(dir: &Path) -> Option<String> {
    keychain::get(dir, TOKEN_ACCOUNT)
}

// 是否已登录：有会话 login + 钥匙串里有 token。
pub fn logged_in(dir: &Path) -> bool {
    !session(dir).login.is_empty() && token(dir).is_some()
}

#[derive(Deserialize)]
struct DeviceCodeResp {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: String,
    #[serde(default = "default_interval")]
    interval: u64,
    #[serde(default)]
    expires_in: u64,
}
fn default_interval() -> u64 {
    5
}

// 第一步：拿设备码 + 用户码。
pub fn device_start(dir: &Path) -> Result<DeviceStart, String> {
    let cid = client_id(dir);
    if cid.is_empty() {
        return Err(
            "未配置 GitHub OAuth App —— 去 GitHub Settings → Developer settings → \
             OAuth Apps 建一个（勾选 Enable Device Flow），把 Client ID 写入 \
             DEVSYS_GH_CLIENT_ID 环境变量或 app 目录的 gh-client-id 文件。"
                .into(),
        );
    }
    let resp = ureq::post("https://github.com/login/device/code")
        .set("Accept", "application/json")
        .set("User-Agent", ua())
        .send_form(&[("client_id", &cid), ("scope", SCOPE)])
        .map_err(|e| format!("请求设备码失败：{e}"))?;
    let r: DeviceCodeResp = resp.into_json().map_err(|e| e.to_string())?;
    Ok(DeviceStart {
        device_code: r.device_code,
        user_code: r.user_code,
        verification_uri: r.verification_uri,
        verification_uri_complete: r.verification_uri_complete,
        interval: r.interval,
        expires_in: r.expires_in,
    })
}

#[derive(Deserialize)]
struct TokenResp {
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    error: String,
}

#[derive(Deserialize)]
struct GhLogin {
    login: String,
}

// 第二步：用设备码轮询换 token。前端按 interval 反复调，直到 ok/error。
pub fn device_poll(dir: &Path, device_code: &str) -> Result<PollResult, String> {
    let cid = client_id(dir);
    let resp = ureq::post("https://github.com/login/oauth/access_token")
        .set("Accept", "application/json")
        .set("User-Agent", ua())
        .send_form(&[
            ("client_id", &cid),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ]);
    // 授权未完成时 GitHub 返回 200 + {"error":"authorization_pending"}；失败可能是 4xx。
    let body: TokenResp = match resp {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(_, r)) => r.into_json().map_err(|e| e.to_string())?,
        Err(e) => return Err(format!("轮询 token 失败：{e}")),
    };
    if !body.access_token.is_empty() {
        // 拿到 token：存钥匙串，取 login 与所属 org，落会话。
        keychain::set(dir, TOKEN_ACCOUNT, &body.access_token)?;
        let login = fetch_login(&body.access_token)?;
        let orgs = fetch_orgs(&body.access_token).unwrap_or_default();
        let mut s = session(dir);
        s.login = login.clone();
        // org 唯一时直接选中（自动选择组织）；多个留给用户挑。
        if s.org.is_empty() && orgs.len() == 1 {
            s.org = orgs[0].clone();
        }
        write_session(dir, &s)?;
        return Ok(PollResult { status: "ok".into(), login: Some(login), orgs, error: None });
    }
    let status = match body.error.as_str() {
        "authorization_pending" => "pending",
        "slow_down" => "slow_down",
        _ => "error",
    };
    Ok(PollResult {
        status: status.into(),
        login: None,
        orgs: Vec::new(),
        error: if status == "error" { Some(body.error) } else { None },
    })
}

fn fetch_login(token: &str) -> Result<String, String> {
    let r: GhLogin = ureq::get("https://api.github.com/user")
        .set("Authorization", &format!("Bearer {token}"))
        .set("User-Agent", ua())
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("取 GitHub 身份失败：{e}"))?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(r.login)
}

// 列出登录用户所属的 org（供「自动选择组织」）。
pub fn fetch_orgs(token: &str) -> Result<Vec<String>, String> {
    let list: Vec<GhLogin> = ureq::get("https://api.github.com/user/orgs?per_page=100")
        .set("Authorization", &format!("Bearer {token}"))
        .set("User-Agent", ua())
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("列出组织失败：{e}"))?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(list.into_iter().map(|o| o.login).collect())
}

// 用已存 token 重新列 org（切换组织时用）。
pub fn orgs(dir: &Path) -> Result<Vec<String>, String> {
    let t = token(dir).ok_or("未登录 GitHub")?;
    fetch_orgs(&t)
}

// 在 org 下建私有仓库 ait-team（需 token 有 repo 权限）。已存在(422)当成功。
// 权限不足返回 "SCOPE"，供上层提示重新授权。
pub fn create_repo(org: &str, token: &str) -> Result<(), String> {
    let url = format!("https://api.github.com/orgs/{org}/repos");
    let body = serde_json::json!({ "name": "ait-team", "private": true, "auto_init": false });
    match ureq::post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("User-Agent", ua())
        .set("Accept", "application/vnd.github+json")
        .send_json(body)
    {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(422, _)) => Ok(()), // 已存在
        Err(ureq::Error::Status(403, _)) | Err(ureq::Error::Status(404, _)) => Err("SCOPE".into()),
        Err(ureq::Error::Status(code, r)) => {
            Err(format!("建仓库失败 {code}：{}", r.into_string().unwrap_or_default()))
        }
        Err(e) => Err(format!("建仓库请求失败：{e}")),
    }
}

// token 已授予的 scope（读 X-OAuth-Scopes 响应头）。用于判断是否需重新授权拿 repo 权限。
pub fn token_scopes(token: &str) -> Vec<String> {
    match ureq::get("https://api.github.com/user")
        .set("Authorization", &format!("Bearer {token}"))
        .set("User-Agent", ua())
        .call()
    {
        Ok(r) => r
            .header("X-OAuth-Scopes")
            .unwrap_or("")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub fn set_org(dir: &Path, org: &str) -> Result<(), String> {
    let mut s = session(dir);
    if s.login.is_empty() {
        return Err("未登录 GitHub".into());
    }
    s.org = org.to_string();
    write_session(dir, &s)
}

// 退出 GitHub 登录：清 token + 会话（保险库不动，它由设备密钥保护）。
pub fn logout(dir: &Path) {
    keychain::del(dir, TOKEN_ACCOUNT);
    let _ = std::fs::remove_file(dir.join(SESSION_FILE));
}

#[cfg(test)]
mod tests {
    use super::*;

    // ★ 空的 client_id = 分发版一打开就「未配置」、用户根本没法登录，
    // 而 dev 构建永远复现不了（它不设这个编译期变量）。所以必须有这条守着。
    #[test]
    fn baked_client_id_is_never_empty() {
        assert!(
            !baked_client_id().is_empty(),
            "烤进去的 client_id 是空的 —— 构建时 DEVSYS_GH_CLIENT_ID 被设成了空串？\
             空串必须当作「没设」，回落到 FALLBACK_CLIENT_ID。"
        );
    }

    // 运行时覆盖链:env → app 目录文件 → 烤进去的默认值。任一环节都不该产出空值。
    #[test]
    fn client_id_falls_back_when_nothing_configured() {
        let d = std::env::temp_dir().join("devsys-ghauth-test");
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // 不碰进程 env（并发测试会互相干扰），只验「什么都没配」时的兜底
        if std::env::var("DEVSYS_GH_CLIENT_ID").is_err() {
            assert_eq!(client_id(&d), FALLBACK_CLIENT_ID);
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
