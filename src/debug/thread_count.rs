//! debug-console change task 4.1: 进程真实线程数 (design D9)。
//!
//! 线程大头在 pi-shell-rs (git 依赖, 无计数接口) — 估算不可信, 走 OS 真值:
//! - macOS: `proc_pidinfo(PROC_PIDTASKINFO)` → `proc_taskinfo.pti_threadnum`
//! - Linux: 读 `/proc/self/status` 的 `Threads:` 行
//! - Windows: Toolhelp32 `TH32CS_SNAPTHREAD` 快照, 数本 pid 的线程
//!
//! 失败一律 `None` (spec: 显示 unavailable, 不给编造数字)。

// 调用计数探针 (task 7.2: "不可见即不采样" 断言) — cfg(test) **线程
// 本地**: 无头测试同步跑在测试线程上 (测试调度器在调用线程执行),
// section 采样与断言同线程, 只数自己线程的 OS 查询 — 4.1 解析用例
// 等并发测试各数各线程, 绝对值断言互不干扰。
#[cfg(test)]
thread_local! {
    static OS_CALLS: std::cell::Cell<u64> = std::cell::Cell::new(0);
}

/// 本线程累计调用 [`process_thread_count`] 的次数 (cfg(test) 探针读口)。
#[cfg(test)]
pub fn os_query_calls() -> u64 {
    OS_CALLS.with(|c| c.get())
}

/// 本进程当前线程数 (OS 真值); 平台查询失败返回 `None`。
pub fn process_thread_count() -> Option<u32> {
    #[cfg(test)]
    OS_CALLS.with(|c| c.set(c.get() + 1));
    platform_thread_count()
}

#[cfg(target_os = "macos")]
fn platform_thread_count() -> Option<u32> {
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as i32;
    let filled = unsafe {
        libc::proc_pidinfo(
            std::process::id() as i32,
            libc::PROC_PIDTASKINFO,
            0,
            &mut info as *mut libc::proc_taskinfo as *mut libc::c_void,
            size,
        )
    };
    // proc_pidinfo 固定结构: 成功返回恰好 size; pti_threadnum 恒 >= 1 (主线程)
    if filled == size && info.pti_threadnum > 0 {
        Some(info.pti_threadnum as u32)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn platform_thread_count() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    parse_threads_line(&status)
}

#[cfg(windows)]
fn platform_thread_count() -> Option<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };

    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let pid = std::process::id();
        let mut count = 0u32;
        let mut has = Thread32First(snap, &mut entry) != 0;
        while has {
            if entry.th32OwnerProcessID == pid {
                count += 1;
            }
            has = Thread32Next(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
        (count > 0).then_some(count)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn platform_thread_count() -> Option<u32> {
    None
}

/// `/proc/self/status` 的 `Threads:` 行解析 (纯函数 — 各平台单测共用)。
fn parse_threads_line(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// macOS/Windows 走各自平台分支, Linux 走 /proc — 三者都必须 >= 1。
    /// (失败 = 平台查询坏掉, spec 要求显示 unavailable 而非 0/编造值)
    #[test]
    fn thread_count_is_at_least_one() {
        let n = process_thread_count().expect("本平台应能取到线程数");
        assert!(n >= 1, "线程数应 >= 1, 得到 {n}");
    }

    #[test]
    fn parses_threads_line() {
        let sample = "Name:\tpi\nUmask:\t0022\nState:\tS (sleeping)\nThreads:\t42\n";
        assert_eq!(parse_threads_line(sample), Some(42));
    }

    #[test]
    fn missing_threads_line_is_none() {
        assert_eq!(parse_threads_line("Name:\tpi\nState:\tR\n"), None);
    }

    #[test]
    fn non_numeric_threads_is_none() {
        assert_eq!(parse_threads_line("Threads:\tNaN\n"), None);
    }

    /// 真实 `/proc/self/status` (仅 Linux 跑; macOS 上本体由上面的
    /// sample 用例覆盖同一解析路径)。
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_real_proc_status_parses() {
        let status = std::fs::read_to_string("/proc/self/status").expect("可读 /proc/self/status");
        let n = parse_threads_line(&status).expect("含 Threads: 行");
        assert!(n >= 1, "真实线程数应 >= 1, 得到 {n}");
    }
}
