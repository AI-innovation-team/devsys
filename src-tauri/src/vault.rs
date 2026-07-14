// 凭据加密保险库（Stronghold）。全平台一致（桌面 + iOS/Android），不依赖 OS 钥匙串。
//   - 主密码 → Argon2(带随机 salt) → 32 字节密钥 → 加密快照文件 vault.stronghold
//   - 解锁后 Stronghold 实例缓存在内存（本会话），密钥/明文只在 Rust 侧
//   - 未解锁时 set/get/delete 返回错误码 "VAULT_LOCKED"，前端据此弹解锁框
use std::path::PathBuf;
use std::sync::Mutex;

use iota_stronghold::{Client, KeyProvider, SnapshotPath, Stronghold};
use zeroize::Zeroizing;

const CLIENT_PATH: &[u8] = b"devsys";
const SALT_FILE: &str = "vault.salt";
const SNAP_FILE: &str = "vault.stronghold";

fn e2s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// Stronghold 2.x 的 KeyProvider 从 Zeroizing<Vec<u8>>（32 字节密钥）构造。
fn key_provider(key: &[u8]) -> Result<KeyProvider, String> {
    KeyProvider::try_from(Zeroizing::new(key.to_vec())).map_err(e2s)
}

pub struct Vault {
    dir: PathBuf,
    inner: Mutex<Option<Unlocked>>,
}

struct Unlocked {
    stronghold: Stronghold,
    client: Client,
    key: Vec<u8>,
}

impl Vault {
    pub fn new(dir: PathBuf) -> Self {
        Vault {
            dir,
            inner: Mutex::new(None),
        }
    }

    fn snap(&self) -> SnapshotPath {
        SnapshotPath::from_path(self.dir.join(SNAP_FILE))
    }

    pub fn exists(&self) -> bool {
        self.dir.join(SNAP_FILE).exists()
    }

    pub fn is_unlocked(&self) -> bool {
        self.inner.lock().unwrap().is_some()
    }

    // 退出登录：清掉内存中的解锁态（快照文件不动，下次仍需密码）。
    pub fn lock(&self) {
        *self.inner.lock().unwrap() = None;
    }

    // 读或建随机 salt（非机密，与快照同目录）。
    fn salt(&self) -> Result<Vec<u8>, String> {
        let p = self.dir.join(SALT_FILE);
        if let Ok(s) = std::fs::read(&p) {
            if s.len() >= 16 {
                return Ok(s);
            }
        }
        let mut salt = [0u8; 16];
        getrandom::getrandom(&mut salt).map_err(e2s)?;
        std::fs::create_dir_all(&self.dir).map_err(e2s)?;
        std::fs::write(&p, salt).map_err(e2s)?;
        Ok(salt.to_vec())
    }

    fn derive(&self, password: &str) -> Result<Vec<u8>, String> {
        let salt = self.salt()?;
        let mut key = vec![0u8; 32];
        argon2::Argon2::default()
            .hash_password_into(password.as_bytes(), &salt, &mut key)
            .map_err(e2s)?;
        Ok(key)
    }

    // 解锁：已有快照则用主密码加载（密码错→解密失败）；无则以该密码新建。
    pub fn unlock(&self, password: &str) -> Result<(), String> {
        let key = self.derive(password)?;
        let stronghold = Stronghold::default();
        let client = if self.exists() {
            let kp = key_provider(&key)?;
            stronghold
                .load_client_from_snapshot(CLIENT_PATH, &kp, &self.snap())
                .map_err(|_| "主密码错误或保险库损坏".to_string())?
        } else {
            let c = stronghold.create_client(CLIENT_PATH).map_err(e2s)?;
            stronghold.write_client(CLIENT_PATH).map_err(e2s)?;
            std::fs::create_dir_all(&self.dir).map_err(e2s)?;
            stronghold
                .commit_with_keyprovider(&self.snap(), &key_provider(&key)?)
                .map_err(e2s)?;
            c
        };
        *self.inner.lock().unwrap() = Some(Unlocked {
            stronghold,
            client,
            key,
        });
        Ok(())
    }

    fn persist(&self, u: &Unlocked) -> Result<(), String> {
        u.stronghold.write_client(CLIENT_PATH).map_err(e2s)?;
        u.stronghold
            .commit_with_keyprovider(&self.snap(), &key_provider(&u.key)?)
            .map_err(e2s)
    }

    pub fn set(&self, name: &str, secret: &str) -> Result<(), String> {
        let guard = self.inner.lock().unwrap();
        let u = guard.as_ref().ok_or("VAULT_LOCKED")?;
        u.client
            .store()
            .insert(name.as_bytes().to_vec(), secret.as_bytes().to_vec(), None)
            .map_err(e2s)?;
        self.persist(u)
    }

    pub fn get(&self, name: &str) -> Result<String, String> {
        let guard = self.inner.lock().unwrap();
        let u = guard.as_ref().ok_or("VAULT_LOCKED")?;
        let v = u
            .client
            .store()
            .get(name.as_bytes())
            .map_err(e2s)?
            .ok_or_else(|| "该服务器未配置凭据".to_string())?;
        String::from_utf8(v).map_err(e2s)
    }

    // 删除；未解锁时静默跳过（孤儿密钥无害，随快照密码保护）。
    pub fn delete_lenient(&self, name: &str) {
        if let Some(u) = self.inner.lock().unwrap().as_ref() {
            let _ = u.client.store().delete(name.as_bytes());
            let _ = self.persist(u);
        }
    }

    // 重置：忘记主密码的唯一出路 —— 销毁快照与 salt，凭据全丢（不可逆）。
    // 拓扑（servers.json）不在这里，调用方负责把 has_secret 标记一并清掉。
    pub fn reset(&self) -> Result<(), String> {
        *self.inner.lock().unwrap() = None;
        for f in [SNAP_FILE, SALT_FILE] {
            let p = self.dir.join(f);
            if p.exists() {
                std::fs::remove_file(&p).map_err(e2s)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Vault;

    #[test]
    fn create_reopen_use() {
        let dir = std::env::temp_dir().join("devsys-vault-selftest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 首次：建库 + 存取
        let v = Vault::new(dir.clone());
        assert!(!v.exists());
        v.unlock("testpass123").expect("create/unlock 失败");
        assert!(v.exists() && v.is_unlocked());
        v.set("srv", "hunter2").expect("set 失败");
        assert_eq!(v.get("srv").unwrap(), "hunter2");

        // 重开：用同密码加载
        let v2 = Vault::new(dir.clone());
        v2.unlock("testpass123").expect("reload 失败");
        assert_eq!(v2.get("srv").unwrap(), "hunter2");

        // 错误密码应失败
        let v3 = Vault::new(dir);
        assert!(v3.unlock("wrongpass").is_err());
    }

    #[test]
    fn reset_destroys_and_allows_new_password() {
        let dir = std::env::temp_dir().join("devsys-vault-reset-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let v = Vault::new(dir.clone());
        v.unlock("oldpass123").unwrap();
        v.set("srv", "secret").unwrap();
        assert!(v.exists());

        // 重置：快照销毁、回到「未建库」，内存解锁态也清掉
        v.reset().unwrap();
        assert!(!v.exists());
        assert!(!v.is_unlocked());

        // 可用全新密码重建；旧凭据不复存在
        let v2 = Vault::new(dir);
        v2.unlock("brandnew456").unwrap();
        assert!(v2.get("srv").is_err()); // 旧凭据已随快照销毁
    }
}
