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
    rt: tokio::runtime::Runtime,
}

impl LayerRuntime {
    pub fn new() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("service-layer")
            .build()
            .expect("build layer runtime");
        LayerRuntime { rt }
    }

    /// 运行时句柄（内部用：eager spawn、定时器）。
    pub(crate) fn handle(&self) -> &tokio::runtime::Handle {
        self.rt.handle()
    }

    /// 急切 spawn（不等 future 被 poll——D9 `call()` 的实现基础）。
    pub(crate) fn spawn<F>(&self, fut: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.rt.spawn(fut)
    }
}

impl Default for LayerRuntime {
    fn default() -> Self {
        Self::new()
    }
}
