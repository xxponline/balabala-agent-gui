//! crate 内单测替身（`solution-service::testkit` 同款形态）：出站信封全量
//! 捕获，供回执 / 视图事件 / reject 断言使用。

use service_core::{
    Address, CallFuture, Envelope, InstanceId, Messaging, Payload, Route, RouteError, RuntimeToken,
    ServiceCtx, TaskFuture,
};
use std::any::Any;
use std::sync::{Arc, Mutex};

/// 捕获型出站能力：`tell`/`call`/`reply`/`reject` 全部落 `sent`。
pub(crate) struct TestMessaging {
    id: InstanceId,
    sent: Mutex<Vec<Envelope>>,
    /// 被丢弃的后台任务计数 —— 证明「重活确实交出去了、handler 没内联跑」。
    dropped_tasks: Mutex<usize>,
}

impl TestMessaging {
    pub fn new(id: InstanceId) -> Self {
        Self {
            id,
            sent: Mutex::new(Vec::new()),
            dropped_tasks: Mutex::new(0),
        }
    }

    /// 已发出的信封快照（断言用）。
    pub fn sent(&self) -> Vec<Envelope> {
        self.sent.lock().expect("test sent poisoned").clone()
    }

    pub fn sent_kinds(&self) -> Vec<&'static str> {
        self.sent().iter().map(|e| e.kind).collect()
    }

    /// 丢给 `spawn_task` 的任务数（= 后台枚举起过几次）。
    pub fn dropped_tasks(&self) -> usize {
        *self.dropped_tasks.lock().expect("test dropped poisoned")
    }
}

impl Messaging for TestMessaging {
    fn instance_id(&self) -> InstanceId {
        self.id
    }

    fn self_address(&self) -> Address {
        Address::instance(self.id)
    }

    fn tell(&self, env: Envelope) -> Result<(), RouteError> {
        self.sent.lock().expect("test sent poisoned").push(env);
        Ok(())
    }

    fn call(&self, env: Envelope) -> CallFuture {
        self.sent.lock().expect("test sent poisoned").push(env);
        Box::pin(std::future::ready(Ok(Payload::new(()))))
    }

    fn reply(
        &self,
        req: &Envelope,
        kind: &'static str,
        payload: Payload,
    ) -> Result<(), RouteError> {
        let corr = req.corr.ok_or(RouteError::NoSuchRoute)?;
        self.sent.lock().expect("test sent poisoned").push(Envelope {
            to: req.from.route().clone(),
            from: Address::instance(self.id),
            corr: Some(corr),
            kind,
            payload,
        });
        Ok(())
    }

    fn reject(&self, env: &Envelope) -> Result<(), RouteError> {
        self.sent
            .lock()
            .expect("test sent poisoned")
            .push(env.clone());
        Ok(())
    }

    fn spawn_task(&self, _fut: TaskFuture) {
        // 状态机测试不驱动后台枚举（枚举结果由测试直接投 `asset.enum_done`）；
        // 计数以证明 handler 确实只做了交接。
        *self.dropped_tasks.lock().expect("test dropped poisoned") += 1;
    }
}

/// 空 runtime 载体（状态机测试不 spawn，无需真实句柄）。
struct NullRuntime;

impl RuntimeToken for NullRuntime {
    fn handle_ptr(&self) -> *const () {
        std::ptr::null()
    }
}

/// 测试用 `ServiceCtx` + 捕获能力。
pub(crate) fn test_ctx(id: u64) -> (ServiceCtx, Arc<TestMessaging>) {
    let msg = Arc::new(TestMessaging::new(InstanceId(id)));
    let ctx = ServiceCtx {
        instance: InstanceId(id),
        service: "asset".to_string().into(),
        messaging: msg.clone(),
        runtime_token: Arc::new(NullRuntime),
    };
    (ctx, msg)
}

/// 构造一条 ask 信封（带 corr；`from` = 远端实例）。
pub(crate) fn ask_env<T: Any + Send + Sync>(kind: &'static str, value: T) -> Envelope {
    Envelope::new(
        Route::Instance(InstanceId(99)),
        Address::instance(InstanceId(99)),
        kind,
        Payload::new(value),
    )
    .with_corr(service_core::Corr(1))
}

/// 构造一条 tell 信封（无 corr —— 事件/回投形态）。
pub(crate) fn tell_env<T: Any + Send + Sync>(kind: &'static str, value: T) -> Envelope {
    Envelope::new(
        Route::Instance(InstanceId(99)),
        Address::instance(InstanceId(99)),
        kind,
        Payload::new(value),
    )
}
