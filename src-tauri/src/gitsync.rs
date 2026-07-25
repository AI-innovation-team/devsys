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

    // 提交范围 = team.yaml + members/(**贡献机器写在 members/<我>.yaml,漏掉它
    // 队友和控制面就永远看不到你共享的机器**)。只加这两个,不碰仓库里别的东西。
    let mut paths: Vec<String> = vec![rel];
    if dir.join("members").is_dir() {
        paths.push("members".to_string());
    }
    // 花名册是本地缓存,不该进 git。
    let gi = dir.join(".gitignore");
    let gi_text = std::fs::read_to_string(&gi).unwrap_or_default();
    if !gi_text.contains(".github-roster.json") {
        let mut t = gi_text;
        if !t.is_empty() && !t.ends_with('\n') {
            t.push('\n');
        }
        t.push_str(".github-roster.json\n");
        let _ = std::fs::write(&gi, t);
        paths.push(".gitignore".to_string());
    }

    let mut args: Vec<&str> = vec!["add", "--"];
    args.extend(paths.iter().map(|s| s.as_str()));
    let (ok, out) = git(&dir, &args)?;
    if !ok {
        return Err(format!("git add 失败：{out}"));
    }

    // 无改动则跳过 commit（幂等：重复点推送不产生空提交）。
    let mut sargs: Vec<&str> = vec!["status", "--porcelain", "--"];
    sargs.extend(paths.iter().map(|s| s.as_str()));
    let (_, st) = git(&dir, &sargs)?;
    let mut log = String::new();
    if !st.is_empty() {
        let mut cargs: Vec<&str> = vec!["commit", "-m", message, "--"];
        cargs.extend(paths.iter().map(|s| s.as_str()));
        let (ok, out) = git(&dir, &cargs)?;
        if !ok {
            return Err(format!("git commit 失败：{out}"));
        }
        log.push_str(&out);
    } else {
        log.push_str("团队配置无改动，跳过 commit");
    }

    // 没设过 upstream 的分支直接 push 会失败(且报错信息随 git 语言变,不能靠匹配文本)——
    // 先探一下,没有就带 -u 建立跟踪。
    let (has_upstream, _) = git(&dir, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])?;
    let (ok, out) = if has_upstream {
        git(&dir, &["push"])?
    } else {
        git(&dir, &["push", "-u", "origin", "HEAD"])?
    };
    if !ok {
        return Err(format!("git push 失败：{out}"));
    }
    if !out.is_empty() {
        log.push('\n');
        log.push_str(&out);
    }
    Ok(log.trim().to_string())
}

// 连上 GitHub 远程并推送（模板生成后一键：加/改 origin → 主分支 main → push）。
// 走系统 git = 用户自己的 SSH/凭据,不需要 gh、不需要 token。要求 GitHub 上已建空仓库。
pub fn set_remote_push(file: &str, url: &str) -> Result<String, String> {
    let dir = repo_dir(file)?;
    let (has, _) = git(&dir, &["remote", "get-url", "origin"])?;
    let (ok, out) = if has {
        git(&dir, &["remote", "set-url", "origin", url])?
    } else {
        git(&dir, &["remote", "add", "origin", url])?
    };
    if !ok {
        return Err(format!("设置 remote 失败：{out}"));
    }
    let _ = git(&dir, &["branch", "-M", "main"]);
    let (ok, out) = git(&dir, &["push", "-u", "origin", "main"])?;
    if !ok {
        return Err(format!("git push 失败：{out}\n(确认 GitHub 上已建好空仓库 {url},且本机 git 有推送权限)"));
    }
    Ok(format!("已推送到 {url}\n{out}").trim().to_string())
}

// 初始化一个新仓库并做初始提交（模板生成用）。commit 失败（git user 未配）不致命 ——
// 文件与 .git 已就位，用户可自行 commit。
pub fn init(dir: &str) -> Result<(), String> {
    let d = Path::new(dir);
    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    let (ok, out) = git(d, &["init", "-q"])?;
    if !ok {
        return Err(format!("git init 失败：{out}"));
    }
    let _ = git(d, &["add", "-A"]);
    let _ = git(d, &["commit", "-qm", "init: AIT.dev 团队配置模板"]);
    Ok(())
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

    // 回归:push 必须把 members/ 一起提交 —— 只提交 team.yaml 的话,
    // 用户在 app 里共享的机器永远同步不到队友和控制面。
    #[test]
    fn push_includes_member_files() {
        let tmp = std::env::temp_dir().join("devsys-git-members");
        let _ = std::fs::remove_dir_all(&tmp);
        let work = tmp.join("work");
        let bare = tmp.join("remote.git");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(&bare).unwrap();
        assert!(git(&bare, &["init", "--bare", "-q"]).unwrap().0);

        let d = work.as_path();
        assert!(git(d, &["init", "-q", "-b", "main"]).unwrap().0);
        git(d, &["config", "user.email", "t@t.t"]).unwrap();
        git(d, &["config", "user.name", "t"]).unwrap();
        git(d, &["remote", "add", "origin", bare.to_str().unwrap()]).unwrap();

        let f = work.join("team.yaml");
        std::fs::write(&f, "team: x\n").unwrap();
        std::fs::create_dir_all(work.join("members")).unwrap();
        std::fs::write(work.join("members/alice.yaml"), "member:\n  name: alice\n").unwrap();
        // 本地缓存不该进 git
        std::fs::write(work.join(".github-roster.json"), "[]").unwrap();

        push(f.to_str().unwrap(), "test").expect("push 应成功");

        let (_, tracked) = git(d, &["ls-files"]).unwrap();
        assert!(tracked.contains("members/alice.yaml"), "members 必须被提交: {tracked}");
        assert!(tracked.contains("team.yaml"));
        assert!(!tracked.contains(".github-roster.json"), "花名册缓存不该进 git: {tracked}");
    }

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
