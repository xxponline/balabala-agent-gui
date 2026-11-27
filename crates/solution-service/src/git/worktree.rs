//! `git worktree` 封装（change solution-git-worktree, design D4）。
//!
//! 真源 = git 自己的元数据（`<project>/.git/worktrees/<id>`），不自建
//! manifest：列表走 `git worktree list --porcelain`（可发现应用外注册的
//! worktree），删除走 `git worktree remove`（**无 `--force`** —— 让 git
//! 内置 `check_clean_worktree` 当门, design D2）。

use super::{GitError, run, run_ok};
use std::path::{Path, PathBuf};

/// worktree 清单项（`list --porcelain` 解析产物）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    /// working copy 绝对路径。
    pub path: PathBuf,
    /// HEAD sha（porcelain 原样）。
    pub head: String,
    /// 当前分支（`refs/heads/` 前缀已剥）；detached → `None`。
    pub branch: Option<String>,
    /// prunable 原因（手删目录遗留时 git 给出）；非 prunable → `None`。
    pub prunable: Option<String>,
}

/// 目录名消毒（design D4）：产出可安全落盘的单段目录名。
///
/// - 拒：绝对路径、`.`/`..` 组件、消毒后为空 —— [`GitError::InvalidName`]
/// - 折：`/` 与 `\`（分支名斜杠）折成 `-`
/// - 换：Windows 非法字符 `:*?"<>|` 与控制字符换成 `-`，折叠连续 `-`，
///   去首尾 `-`/`.`/空格
pub fn sanitize_name(raw: &str) -> Result<String, GitError> {
    let invalid = |why: &str| GitError::InvalidName(format!("{raw:?}: {why}"));

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(invalid("empty"));
    }
    // 绝对路径: unix 根 或 Windows 盘符带分隔 (`C:\`, `C:/`) —— 纯 `a:b`
    // 是非法字符 `:` 的正常输入, 不算盘符
    if trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err(invalid("absolute path"));
    }
    let b = trimmed.as_bytes();
    if b.len() >= 3 && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\') && b[0].is_ascii_alphabetic() {
        return Err(invalid("absolute path (drive letter)"));
    }
    // 组件级拒绝 . / .. (含斜杠分隔后的形态: "a/../b" 折前先查)
    for comp in trimmed.split(['/', '\\']) {
        match comp {
            "." | ".." => return Err(invalid("`.`/`..` component")),
            _ => {}
        }
    }

    let mut out = String::with_capacity(trimmed.len());
    let mut last_dash = false;
    for ch in trimmed.chars() {
        let folded = match ch {
            '/' | '\\' => '-',
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        };
        if folded == '-' {
            if last_dash || out.is_empty() {
                continue; // 折叠连续 '-', 不以 '-' 开头
            }
            last_dash = true;
            out.push('-');
        } else {
            last_dash = false;
            out.push(folded);
        }
    }
    let out = out.trim_matches(['-', '.', ' '].as_slice());
    if out.is_empty() {
        return Err(invalid("nothing left after sanitizing"));
    }
    // 单段: 消毒后不得再含分隔符 (折叠加换已保证, 钉死防回归)
    if out.contains(['/', '\\']) {
        return Err(invalid("separator survived sanitizing"));
    }
    Ok(out.to_string())
}

/// `git worktree add <parent>/<消毒名>`：建在 `parent` 下，返回落盘路径。
///
/// 不带分支参数 —— git 按目录 basename 自建/检出同名分支（create 表单
/// 的 ref 参数属 design OQ2, 落地时再开）。
pub fn add(repo_root: &Path, parent: &Path, raw_name: &str) -> Result<PathBuf, GitError> {
    let name = sanitize_name(raw_name)?;
    let dest = parent.join(&name);
    run_ok(&["worktree", "add", &dest.to_string_lossy()], repo_root)?;
    // 回传 canonical 路径: git list 报的是解析符号链接后的真身
    // (macOS `/var` -> `/private/var`), 调用方拿它与 list 条目比对才相等
    Ok(std::fs::canonicalize(&dest).unwrap_or(dest))
}

/// `git worktree list --porcelain` 解析（含应用外注册的条目）。
pub fn list(repo_root: &Path) -> Result<Vec<WorktreeEntry>, GitError> {
    let out = run(&["worktree", "list", "--porcelain"], repo_root)?;
    if out.code != 0 {
        return Err(GitError::Failed {
            code: out.code,
            stderr: out.stderr,
        });
    }
    Ok(parse_porcelain(&out.stdout))
}

/// `git worktree remove <path>`（**无 `--force`**）：git 内置
/// `check_clean_worktree` 当门 —— 脏树/locked/校验失败 → [`GitError::Failed`]
/// 携带 git 原话，且先查后删原子（脏则一个字节不动）。
pub fn remove(repo_root: &Path, worktree_path: &Path) -> Result<(), GitError> {
    run_ok(
        &["worktree", "remove", &worktree_path.to_string_lossy()],
        repo_root,
    )?;
    Ok(())
}

/// porcelain 文本 → 条目（空行分块；未知字段容忍跳过, 保向前兼容）。
fn parse_porcelain(text: &str) -> Vec<WorktreeEntry> {
    let mut out: Vec<WorktreeEntry> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(' ') else {
            continue; // "detached" 等无值字段
        };
        match key {
            "worktree" => out.push(WorktreeEntry {
                path: PathBuf::from(value),
                head: String::new(),
                branch: None,
                prunable: None,
            }),
            "HEAD" => {
                if let Some(cur) = out.last_mut() {
                    cur.head = value.to_string();
                }
            }
            "branch" => {
                if let Some(cur) = out.last_mut() {
                    cur.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).into());
                }
            }
            "prunable" => {
                if let Some(cur) = out.last_mut() {
                    cur.prunable = Some(value.to_string());
                }
            }
            // locked / 其它字段: 容忍跳过 (删除侧由 git 自己拒, design D2)
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::run_ok as git_ok;

    /// 测试脚手架: 真 git 仓 (无网络, 本地提交)。
    fn repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        git_ok(&["init", "-q"], &dir).unwrap();
        git_ok(&["config", "user.email", "t@t"], &dir).unwrap();
        git_ok(&["config", "user.name", "t"], &dir).unwrap();
        std::fs::write(dir.join("f.txt"), "v1").unwrap();
        git_ok(&["add", "f.txt"], &dir).unwrap();
        git_ok(&["commit", "-q", "-m", "init"], &dir).unwrap();
        dir
    }

    /// 任务 1.3 验证: 建 / list 见 / 脏树 remove 被拒且目录原封 / 干净删净。
    #[test]
    fn add_list_remove_lifecycle() {
        let main = repo("life");
        let parent = main.join(".balabala.worktrees");
        std::fs::create_dir_all(&parent).unwrap();

        // 建
        let dest = add(&main, &parent, "feature-x").expect("add 应成功");
        assert!(dest.is_dir(), "worktree 目录应存在: {dest:?}");

        // list 见 (且含 main 自身) —— 路径与 git 报的一样是 canonical 真身
        let entries = list(&main).expect("list 应成功");
        let same = |e: &WorktreeEntry| {
            std::fs::canonicalize(&e.path).unwrap_or_else(|_| e.path.clone()) == dest
        };
        assert!(
            entries.iter().any(same),
            "list 应见新条目 (dest={dest:?}, got={entries:?})"
        );
        let mine = entries.iter().find(|e| same(e)).unwrap();
        assert_eq!(mine.branch.as_deref(), Some("feature-x"), "应自建同名分支");

        // 脏树 remove 被拒且目录原封 (git 先查后删原子, design D2)
        std::fs::write(dest.join("dirty.txt"), "x").unwrap();
        let err = remove(&main, &dest).expect_err("脏树必须被 git 门拒下");
        assert!(err.to_string().contains("modified or untracked"), "git 原话: {err}");
        assert!(dest.is_dir(), "被拒后目录必须原封不动");

        // 干净后 remove 删净, list 不再见
        std::fs::remove_file(dest.join("dirty.txt")).unwrap();
        remove(&main, &dest).expect("干净树应可删");
        assert!(!dest.exists(), "目录应删净");
        let after = list(&main).expect("list 应成功");
        assert!(!after.iter().any(same), "list 不应再见已删条目");
        // 元数据同步删净 (design D4 真源)
        assert!(
            !main.join(".git/worktrees/feature-x").exists(),
            "git 元数据应删净"
        );
    }

    /// 真机 D:\Balabala 冒烟回归: parent 带 `\\?\` 扩展前缀也能建成 ——
    /// 不剥前缀时 git (MSYS) 把它转成 `//?/` → mkdir "Invalid argument"
    /// (线上原文: could not create leading directories … Invalid argument;
    /// 探针 A 同形复现)。顺带覆盖 remove 的同款 dest 路径。
    #[cfg(windows)]
    #[test]
    fn add_succeeds_with_extended_prefix_parent() {
        let main = repo("extpfx");
        let parent = main.join(".balabala.worktrees");
        std::fs::create_dir_all(&parent).unwrap();
        let parent_ext = std::path::PathBuf::from(format!("\\\\?\\{}", parent.display()));

        let dest = add(&main, &parent_ext, "ext-x").expect("剥前缀后 add 应成功");
        assert!(dest.is_dir(), "worktree 目录应存在: {dest:?}");

        let entries = list(&main).expect("list 应成功");
        let same = |e: &WorktreeEntry| {
            std::fs::canonicalize(&e.path).unwrap_or_else(|_| e.path.clone()) == dest
        };
        assert!(entries.iter().any(same), "list 应见新条目");

        remove(&main, &dest).expect("干净删应成功 (dest 同样带前缀)");
        assert!(!dest.exists(), "目录应删净");
        std::fs::remove_dir_all(&parent).ok();
    }

    /// 消毒: 斜杠折叠 / Windows 非法字符 / 拒 .. 与绝对路径。
    #[test]
    fn sanitize_name_rules() {
        // 斜杠折叠 (分支名 feature/x)
        assert_eq!(sanitize_name("feature/x").unwrap(), "feature-x");
        assert_eq!(sanitize_name("a\\b").unwrap(), "a-b");
        // Windows 非法字符替换
        assert_eq!(sanitize_name("a:b*c?d\"e<f>g|h").unwrap(), "a-b-c-d-e-f-g-h");
        // 连续折叠 + 首尾修剪
        assert_eq!(sanitize_name("--x--").unwrap(), "x");
        assert_eq!(sanitize_name("  x  ").unwrap(), "x");
        // 拒 .. / 绝对路径 / 空
        assert!(matches!(sanitize_name(".."), Err(GitError::InvalidName(_))));
        assert!(matches!(sanitize_name("a/../b"), Err(GitError::InvalidName(_))));
        assert!(matches!(sanitize_name("."), Err(GitError::InvalidName(_))));
        assert!(matches!(sanitize_name("/abs"), Err(GitError::InvalidName(_))));
        assert!(matches!(sanitize_name("C:\\win"), Err(GitError::InvalidName(_))));
        assert!(matches!(sanitize_name("   "), Err(GitError::InvalidName(_))));
        // ".." 不会以折叠逃逸: 三点是合法字符, 但组件已拒; 全非法字符 → 空
        assert!(matches!(sanitize_name(":::"), Err(GitError::InvalidName(_))));
    }

    /// list 解析: prunable 标记 (手删目录遗留) 与 detached 容忍。
    #[test]
    fn parse_porcelain_fields() {
        let text = "\
worktree /main
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree /gone
HEAD 2222222222222222222222222222222222222222
branch refs/heads/z
prunable gitdir file points to non-existent location

worktree /detached-wt
HEAD 3333333333333333333333333333333333333333
detached
";
        let entries = parse_porcelain(text);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(entries[0].prunable, None);
        assert_eq!(entries[1].prunable.as_deref(), Some("gitdir file points to non-existent location"));
        assert_eq!(entries[1].branch.as_deref(), Some("z"));
        assert_eq!(entries[2].branch, None, "detached → None");
        assert_eq!(entries[2].head, "3333333333333333333333333333333333333333");
    }
}
