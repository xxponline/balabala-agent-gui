//! Listener：拉模式 actor 的 mailbox 句柄（design D9 的 **poll 门**）。
//!
//! **GUI 就是总线上的拉模式参与者**：有自己的 Route、自己的 mailbox（Router
//! per-outlet 队列），`poll()` = 一次消息循环迭代，由宿主自己的泵驱动
//! （gpui 泵/测试线程），同步执行。
//!
//! - **无回调 API**：跨线程回调合同问题从结构上消失（B 方案定案）；
//! - **订阅塌缩成路由**：`ServiceManager::subscribe_state` 即往路由表加规则；
//! - **快照零回程**：变更事件自带快照 `Arc`（payload 本来就 Arc 透传），
//!   [`Listener::snapshot`] = 各 kind 的最后一条 payload，按类型取；
//! - **合并红利**：连续 N 次变更 [`Listener::poll`] 只亮一次 changed；
//!   Reply 事件**不合并**（drain 逐条保 corr 语义）。

use crate::router::{Completion, Router};
use service_core::{Address, Corr, Envelope, InstanceId, Payload, Route, RouteError};
use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// 拉模式参与者的 mailbox 句柄（宿主侧 API，不含任何 tokio/kameo 类型）。
pub struct Listener {
    router: Router,
    id: InstanceId,
    addr: Address,
    /// 自己的 outlet（submit 回复收口直达本 mailbox）。
    outlet: mpsc::Sender<Envelope>,
    rx: mpsc::Receiver<Envelope>,
    /// poll 灌入的本地队列（drain 逐条取件）。
    local: VecDeque<Envelope>,
    /// 自上次 poll 以来是否有新件（合并 N 次为一次 true）。
    changed: bool,
    /// 各 kind 最后一条 payload（快照 = mailbox 最后一值，零回程）。
    snapshots: HashMap<&'static str, Payload>,
}

impl Listener {
    pub(crate) fn new(
        router: Router,
        id: InstanceId,
        outlet: mpsc::Sender<Envelope>,
        rx: mpsc::Receiver<Envelope>,
    ) -> Self {
        Listener {
            router,
            id,
            addr: Address::instance(id),
            outlet,
            rx,
            local: VecDeque::new(),
            changed: false,
            snapshots: HashMap::new(),
        }
    }

    /// 本参与者的 id（= `Route::Instance(..)` 目标）。
    pub fn id(&self) -> InstanceId {
        self.id
    }

    /// 本参与者的地址（作 `from`）。
    pub fn address(&self) -> &Address {
        &self.addr
    }

    /// 一次消息循环迭代：把 mailbox 里的件灌入本地队列并刷新快照。
    ///
    /// 返回 `true` 表示**自上次 poll 以来**有新件（连续 N 次翻转合并为
    /// 一次 `true`；此后无新件返回 `false`）。
    pub fn poll(&mut self) -> bool {
        while let Ok(env) = self.rx.try_recv() {
            self.snapshots.insert(env.kind, env.payload.clone());
            self.local.push_back(env);
            self.changed = true;
        }
        std::mem::take(&mut self.changed)
    }

    /// 取走全部待处理信封（逐条，保 corr 语义——Reply 不合并）。
    pub fn drain(&mut self) -> std::collections::vec_deque::Drain<'_, Envelope> {
        self.local.drain(..)
    }

    /// 当前快照：poll 过的各 kind 最后一条 payload 中首个能按 `T` downcast
    /// 的值——**零回程**，不需要额外 ask。
    pub fn snapshot<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        for payload in self.snapshots.values() {
            let Payload::Typed(arc) = payload;
            if let Ok(typed) = Arc::clone(arc).downcast::<T>() {
                return Some(typed);
            }
        }
        None
    }

    /// submit 门（默认超时）：发起 ask，回复/错误以信封落本 mailbox 等 poll 取件。
    /// 返回 corr，供 drain 时配对。
    pub fn submit(&self, to: Route, kind: &'static str, payload: Payload) -> Result<Corr, RouteError> {
        self.submit_with_timeout(to, kind, payload, self.router.default_timeout())
    }

    /// submit 门（显式超时）。
    pub fn submit_with_timeout(
        &self,
        to: Route,
        kind: &'static str,
        payload: Payload,
        timeout: Duration,
    ) -> Result<Corr, RouteError> {
        let env = Envelope::new(to, self.addr.clone(), kind, payload);
        self.router
            .dispatch(env, self.addr.clone(), Completion::Mailbox(self.outlet.clone()), timeout)
    }
}
