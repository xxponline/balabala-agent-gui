//! 中心 Router（design D3/D4/D5/D6）：全部消息（宿主 actor、GUI、插件 actor）
//! 走统一信封过此——路由表里没有特权参与者。
//!
//! 关键语义：
//! - **两级寻址**：`Route::Instance(..)` 直投；`Service(name, Direct(id))`
//!   带"实例属于该服务"断言；`Service(name, Any)` 挑 `available` 实例。
//! - **fail-fast 零缓冲**：目标非 `available` 立即拒绝；per-outlet 有界通道
//!   满 = 即刻失败（try_send，调用方不干等超时）；Router 无隐藏队列。
//! - **ask**：`corr` 进 pending 表，回复按 `corr` 销账（**不按 from 寻址**），
//!   超时由 Router 持有（到期销账 + 落死信 + `Err(Timeout)`）。
//! - **观测面**：死信表 + 消息追踪（一问一答全程留痕）+ 状态事件转发。

use crate::instance::Instances;
use service_core::{
    Address, Corr, DeadLetterReason, Envelope, InstanceId, Payload, Route, RouteError,
    SelectPolicy, ServiceState, StateChange, KIND_STATE_CHANGE, KIND_ROUTE_ERROR,
};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

/// 消息追踪环缓冲上限（观测面，不看每帧内容的承诺见 D7 逃生门注记）。
const TRACE_CAP: usize = 1024;
/// 死信表上限（环形，最旧淘汰）。
const DEAD_LETTER_CAP: usize = 1024;
/// Router 自身的观测地址（框架自发信封的 `from`；id 0 不入参与者表）。
const ROUTER_ID: InstanceId = InstanceId(0);

/// 死信记录（可观测面：ask/tell 失败、超时、晚到回复）。
#[derive(Clone, Debug)]
pub struct DeadLetterRecord {
    pub reason: DeadLetterReason,
    pub kind: &'static str,
    pub corr: Option<Corr>,
    pub to: Route,
    pub envelope: Option<Envelope>,
}

/// 追踪方向。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceDirection {
    /// 调用方发出（tell 或 ask 请求）。
    Out,
    /// 回复（按 corr 销账）。
    Reply,
}

/// 追踪条目——"一问一答全程留痕"（D6）。
#[derive(Clone, Debug)]
pub struct TraceEntry {
    pub direction: TraceDirection,
    pub kind: &'static str,
    pub corr: Option<Corr>,
    pub from: Address,
    pub to: Route,
}

/// ask 收口：future 门（oneshot）或宿主 mailbox（GUI poll 门，D6/D9）。
/// Mailbox 收口的错误可见性（超时）在 dispatch 定时器内以
/// `KIND_ROUTE_ERROR` 信封送达。
pub(crate) enum Completion {
    Oneshot(oneshot::Sender<Result<Payload, RouteError>>),
    Mailbox(mpsc::Sender<Envelope>),
}

struct Pending {
    completion: Completion,
    /// ask 发起方（回复信封的 `to` 重写目标）。
    submitter: Address,
    /// 原始请求（超时落死信的可观测记录）。
    request: Envelope,
}

pub(crate) struct RouterInner {
    instances: Arc<Mutex<Instances>>,
    pending: Mutex<HashMap<Corr, Pending>>,
    next_corr: AtomicU64,
    dead_letters: Mutex<Vec<DeadLetterRecord>>,
    trace: Mutex<VecDeque<TraceEntry>>,
    /// 订阅塌缩成路由（D7/D9）：目标实例 → 订阅者参与者 id 列表。
    subscriptions: Mutex<HashMap<InstanceId, Vec<InstanceId>>>,
    handle: tokio::runtime::Handle,
    default_timeout: Duration,
}

/// 中心路由（`ServiceManager` 持有并克隆分发；内部 `Arc`）。
#[derive(Clone)]
pub struct Router {
    inner: Arc<RouterInner>,
}

impl Router {
    pub(crate) fn new(
        instances: Arc<Mutex<Instances>>,
        handle: tokio::runtime::Handle,
        default_timeout: Duration,
    ) -> Self {
        Router {
            inner: Arc::new(RouterInner {
                instances,
                pending: Mutex::new(HashMap::new()),
                next_corr: AtomicU64::new(1),
                dead_letters: Mutex::new(Vec::new()),
                trace: Mutex::new(VecDeque::new()),
                subscriptions: Mutex::new(HashMap::new()),
                handle,
                default_timeout,
            }),
        }
    }

    /// Router 自身地址（框架自发信封的 `from`）。
    pub fn self_address(&self) -> Address {
        Address::instance(ROUTER_ID)
    }

    pub fn default_timeout(&self) -> Duration {
        self.inner.default_timeout
    }

    fn next_corr(&self) -> Corr {
        Corr(self.inner.next_corr.fetch_add(1, Ordering::Relaxed))
    }

    // ---------- 观测面 ----------

    fn trace(&self, direction: TraceDirection, env: &Envelope) {
        let mut tr = self.inner.trace.lock().expect("trace poisoned");
        if tr.len() == TRACE_CAP {
            tr.pop_front();
        }
        tr.push_back(TraceEntry {
            direction,
            kind: env.kind,
            corr: env.corr,
            from: env.from.clone(),
            to: env.to.clone(),
        });
    }

    pub fn trace_entries(&self) -> Vec<TraceEntry> {
        self.inner.trace.lock().expect("trace poisoned").iter().cloned().collect()
    }

    pub fn dead_letters(&self) -> Vec<DeadLetterRecord> {
        self.inner.dead_letters.lock().expect("dead poisoned").iter().cloned().collect()
    }

    pub fn pending_count(&self) -> usize {
        self.inner.pending.lock().expect("pending poisoned").len()
    }

    fn record_dead(&self, env: &Envelope, reason: DeadLetterReason) {
        let mut dl = self.inner.dead_letters.lock().expect("dead poisoned");
        if dl.len() == DEAD_LETTER_CAP {
            dl.remove(0);
        }
        dl.push(DeadLetterRecord {
            reason,
            kind: env.kind,
            corr: env.corr,
            to: env.to.clone(),
            envelope: Some(env.clone()),
        });
    }

    // ---------- 寻址与投递 ----------

    /// 解析目标参与者（同一把实例表锁内完成投递，避免检查-使用竞态）。
    fn deliver(&self, env: Envelope) -> Result<(), (RouteError, Envelope)> {
        let table = self.inner.instances.lock().expect("instances poisoned");
        let id = match &env.to {
            Route::Instance(id) => {
                if table.get(*id).is_none() {
                    return Err((RouteError::NoSuchRoute, env));
                }
                *id
            }
            Route::Service(name, SelectPolicy::Direct(id)) => {
                // 断言: 实例必须属于该服务 (D4 防误投, spec R2)
                if !table.is_instance_of(*id, name) {
                    return Err((RouteError::NoSuchRoute, env));
                }
                *id
            }
            Route::Service(name, SelectPolicy::Any) => match table.pick_available(name) {
                Some(id) => id,
                None => {
                    // 有实例但全不可用 → TargetNotAvailable; 无实例 → NoSuchRoute
                    let existing = table.instances_of(name).into_iter().next().and_then(|id| table.get(id).map(|p| p.state));
                    match existing {
                        Some(state) => return Err((RouteError::TargetNotAvailable(state), env)),
                        None => return Err((RouteError::NoSuchRoute, env)),
                    }
                }
            },
        };
        let p = table.get(id).expect("resolved");
        if p.state != ServiceState::Available {
            // fail-fast: 目标非 available 立即拒绝 (D6)
            return Err((RouteError::TargetNotAvailable(p.state), env));
        }
        match p.outlet.try_send(env) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(env)) => {
                Err((RouteError::DeadLetter(DeadLetterReason::ChannelFull), env))
            }
            Err(mpsc::error::TrySendError::Closed(env)) => {
                Err((RouteError::DeadLetter(DeadLetterReason::NoSuchRoute), env))
            }
        }
    }

    fn reason_of(err: &RouteError) -> DeadLetterReason {
        match err {
            RouteError::Timeout => DeadLetterReason::Other("ask timeout"),
            RouteError::TargetNotAvailable(s) => DeadLetterReason::TargetNotAvailable(*s),
            RouteError::DeadLetter(r) => r.clone(),
            RouteError::NoSuchRoute => DeadLetterReason::NoSuchRoute,
            RouteError::TypeMismatch { .. } => DeadLetterReason::TypeMismatch,
            RouteError::AmbiguousAddress => DeadLetterReason::Other("ambiguous address"),
        }
    }

    /// 统一发送入口：
    /// - `corr = None` → tell 投递（失败落死信 + `Err`——tell 无同步反馈，
    ///   **GUI 用户操作一律走 ask**，D6）；
    /// - `corr = Some(..)` → 回复：**按 corr 在 pending 表销账**，不按 to/from
    ///   寻址（D5/D6）。
    pub fn send(&self, env: Envelope) -> Result<(), RouteError> {
        if let Some(corr) = env.corr {
            return self.complete(corr, env);
        }
        self.trace(TraceDirection::Out, &env);
        match self.deliver(env) {
            Ok(()) => Ok(()),
            Err((err, env)) => {
                let reason = Self::reason_of(&err);
                self.record_dead(&env, reason);
                Err(err)
            }
        }
    }

    /// 回复销账（reply 的底层；失败同样落死信 + Err）。
    fn complete(&self, corr: Corr, mut env: Envelope) -> Result<(), RouteError> {
        self.trace(TraceDirection::Reply, &env);
        let pending = self.inner.pending.lock().expect("pending poisoned").remove(&corr);
        match pending {
            None => {
                // 晚到/未知回复: 死信可观测, 调用方收 NoSuchRoute
                let reason = DeadLetterReason::Other("late or unknown reply");
                self.record_dead(&env, reason);
                Err(RouteError::NoSuchRoute)
            }
            Some(p) => match p.completion {
                Completion::Oneshot(tx) => {
                    let _ = tx.send(Ok(env.payload));
                    Ok(())
                }
                Completion::Mailbox(tx) => {
                    // 信封按 corr 收口; `to` 重写为发起方 (字段仅观测, 匹配靠 corr)
                    env.to = p.submitter.route().clone();
                    match tx.try_send(env) {
                        Ok(()) => Ok(()),
                        Err(mpsc::error::TrySendError::Full(env)) => {
                            self.record_dead(&env, DeadLetterReason::ChannelFull);
                            Err(RouteError::DeadLetter(DeadLetterReason::ChannelFull))
                        }
                        Err(mpsc::error::TrySendError::Closed(env)) => {
                            self.record_dead(&env, DeadLetterReason::NoSuchRoute);
                            Err(RouteError::DeadLetter(DeadLetterReason::NoSuchRoute))
                        }
                    }
                }
            },
        }
    }

    // ---------- ask ----------

    /// future 门的 ask：**急切分发**（请求投递与超时定时器都在当下完成，
    /// 不等 poll），返回的 future 只剩 oneshot await——executor-agnostic
    /// （gpui executor / tokio / 测试 block_on 均可 await，D9）。
    pub(crate) fn call_future(
        &self,
        env: Envelope,
        submitter: Address,
        timeout: Duration,
    ) -> impl Future<Output = Result<Payload, RouteError>> + Send + 'static {
        let (tx, rx) = oneshot::channel();
        let dispatched = self.dispatch(env, submitter, Completion::Oneshot(tx), timeout);
        async move {
            match dispatched {
                Ok(_corr) => rx.await.unwrap_or_else(|_| {
                    Err(RouteError::DeadLetter(service_core::DeadLetterReason::Other(
                        "router dropped",
                    )))
                }),
                Err(e) => Err(e),
            }
        }
    }

    /// ask 分发（eager：请求投递与超时定时器都在**调用当下**完成，不等 poll）。
    /// 返回 corr；future 门用 oneshot 收口，宿主门用 mailbox 收口。
    pub(crate) fn dispatch(
        &self,
        mut env: Envelope,
        submitter: Address,
        completion: Completion,
        timeout: Duration,
    ) -> Result<Corr, RouteError> {
        let corr = self.next_corr();
        env.corr = Some(corr);
        self.trace(TraceDirection::Out, &env);
        self.inner.pending.lock().expect("pending poisoned").insert(
            corr,
            Pending {
                completion,
                submitter,
                request: env.clone(),
            },
        );
        if let Err((err, env)) = self.deliver(env) {
            self.inner.pending.lock().expect("pending poisoned").remove(&corr);
            self.record_dead(&env, Self::reason_of(&err));
            return Err(err);
        }
        // 急切 spawn 超时定时器 (Router 持 deadline, D6)
        let inner = Arc::clone(&self.inner);
        let handle = inner.handle.clone();
        handle.spawn(async move {
            tokio::time::sleep(timeout).await;
            let pending = inner.pending.lock().expect("pending poisoned").remove(&corr);
            if let Some(p) = pending {
                // 到期销账、落死信 (D6: 超时是可观测事件)
                let mut dl = inner.dead_letters.lock().expect("dead poisoned");
                if dl.len() == DEAD_LETTER_CAP {
                    dl.remove(0);
                }
                dl.push(DeadLetterRecord {
                    reason: DeadLetterReason::Other("ask timeout"),
                    kind: p.request.kind,
                    corr: Some(corr),
                    to: p.request.to.clone(),
                    envelope: Some(p.request.clone()),
                });
                drop(dl);
                match p.completion {
                    Completion::Oneshot(tx) => {
                        let _ = tx.send(Err(RouteError::Timeout));
                    }
                    Completion::Mailbox(tx) => {
                        // 宿主门也要看得见超时: 以 route_error 信封送达 (带 corr 供配对)
                        let env = Envelope::new(
                            p.submitter.route().clone(),
                            Address::instance(ROUTER_ID),
                            KIND_ROUTE_ERROR,
                            Payload::new(RouteError::Timeout),
                        )
                        .with_corr(corr);
                        let _ = tx.try_send(env);
                    }
                }
            }
        });
        Ok(corr)
    }

    // ---------- 订阅 (订阅塌缩成路由, D7/D9) ----------

    /// 订阅目标实例的状态翻转：加一条"翻转事件投到 subscriber 地址"的路由规则。
    pub fn subscribe_state(&self, target: InstanceId, subscriber: InstanceId) {
        let mut subs = self.inner.subscriptions.lock().expect("subs poisoned");
        let list = subs.entry(target).or_default();
        if !list.contains(&subscriber) {
            list.push(subscriber);
        }
    }

    /// 状态翻转事件转发（由 supervisor 事件流的转发循环调用）。
    pub(crate) fn publish_state_change(&self, change: StateChange) {
        let subs = {
            let subs = self.inner.subscriptions.lock().expect("subs poisoned");
            subs.get(&change.instance).cloned().unwrap_or_default()
        };
        for subscriber in subs {
            let env = Envelope::new(
                Route::Instance(subscriber),
                self.self_address(),
                KIND_STATE_CHANGE,
                Payload::new(change.clone()),
            );
            // tell 语义: 失败 (订阅者队列满/消失) 落死信, 观测面可见
            let _ = self.send(env);
        }
    }
}
