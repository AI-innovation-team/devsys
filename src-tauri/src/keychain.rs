// OS 钥匙串封装：存两样机密 —— 保险库设备主密钥、GitHub token。
//   · macOS → Keychain，Windows → 凭据管理器（keyring crate 的 native 后端）。
//   · 钥匙串不可用/被拒（未签名的 dev 构建有时会被拦）→ 回退到 app 目录下的 0600 文件，
//     保证功能不断；生产签名构建走钥匙串，拿到更强的 at-rest 保护。
use std::path::Path;

const SERVICE: &str = "dev.ait.devsys";

fn file(dir: &Path, account: &str) -> std::path::PathBuf {
    dir.join(format!(".kc-{account}"))
}

// ★ 开发构建绕开钥匙串（否则每次启动都被要系统密码）。
//
// 为什么必须绕：macOS 的钥匙串 ACL 绑的是应用的**代码签名标识**。`tauri dev` 出来的是
// ad-hoc 签名（`codesign -dvvv` 看到 `Signature=adhoc, linker-signed`），它的标识就是
// 二进制自己的 cdhash —— **每次重编 cdhash 都变** → 钥匙串认为「另一个 app 想读这个项」
// → 弹密码。点多少次「始终允许」都没用，因为下一次编译又是"另一个 app"。
// 发行版二进制不变，所以点一次就再不弹（换版本时会再问一次；有 Developer ID 签名则一次都不问）。
//
// 想在 dev 里验真钥匙串路径：`DEVSYS_KEYCHAIN=1 npm run tauri dev`。
fn skip_keychain() -> bool {
    cfg!(debug_assertions) && std::env::var("DEVSYS_KEYCHAIN").is_err()
}

fn read_file(dir: &Path, account: &str) -> Option<String> {
    std::fs::read_to_string(file(dir, account))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn write_file(dir: &Path, account: &str, secret: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let p = file(dir, account);
    std::fs::write(&p, secret).map_err(|e| e.to_string())?;
    restrict(&p);
    Ok(())
}

// 取一个机密：先钥匙串，再回退文件。都没有 → None。
pub fn get(dir: &Path, account: &str) -> Option<String> {
    if skip_keychain() {
        // 文件优先。文件里没有就去钥匙串**取一次**（这一次会弹密码），
        // 顺手落进文件 —— 于是老用户不必迁移保险库，之后也不再被打扰。
        if let Some(s) = read_file(dir, account) {
            return Some(s);
        }
        let from_kc = keyring::Entry::new(SERVICE, account)
            .ok()
            .and_then(|e| e.get_password().ok())
            .filter(|s| !s.is_empty())?;
        let _ = write_file(dir, account, &from_kc);
        return Some(from_kc);
    }
    if let Ok(entry) = keyring::Entry::new(SERVICE, account) {
        if let Ok(s) = entry.get_password() {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    read_file(dir, account)
}

// 存一个机密：优先钥匙串（成功则清掉回退文件）；失败则写 0600 回退文件。
pub fn set(dir: &Path, account: &str, secret: &str) -> Result<(), String> {
    if skip_keychain() {
        return write_file(dir, account, secret);
    }
    if let Ok(entry) = keyring::Entry::new(SERVICE, account) {
        if entry.set_password(secret).is_ok() {
            let _ = std::fs::remove_file(file(dir, account));
            return Ok(());
        }
    }
    write_file(dir, account, secret)
}

// 删除一个机密（钥匙串项 + 回退文件都清）。
pub fn del(dir: &Path, account: &str) {
    if !skip_keychain() {
        if let Ok(entry) = keyring::Entry::new(SERVICE, account) {
            let _ = entry.delete_credential();
        }
    }
    let _ = std::fs::remove_file(file(dir, account));
}

#[cfg(unix)]
fn restrict(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict(_p: &Path) {}
