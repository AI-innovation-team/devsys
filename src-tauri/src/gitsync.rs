// team.yaml 的 git 同步。呼应「配置即代码」:团队配置放 git 仓库,每人维护自己那段,
// 天然有历史、有 review、有归属 —— 这就是那个"轻到几乎没有"的协调器。
//
// 直接调用系统 git(不引 libgit2):用户的 SSH key / credential helper / 代理设置全都复用,
// 私有仓库、企业 GitHub 一概照常工作。我们只做编排。
//
// 只碰 team.yaml 所在的那个仓库,只提交那一个文件 —— 不替用户 commit 别的东西。
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct GitStatus {
    pub is_repo: bool,
    pub branch: String,
    pub dirty: bool,          // team.yaml 有未提交改动
    pub has_remote: bool,
    pub remote: String,
}

fn repo_dir(file: &str) -> Result<PathBuf, String> {
    Path::new(file)
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "无法定位 team.yaml 所在目录".to_string())
}

// 跑一条 git 命令，返回 (成功, stdout+stderr)。
fn git(dir: &Path, args: &[&str]) -> Result<(bool, String), String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| format!("无法执行 git：{e}（系统里装了 git 吗？）"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&err);
    }
    Ok((out.status.success(), text.trim().to_string()))
}

// team.yaml 相对仓库根的路径（git add 要用）。
// 两边都 canonicalize：macOS 的 /tmp 是 /private/tmp 的符号链接，git 返回真实路径，
// 不规范化会 strip_prefix 失配。
fn rel_path(dir: &Path, file: &str) -> Result<String, String> {
    let (ok, root) = git(dir, &["rev-parse", "--show-toplevel"])?;
    if !ok {
        return Err("不是一个 git 仓库".into());
    }
    let root = std::fs::canonicalize(&root).unwrap_or_else(|_| PathBuf::from(&root));
    let f = std::fs::canonicalize(file).unwrap_or_else(|_| PathBuf::from(file));
    f.strip_prefix(&root)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|_| "team.yaml 不在该仓库内".to_string())
}

pub fn status(file: &str) -> Result<GitStatus, String> {
    let dir = repo_dir(file)?;
    let (is_repo, _) = git(&dir, &["rev-parse", "--is-inside-work-tree"])?;
    if !is_repo {
        return Ok(GitStatus {
            is_repo: false,
            branch: String::new(),
            dirty: false,
            has_remote: false,
            remote: String::new(),
        });
    }
    let (_, branch) = git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let (has_remote, remote) = git(&dir, &["remote", "get-url", "origin"])?;

    // rel_path 失败 = team.yaml 不在这个仓库里 —— 别把它悄悄当成空 pathspec
    // （空 pathspec 会匹配整个仓库，"脏"的判断就假了）。
    let rel = rel_path(&dir, file)?;
    let (_, st) = git(&dir, &["status", "--porcelain", "--", &rel])?;

    Ok(GitStatus {
        is_repo: true,
        branch,
        dirty: !st.is_empty(),
        has_remote,
        remote: if has_remote { remote } else { String::new() },
    })
}

// 拉取队友的更新（他们贡献的机器、新成员）。
pub fn pull(file: &str) -> Result<String, String> {
    let dir = repo_dir(file)?;
    let (ok, out) = git(&dir, &["pull", "--ff-only"])?;
    if ok {
        Ok(out)
    } else {
        Err(format!("git pull 失败：{out}"))
    }
}

// 把我对 team.yaml 的改动（贡献机器/邀成员）提交并推给团队。
// 只 add team.yaml 这一个文件 —— 不碰仓库里别的东西。
pub fn push(file: &str, message: &str) -> Result<String, String> {
    let dir = repo_dir(file)?;
    let rel = rel_path(&dir, file)?;

    let (ok, out) = git(&dir, &["add", "--", &rel])?;
    if !ok {
        return Err(format!("git add 失败：{out}"));
    }

    // 无改动则跳过 commit（幂等：重复点推送不产生空提交）。
    let (_, st) = git(&dir, &["status", "--porcelain", "--", &rel])?;
    let mut log = String::new();
    if !st.is_empty() {
        let (ok, out) = git(&dir, &["commit", "-m", message, "--", &rel])?;
        if !ok {
            return Err(format!("git commit 失败：{out}"));
        }
        log.push_str(&out);
    } else {
        log.push_str("team.yaml 无改动，跳过 commit");
    }

    let (ok, out) = git(&dir, &["push"])?;
    if !ok {
        return Err(format!("git push 失败：{out}"));
    }
    if !out.is_empty() {
        log.push('\n');
        log.push_str(&out);
    }
    Ok(log.trim().to_string())
}

// 克隆团队仓库到本地目录，返回其中 team.yaml 的路径。
pub fn clone(url: &str, dest: &str) -> Result<String, String> {
    let dest_p = Path::new(dest);
    let parent = dest_p
        .parent()
        .ok_or_else(|| "目标路径无效".to_string())?;
    let name = dest_p
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "目标路径无效".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;

    let (ok, out) = git(parent, &["clone", url, name])?;
    if !ok {
        return Err(format!("git clone 失败：{out}"));
    }

    // 找 team.yaml（根目录优先）。
    for cand in ["team.yaml", "team.yml"] {
        let p = dest_p.join(cand);
        if p.exists() {
            return Ok(p.to_string_lossy().to_string());
        }
    }
    Err("仓库里没找到 team.yaml —— 请确认这是团队配置仓库".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_on_non_repo_is_graceful() {
        let tmp = std::env::temp_dir().join("devsys-git-nonrepo");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("team.yaml");
        std::fs::write(&f, "team: x\n").unwrap();

        let st = status(f.to_str().unwrap()).unwrap();
        assert!(!st.is_repo, "非 git 目录应优雅返回，而非报错");
        assert!(!st.has_remote);
    }

    #[test]
    fn detects_repo_and_dirty_file() {
        let tmp = std::env::temp_dir().join("devsys-git-repo");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let d = tmp.as_path();

        assert!(git(d, &["init", "-q"]).unwrap().0);
        git(d, &["config", "user.email", "t@t.t"]).unwrap();
        git(d, &["config", "user.name", "t"]).unwrap();

        let f = tmp.join("team.yaml");
        std::fs::write(&f, "team: neuroai\n").unwrap();
        let fs = f.to_str().unwrap();

        let st = status(fs).unwrap();
        assert!(st.is_repo);
        assert!(st.dirty, "新文件未提交 → dirty");
        assert!(!st.has_remote);

        // 提交后应变干净
        git(d, &["add", "team.yaml"]).unwrap();
        git(d, &["commit", "-qm", "init"]).unwrap();
        let st = status(fs).unwrap();
        assert!(!st.dirty);
    }
}
