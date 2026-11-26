//! 服务核心 — 进程级状态 (design D2) + 单 worker 线程 (design D5) + 公开查询面。
//!
//! **状态归进程级单例**: [`STATE`] 持 `RwLock<ServiceState>` — 后端、状态
//! 快照、诊断缓存、注入代际。DebugView 关签不影响它; 后续 solution-service
//! 注入走同一个 [`set_roots`]。
//!
//! **worker 纪律 (design D5)**:
//! - 重活 (建后端 = init_std_lib + fs 收集) 在**锁外**做;
//! - 装入/分批喂文件/落诊断各持**短写锁**, 批间释放 — 读方 (search/
//!   diagnostics) 只等当前一批, 看到"已索引的部分状态" (spec: 不等待整轮);
//! - 代际 `generation` 每阶段在锁内比对 — 新 `set_roots` 使陈旧 job 在
//!   下一个检查点中止, 陈旧结果永不落地 (spec: 全量替换 + abort)。
//!
//! 公开 API 四件套: [`set_roots`] / [`search_symbols`] /
//! [`diagnostics_all`] / [`index_status`] — 只出本 crate 类型。

use std::path::PathBuf;
use std::sync::{mpsc, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Instant;

#[cfg(test)]
use std::time::Duration;

use crate::backend::{Backend, create_backend};
use crate::types::{IndexPhase, IndexStatus, LuaDiagnostic, SymbolHit};

/// 分批喂文件的批大小: 每批一记短写锁 — 读方最坏等一批 (design D5)。
const CHUNK: usize = 32;

/// 后台任务: 按代际全量重建索引 (唯一任务类型, 单 worker 串行消费)。
enum Job {
    Reindex {
        roots: Vec<PathBuf>,
        generation: u64,
    },
}

/// 进程级状态 (design D2)。
struct ServiceState {
    /// 当前后端; `None` = 无 workspace (查询返回空 — spec: no workspace)。
    /// `set_roots` 同步置 None → 旧 roots 立即不可见 (全量替换)。
    backend: Option<Box<dyn Backend>>,
    /// 对外状态快照 (查询克隆出去)。
    status: IndexStatus,
    /// 最近一次成功落定的诊断 (陈旧 sweep 永不覆盖 — 代际守卫)。
    diagnostics: Vec<LuaDiagnostic>,
    /// 注入代际: 每次 `set_roots` 递增; worker 各阶段在锁内比对,
    /// 不符即中止 — design D5 的 abort flag (代际即旗标)。
    generation: u64,
}

impl ServiceState {
    fn new() -> Self {
        Self {
            backend: None,
            status: IndexStatus {
                roots: Vec::new(),
                file_count: 0,
                elapsed_ms: None,
                phase: IndexPhase::Empty,
                error: None,
            },
            diagnostics: Vec::new(),
            generation: 0,
        }
    }
}

/// 进程级单例 (design D2)。
fn state() -> &'static RwLock<ServiceState> {
    static STATE: OnceLock<RwLock<ServiceState>> = OnceLock::new();
    STATE.get_or_init(|| RwLock::new(ServiceState::new()))
}

/// 读锁 — 毒锁恢复 (worker 若持锁 panic, 查询面不随之瘫痪)。
fn read() -> RwLockReadGuard<'static, ServiceState> {
    state().read().unwrap_or_else(|p| p.into_inner())
}

fn write() -> RwLockWriteGuard<'static, ServiceState> {
    state().write().unwrap_or_else(|p| p.into_inner())
}

/// 惰性启动单 worker 线程 (首个 `set_roots` 时)。
fn worker() -> &'static mpsc::Sender<Job> {
    static WORKER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("lua-index-worker".into())
            .spawn(move || worker_loop(rx))
            .expect("spawn lua-index-worker");
        tx
    })
}

fn worker_loop(rx: mpsc::Receiver<Job>) {
    while let Ok(job) = rx.recv() {
        match job {
            Job::Reindex { roots, generation } => run_reindex(generation, &roots),
        }
    }
}

// ---------- 公开 API ----------

/// 全量替换注入 workspace roots (spec: Root injection replaces the workspace set)。
///
/// 同步部分只做: 代际递增 + 丢弃旧后端 + 状态复位 (µs 级) — 索引本身
/// 一律丢给后台 worker (spec: Analysis runs off the UI thread)。
/// `roots` 为空 = 清空整个 workspace, 不投递任务。
pub fn set_roots(roots: Vec<PathBuf>) {
    let generation = {
        let mut st = write();
        st.generation += 1;
        let generation = st.generation;
        // 全量替换的同步半边: 旧后端立离场 → 查询即刻不见旧 roots。
        st.backend = None;
        st.diagnostics.clear();
        st.status = IndexStatus {
            roots: roots.clone(),
            file_count: 0,
            elapsed_ms: None,
            phase: if roots.is_empty() {
                IndexPhase::Empty
            } else {
                IndexPhase::InProgress
            },
            error: None,
        };
        generation
    };
    if !roots.is_empty() {
        // 投递失败仅当 worker 线程已亡 (不可达: worker_loop 不退出)。
        let _ = worker().send(Job::Reindex { roots, generation });
    }
}

/// 符号搜索 (spec: Symbol search answers by name)。
///
/// 无 workspace 注入 → 空列表 (spec: Search with no workspace returns empty)。
pub fn search_symbols(query: &str) -> Vec<SymbolHit> {
    let st = read();
    match &st.backend {
        Some(b) => b.search(query),
        None => Vec::new(),
    }
}

/// 工作区诊断快照 (spec: Workspace diagnostics cover all indexed files)。
///
/// 由 worker 在每轮索引完成后后台扫描落定; 无 workspace → 空列表。
pub fn diagnostics_all() -> Vec<LuaDiagnostic> {
    read().diagnostics.clone()
}

/// 索引状态快照 (spec: Lua index section injects roots and shows index state)。
pub fn index_status() -> IndexStatus {
    read().status.clone()
}

// ---------- worker 任务 ----------

/// 代际仍当前? (调用方须持锁 — 状态与代际原子比对)。
fn is_current(st: &ServiceState, generation: u64) -> bool {
    st.generation == generation
}

fn run_reindex(generation: u64, roots: &[PathBuf]) {
    // 阶段 0: 入队即过期 (连续注入) → 直接跳过。
    if read().generation != generation {
        return;
    }
    let started = Instant::now();

    // 阶段 1: 锁外重活 — 建后端 (init_std_lib + fs 收集全部内容)。
    let (backend, files) = match create_backend(roots) {
        Ok(x) => x,
        Err(e) => {
            fail(generation, e);
            return;
        }
    };
    if read().generation != generation {
        return;
    }

    // 阶段 2: 装入 (短写锁) — 从此查询可见该后端 (尚无 workspace 文件)。
    {
        let mut st = write();
        if !is_current(&st, generation) {
            return;
        }
        st.backend = Some(backend);
    }

    // 阶段 3: 分批喂文件 — 每批一记短写锁, 批间读方插队,
    // 查询看到"已索引的部分状态" (spec: 不等待整轮分析)。
    let mut remaining = files;
    while !remaining.is_empty() {
        if read().generation != generation {
            return; // 新注入接管 — 本 job 中止
        }
        let n = remaining.len().min(CHUNK);
        let chunk: Vec<_> = remaining.drain(..n).collect();
        let mut st = write();
        if !is_current(&st, generation) {
            return;
        }
        let Some(b) = st.backend.as_mut() else {
            return; // 不可达 (本 job 装入) — 防御
        };
        b.index_chunk(chunk);
        st.status.file_count += n;
    }

    // 阶段 4: 索引完成 — elapsed 落快照 (phase 仍 InProgress, 诊断随后)。
    {
        let mut st = write();
        if !is_current(&st, generation) {
            return;
        }
        st.status.elapsed_ms = Some(started.elapsed().as_millis() as u64);
    }

    // 阶段 5: 诊断扫描 — 逐文件短读锁; 每文件前比对代际,
    // 新注入即中止且**已收集部分不落地** (spec: abort stale run)。
    let targets = {
        let st = read();
        if !is_current(&st, generation) {
            return;
        }
        match &st.backend {
            Some(b) => b.workspace_files(),
            None => return,
        }
    };
    let mut collected = Vec::new();
    for path in &targets {
        let batch = {
            let st = read();
            if !is_current(&st, generation) {
                return; // 中止: 陈旧 sweep 结果丢弃
            }
            match &st.backend {
                Some(b) => b.diagnose(path),
                None => return,
            }
        };
        collected.extend(batch);
    }

    // 阶段 6: 落定 — 诊断 + Idle (一个 job 的完整终点)。
    {
        let mut st = write();
        if !is_current(&st, generation) {
            return;
        }
        st.diagnostics = collected;
        st.status.phase = IndexPhase::Idle;
    }
}

/// job 失败 (root 非法等) — 代际仍当前才落 Failed。
fn fail(generation: u64, error: String) {
    let mut st = write();
    if is_current(&st, generation) {
        st.status.phase = IndexPhase::Failed;
        st.status.error = Some(error);
    }
}

/// 轮询等待分析终点 (Idle/Failed) — 测试与调试用; 超时 panic 暴露挂死。
#[cfg(test)]
pub(crate) fn wait_settled() -> IndexStatus {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let st = index_status();
        match st.phase {
            IndexPhase::Idle | IndexPhase::Failed => return st,
            IndexPhase::Empty => panic!("等待中出现 Empty (未注入?): {st:?}"),
            IndexPhase::InProgress => {
                if Instant::now() >= deadline {
                    panic!("等待分析落定超时: {st:?}");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

/// 测试串行锁 (term_rate TEST_LOCK 同款): 状态是**进程级单例**,
/// 本 crate 的 service/view 测试共用同一份 — 并行跑会互相踩 roots。
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
