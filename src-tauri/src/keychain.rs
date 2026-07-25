// OS 钥匙串封装：存两样机密 —— 保险库设备主密钥、GitHub token。
//   · macOS → Keychain，Windows → 凭据管理器（keyring crate 的 native 后端）。
//   · 钥匙串不可用/被拒（未签名的 dev 构建有时会被拦）→ 回退到 app 目录下的 0600 文件，
//     保证功能不断；生产签名构建走钥匙串，拿到更强的 at-rest 保护。
use std::path::Path;

const SERVICE: &str = "dev.ait.devsys";

fn file(dir: &Path, account: &str) -> std::path::PathBuf {
    dir.join(format!(".kc-{account}"))
}

// 取一个机密：先钥匙串，再回退文件。都没有 → None。
pub fn get(dir: &Path, account: &str) -> Option<String> {
    if let Ok(entry) = keyring::Entry::new(SERVICE, account) {
        if let Ok(s) = entry.get_password() {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    std::fs::read_to_string(file(dir, account))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// 存一个机密：优先钥匙串（成功则清掉回退文件）；失败则写 0600 回退文件。
pub fn set(dir: &Path, account: &str, secret: &str) -> Result<(), String> {
    if let Ok(entry) = keyring::Entry::new(SERVICE, account) {
        if entry.set_password(secret).is_ok() {
            let _ = std::fs::remove_file(file(dir, account));
            return Ok(());
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let p = file(dir, account);
    std::fs::write(&p, secret).map_err(|e| e.to_string())?;
    restrict(&p);
    Ok(())
}

// 删除一个机密（钥匙串项 + 回退文件都清）。
pub fn del(dir: &Path, account: &str) {
    if let Ok(entry) = keyring::Entry::new(SERVICE, account) {
        let _ = entry.delete_credential();
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
