//! 中间层自建 tokio 运行时（design D9）：自建自持、默认最小线程数、
//! **对外不泄漏 tokio**——宿主 API（双门）不出现任何 tokio/kameo 类型。

use std::future::Future;

/// 中间层运行时：`Router`/driver/定时器/转发循环全在这里。
///
/// - 自建自持：由 [`crate::ServiceManager::new`] 创建并持有，宿主零侵入（D9）；
/// - 最小线程数（1 worker）：Risks 条目"默认最小线程数"；阻塞活走 `spawn_blocking`；
/// - **必须在非 tokio 上下文创建**（tokio 运行时不能嵌套创建——GUI 主线程 /
///   测试同步线程均满足）。
pub struct LayerRuntime {
    /// `Option` 仅为 [`Drop`] 能拿走所有权做非等待关停（见 Drop 注释）。
    rt: Option<tokio::runtime::Runtime>,
}

impl LayerRuntime {
    pub fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("service-layer")
            .build()
            .expect("build layer runtime");
        LayerRuntime { rt: Some(rt) }
    }

    /// 运行时句柄（内部用：eager spawn、定时器）。
    pub(crate) fn handle(&self) -> &tokio::runtime::Handle {
        &self.rt.as_ref().expect("layer runtime alive").handle()
    }

    /// 急切 spawn（不等 future 被 poll——D9 `call()` 的实现基础）。
    pub(crate) fn spawn<F>(&self, fut: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.rt.as_ref().expect("layer runtime alive").spawn(fut)
    }
}

impl Drop for LayerRuntime {
    fn drop(&mut self) {
        // 关窗收尾语义: **不等在途 spawn_blocking**（tokio 默认 Drop
        // 无限期等待——源码文档 "block indefinitely for spawned blocking
        // tasks"）。资产枚举正用它遍历项目树（真机 D:\UnrealEngine 百万
        // 级文件 + 杀软逐个扫），等它跑完才退 = 关窗后进程残留数分钟。
        // shutdown_background 即 shutdown_timeout(0): 立即返回, 在途
        // 阻塞线程随进程消亡; 闭包数据 move 独占, 无共享悬垂。
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

impl Default for LayerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// 关窗收尾语义（真机 D:\UnrealEngine 残留复盘）：Drop **不得**等在途
    /// `spawn_blocking` —— tokio 默认 Drop 会无限期等待（runtime.rs:466
    /// 文档原话 "block indefinitely for spawned blocking tasks"），而资产
    /// 枚举正用它遍历项目树（百万级文件 + 杀软逐个扫），等它跑完才退 =
    /// 关窗后进程残留数分钟。
    #[test]
    fn drop_returns_while_blocking_task_pending() {
        let rt = LayerRuntime::new();
        let handle = rt.handle().clone();
        handle.spawn_blocking(|| std::thread::sleep(Duration::from_secs(3)));
        std::thread::sleep(Duration::from_millis(100)); // 让阻塞任务真正开跑
        let t = Instant::now();
        drop(rt);
        let elapsed = t.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "Drop 等了在途 spawn_blocking: {elapsed:?} (关窗残留根因)"
        );
    }
}
