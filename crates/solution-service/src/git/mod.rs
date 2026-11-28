//! 系统 git 统一出口（change solution-git-worktree, design D6）。
//!
//! 全 crate 的 git 子进程**只准经本模块**（不散落裸 `Command`）：统一
//! 防挂三件套 —— `stdin = null`（绝不读输入）、`GIT_TERMINAL_PROMPT=0`
//! + `GIT_ASKPASS=:`（凭证请求即失败而非挂死后台任务）、stderr 截断
//! **保留尾部**（git 的错误信息在尾部）。
//!
//! # 无 git 在 worker 上（change solution-git-offload, design D1）
//!
//! layer runtime 是 `worker_threads(1)`，同步 `Command::output()` 会占住
//! 唯一 worker —— 大仓一次 `git status` 冻结整层（spec: Git work does
//! not delay other services）。因此同步实现（[`run`] / [`run_ok`] /
//! [`is_tracked_clean`]）**模块外不可见**：模块外只拿得到异步出口
//! （[`run_async`] / [`run_ok_async`] / [`is_tracked_clean_async`]），
//! 子进程落在 `spawn_blocking` 阻塞池，编译期钉死这条规则。

use std::path::Path;
use std::process::{Command, Stdio};

/// stderr 保留的尾部字节上限（超限丢头部、留尾部, design D6）。
const STDERR_TAIL: usize = 4096;

/// git 命令产出（退出码 + 两侧原文, stderr 已尾部截断）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// git 调用的结构化错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// git 无法启动（二进制缺失/不可执行）—— spawn 失败, 非零退出不走这里。
    Spawn(String),
    /// 命令非零退出；`stderr` 为尾部截断原文（`worktree remove` 的门
    /// 文案、pull/push 失败原因都从这里进 notice）。
    Failed { code: i32, stderr: String },
    /// 输入不合法（如 worktree 目录名消毒失败, design D4）—— 未跑 git 即拒。
    InvalidName(String),
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitError::Spawn(m) => write!(f, "git unavailable: {m}"),
            GitError::Failed { code, stderr } => {
                let msg = stderr.trim();
                if msg.is_empty() {
                    write!(f, "git exited with code {code}")
                } else {
                    write!(f, "{msg}")
                }
            }
            GitError::InvalidName(m) => write!(f, "invalid worktree name: {m}"),
        }
    }
}

impl std::error::Error for GitError {}

pub mod worktree;

/// 跑 `git <args>` 于 `cwd`：spawn 失败 → [`GitError::Spawn`]；否则
/// 返回产出（**不**把非零退出当错误 —— 门控类调用要读 code/stderr 自行裁决）。
fn run(args: &[&str], cwd: &Path) -> Result<GitOutput, GitError> {
    run_program("git", args, cwd)
}

/// 跑 `git <args>` 并要求退出码 0：成功返回 stdout；非零 →
/// [`GitError::Failed`]（携带尾部 stderr）。
fn run_ok(args: &[&str], cwd: &Path) -> Result<String, GitError> {
    let out = run(args, cwd)?;
    if out.code == 0 {
        Ok(out.stdout)
    } else {
        Err(GitError::Failed {
            code: out.code,
            stderr: out.stderr,
        })
    }
}

/// tracked-clean 判据（change solution-git-worktree, design D2）：
/// `status --porcelain --untracked-files=no` 为空。
///
/// **只看 tracked 改动** —— untracked（含应用自建的 `.balabala.worktrees/`，
/// spike 实证它会出现在主仓 porcelain）不阻断 pull/push；非 git 目录或
/// status 失败 → `Err`（按不干净处理, 门控侧走拒绝路径）。
fn is_tracked_clean(root: &Path) -> Result<bool, GitError> {
    let out = run(&["status", "--porcelain", "--untracked-files=no"], root)?;
    if out.code != 0 {
        return Err(GitError::Failed {
            code: out.code,
            stderr: out.stderr,
        });
    }
    Ok(out.stdout.trim().is_empty())
}

/// 异步出口：`git <args>` 于 `cwd`，子进程在**阻塞池**执行（不占单
/// worker，spec: Git work does not delay other services）。
pub async fn run_async(args: &[&str], cwd: &Path) -> Result<GitOutput, GitError> {
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let cwd = cwd.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        run(&borrowed, &cwd)
    })
    .await
    .map_err(|e| GitError::Spawn(format!("git task: {e}")))?
}

/// 异步出口：[`run_ok`] 的阻塞池版（见 [`run_async`] 的规则说明）。
pub async fn run_ok_async(args: &[&str], cwd: &Path) -> Result<String, GitError> {
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let cwd = cwd.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        run_ok(&borrowed, &cwd)
    })
    .await
    .map_err(|e| GitError::Spawn(format!("git task: {e}")))?
}

/// 异步出口：[`is_tracked_clean`] 的阻塞池版（大仓 status 是五个占线点
/// 里最慢的一个 —— 门控也必须离 worker，change solution-git-offload D2）。
pub async fn is_tracked_clean_async(root: &Path) -> Result<bool, GitError> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || is_tracked_clean(&root))
        .await
        .map_err(|e| GitError::Spawn(format!("git task: {e}")))?
}

/// 实际执行（`program` 可注入, 供「二进制缺失」路径单测）。
///
/// 防挂三件套全在这一处（design D6）：stdin null、`GIT_TERMINAL_PROMPT=0`、
/// `GIT_ASKPASS=:`。
fn run_program(program: &str, args: &[&str], cwd: &Path) -> Result<GitOutput, GitError> {
    // 门闩探针 (cfg(test), change solution-git-offload 4.1): 卡住目标
    // cwd 的子进程入口并记录执行线程 —— 离载断言用, 正常构建无此句。
    #[cfg(test)]
    probe::on_run(cwd);
    // `\\?\` 扩展长度前缀: Rust fs 认, git (MSYS) 不认 —— argv 里的
    // `\\?\D:\x` 会被转成 `//?/D:/x` → mkdir "Invalid argument" (真机
    // D:\Balabala 冒烟: worktree add 必炸于 "could not create leading
    // directories …"; 探针同形复现)。cwd 不走 argv 转换 (CreateProcess
    // 直收, pull 在带前缀仓上实测通过) —— 只剥 argv, cwd 不动。
    let args: Vec<&str> = args.iter().map(|a| strip_extended_prefix(*a)).collect();
    let out = Command::new(program)
        .args(&args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", ":")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| GitError::Spawn(format!("{program}: {e}")))?;
    Ok(GitOutput {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: tail_lossy(&out.stderr),
    })
}

/// 剥 `\\?\` 扩展长度前缀（仅 Windows 形态）。
///
/// git (MSYS 路径层) 不认该前缀：argv 里的 `\\?\D:\x` 会被转成
/// `//?/D:/x` → mkdir "Invalid argument" —— 真机 D:\Balabala 冒烟
/// `worktree add` 必炸于 "could not create leading directories …"。
/// cwd 不走 argv 转换（CreateProcess 直收）, 实测无碍 → 不剥。
#[cfg(windows)]
fn strip_extended_prefix(arg: &str) -> &str {
    arg.strip_prefix("\\\\?\\").unwrap_or(arg)
}

/// 非 Windows 无此形态 → 恒等。
#[cfg(not(windows))]
fn strip_extended_prefix(arg: &str) -> &str {
    arg
}

/// 取字节尾部且不切断 UTF-8 字符（多字节序列的续字节 10xxxxxx 跳过）。
fn tail_lossy(bytes: &[u8]) -> String {
    let slice = if bytes.len() <= STDERR_TAIL {
        bytes
    } else {
        let mut start = bytes.len() - STDERR_TAIL;
        // 落点多字节序列的续字节 (10xxxxxx) 时前移到下一字符起点
        while start < bytes.len() && bytes[start] & 0xC0 == 0x80 {
            start += 1;
        }
        &bytes[start..]
    };
    String::from_utf8_lossy(slice).into_owned()
}

/// 门闩探针（`cfg(test)`, change solution-git-offload 4.1）：卡住指定
/// `cwd` 的 git 子进程入口, 记录**实际执行线程**供离载断言 —— 看门狗
/// 3s 自动放行, 实现写错时测试是"失败"而不是"挂死"（asset-service
/// `offload_tests` 同款纪律）。
#[cfg(test)]
pub(crate) mod probe {
    use std::path::Path;
    use std::sync::{Condvar, LazyLock, Mutex};
    use std::time::{Duration, Instant};

    struct State {
        /// 上闩的目标 cwd（只有匹配的调用被卡）。
        armed: Option<std::path::PathBuf>,
        /// 门闩命中时的执行线程。
        hit: Option<std::thread::ThreadId>,
        released: bool,
        /// 自动放行期限（arm 起 3s）。
        deadline: Instant,
    }

    static STATE: LazyLock<(Mutex<State>, Condvar)> = LazyLock::new(|| {
        (
            Mutex::new(State {
                armed: None,
                hit: None,
                released: false,
                deadline: Instant::now(),
            }),
            Condvar::new(),
        )
    });

    fn lock() -> std::sync::MutexGuard<'static, State> {
        STATE.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 上闩：此后 `cwd` 匹配的 git 调用会被卡住并记录线程。
    pub(crate) fn arm(cwd: &Path) {
        let mut st = lock();
        st.armed = Some(cwd.to_path_buf());
        st.hit = None;
        st.released = false;
        st.deadline = Instant::now() + Duration::from_secs(3);
    }

    /// 等门闩命中, 返回实际执行 git 的线程；超时 → `None`（测试据此失败）。
    pub(crate) fn wait_hit(timeout: Duration) -> Option<std::thread::ThreadId> {
        let deadline = Instant::now() + timeout;
        let mut st = lock();
        loop {
            if let Some(t) = st.hit {
                return Some(t);
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let (guard, wait) = STATE
                .1
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            st = guard;
            if wait.timed_out() {
                return st.hit;
            }
        }
    }

    /// 放行（看门狗到期也会自动放行）。
    pub(crate) fn release() {
        let mut st = lock();
        st.released = true;
        STATE.1.notify_all();
    }

    /// `run_program` 入口钩子：目标调用 → 记线程 → 等放行。
    pub(crate) fn on_run(cwd: &Path) {
        let mut st = lock();
        if st.armed.as_deref() != Some(cwd) || st.released {
            return;
        }
        st.hit = Some(std::thread::current().id());
        STATE.1.notify_all();
        while !st.released && Instant::now() < st.deadline {
            st = STATE.1.wait(st).unwrap_or_else(|e| e.into_inner());
        }
        // 到期自动放行: 实现写错时测试失败而不是挂死
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("git-mod-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// 任务 1.1 验证: 缺 git 二进制 → 结构化 Spawn 错误（非 panic）。
    #[test]
    fn missing_binary_is_structured_error() {
        let err = run_program("no-such-git-binary-solution-gw", &["--version"], &scratch("spawn"))
            .expect_err("注入的假程序必须 spawn 失败");
        assert!(matches!(err, GitError::Spawn(_)), "应为 Spawn: {err:?}");
        assert!(err.to_string().contains("no-such-git-binary"));
    }

    /// `\\?\` 前缀剥除 (真机 D:\Balabala 冒烟): argv 带前缀 → git 转
    /// `//?/D:/` → mkdir EINVAL (worktree add 必炸); cwd 不剥 (实测无碍),
    /// 普通参数不动。非 Windows 恒等 → 测试门控 Windows。
    #[cfg(windows)]
    #[test]
    fn extended_prefix_stripped_from_args() {
        assert_eq!(
            strip_extended_prefix("\\\\?\\D:\\ws\\.balabala.worktrees\\x"),
            "D:\\ws\\.balabala.worktrees\\x"
        );
        assert_eq!(strip_extended_prefix("D:\\plain"), "D:\\plain");
        assert_eq!(strip_extended_prefix("--branch"), "--branch");
    }

    /// 任务 1.1 验证: 正常 `git --version` 走通出口。
    #[test]
    fn version_runs_through_the_exit() {
        let out = run(&["--version"], &scratch("ver")).expect("git --version 应可用");
        assert_eq!(out.code, 0);
        assert!(
            out.stdout.starts_with("git version"),
            "stdout: {}",
            out.stdout
        );
    }

    /// run_ok: 0 退 → stdout；非零 → Failed 带 stderr 尾部。
    #[test]
    fn run_ok_maps_exit_codes() {
        let dir = scratch("ok");
        let ok = run_ok(&["--version"], &dir).expect("version ok");
        assert!(ok.contains("git version"));

        // 非 git 仓跑 status → 非零退出, stderr 是 git 的原话
        let err = run_ok(&["status", "--porcelain"], &dir)
            .expect_err("非 git 目录 status 必须非零");
        match err {
            GitError::Failed { code, stderr } => {
                assert_ne!(code, 0);
                assert!(!stderr.trim().is_empty(), "Failed 必须携带 stderr 原文");
            }
            other => panic!("应为 Failed: {other:?}"),
        }
    }

    /// 尾部截断: 超限留尾、且不切断多字节字符。
    #[test]
    fn stderr_tail_keeps_the_end_and_char_boundaries() {
        // 超限: 留最后 STDERR_TAIL 字节的头部对齐到字符边界
        let mut big = Vec::new();
        big.extend(std::iter::repeat_n(b'a', STDERR_TAIL + 100));
        big.extend("错误尾部".as_bytes());
        let t = tail_lossy(&big);
        assert!(t.ends_with("错误尾部"), "尾部必须保留");
        assert!(t.len() <= STDERR_TAIL + "错误尾部".len());

        // 不超限: 原样
        assert_eq!(tail_lossy("git: boom".as_bytes()), "git: boom");
    }

    /// 任务 1.2 验证: 三态仓 —— 干净 / 仅 untracked → 通过 / tracked 改动 → 拒。
    #[test]
    fn tracked_clean_three_states() {
        let repo = scratch("clean");
        let git = |args: &[&str], cwd: &Path| {
            run_ok(args, cwd).unwrap_or_else(|e| panic!("git {args:?} 失败: {e}"))
        };
        git(&["init", "-q"], &repo);
        git(&["config", "user.email", "t@t"], &repo);
        git(&["config", "user.name", "t"], &repo);
        std::fs::write(repo.join("tracked.txt"), "v1").unwrap();
        git(&["add", "tracked.txt"], &repo);
        git(&["commit", "-q", "-m", "init"], &repo);

        // 态1: 干净 → true
        assert_eq!(is_tracked_clean(&repo).expect("干净仓 status 可跑"), true);

        // 态2: 仅 untracked → 仍 true (spike: .balabala.worktrees/ 不阻门)
        std::fs::write(repo.join("untracked.txt"), "new").unwrap();
        assert_eq!(
            is_tracked_clean(&repo).expect("untracked 不算脏"),
            true,
            "untracked 文件不得阻断 tracked-clean 门"
        );

        // 态3: tracked 改动 → false
        std::fs::write(repo.join("tracked.txt"), "v2-dirty").unwrap();
        assert_eq!(
            is_tracked_clean(&repo).expect("status 可跑"),
            false,
            "tracked 改动必须判脏"
        );

        // 非 git 目录 → Err (按不干净处理)
        let plain = scratch("not-a-repo");
        assert!(matches!(
            is_tracked_clean(&plain),
            Err(GitError::Failed { .. })
        ));
    }
}
