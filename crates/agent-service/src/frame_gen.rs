//! 帧代际号原语（change `2026-11-20-streaming-input-echo-latency` 任务 1.2，
//! design Decisions 方案 A）：输入注入 dispatch 推进代际（[`advance`]），
//! 帧在 take_ansi/推送处取当时代际（[`current`]）打标（`FrameBytes.gen`）。
//!
//! **"帧代际 > 输入基线" = 该帧字节集产生于输入注入之后**——echo 口径共用
//! （design Decisions 代际号原语表）：`rate_log` 只计代际 > 基线的首帧为回显
//! 样本（H4 口径修正）。注：同代际不区分流式续帧/回显帧（探针盲区，见
//! design.md 归因段）；探针/关窗归因已随任务 2.3 拆除。
//!
//! 代际推进点在**注入 dispatch 的内联 drain 之后**（`service::dispatch`）：
//! - `note_input`→注入之间产出的帧仍是旧代际，不会被误计为回显；
//! - 内联 drain 取走的残留字节画于输入生效前，同属旧代际。

use std::sync::atomic::{AtomicU64, Ordering};

/// 输入代际：每次输入 dispatch（内联 drain 之后）+1。
static GEN: AtomicU64 = AtomicU64::new(0);

/// 输入 dispatch 推进代际（service 侧 `dispatch` 内联 drain 之后调用）。
pub fn advance() {
    GEN.fetch_add(1, Ordering::Relaxed);
}

/// 当前代际快照——帧打标（take_ansi/推送处）与输入基线（`note_input`）
/// 共用同一读数。
pub fn current() -> u64 {
    GEN.load(Ordering::Relaxed)
}
