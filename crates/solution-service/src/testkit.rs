//! crate 内单测替身（agent-service `testkit` 同款形态，本地等价物）：
//! 出站信封全量捕获，供回执/树变更断言使用。

#![allow(dead_code)] // 后续 task 测试消费的预置件

use service_core::{
    Address, CallFuture, Envelope, InstanceId, Messaging, Payload, RouteError, RuntimeToken,
    ServiceCtx, TaskFuture,
};
use std::any::Any;
use std::sync::{Arc, Mutex};

/// 捕获型出站能力：`tell`/`call`/`reply`/`reject` 全部落 `sent`。
pub(crate) struct TestMessaging {
    id: InstanceId,
    sent: Mutex<Vec<Envelope>>,
    /// 捕获的后台任务 (3.2: 后台 git 任务由测试 drive)。
    spawned: Mutex<Vec<TaskFuture>>,
}

impl TestMessaging {
    pub fn new(id: InstanceId) -> Self {
        Self {
            id,
            sent: Mutex::new(Vec::new()),
            spawned: Mutex::new(Vec::new()),
        }
    }

    /// 已发出的信封快照（断言用）。
    pub fn sent(&self) -> Vec<Envelope> {
        self.sent.lock().expect("test sent poisoned").clone()
    }

    pub fn sent_kinds(&self) -> Vec<&'static str> {
        self.sent().iter().map(|e| e.kind).collect()
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

    fn reply(&self, req: &Envelope, kind: &'static str, payload: Payload) -> Result<(), RouteError> {
        let corr = req.corr.ok_or(RouteError::NoSuchRoute)?;
        let env = Envelope {
            to: req.from.route().clone(),
            from: Address::instance(self.id),
            corr: Some(corr),
            kind,
            payload,
        };
        self.sent.lock().expect("test sent poisoned").push(env);
        Ok(())
    }

    fn reject(&self, env: &Envelope) -> Result<(), RouteError> {
        self.sent
            .lock()
            .expect("test sent poisoned")
            .push(env.clone());
        Ok(())
    }

    fn spawn_task(&self, fut: TaskFuture) {
        // 捕获而非丢弃: change solution-git-worktree 3.2 的后台 git 任务
        // 由测试自行 drive (断言回执先于完成事件 / 无 corr / stderr 摘要)。
        self.spawned
            .lock()
            .expect("test spawned poisoned")
            .push(fut);
    }
}

impl TestMessaging {
    /// 驱动所有已捕获的后台任务至完成（测试用）。
    ///
    /// 未来体内无 await 点（同步 git 子进程）→ 单次 poll 即 Ready；
    /// 兼顾起见循环至 Ready, 上限防意外 Pending 死转。
    pub fn drive_spawned(&self) {
        let mut futs = std::mem::take(&mut *self.spawned.lock().expect("test spawned poisoned"));
        for mut fut in futs.drain(..) {
            let waker = std::task::Waker::noop();
            let mut cx = std::task::Context::from_waker(waker);
            for _ in 0..16 {
                match fut.as_mut().poll(&mut cx) {
                    std::task::Poll::Ready(()) => break,
                    std::task::Poll::Pending => continue,
                }
            }
        }
    }
}

/// 空 runtime 载体（SolutionService 不 spawn，无需真实句柄）。
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
        service: "solution".to_string().into(),
        messaging: msg.clone(),
        runtime_token: Arc::new(NullRuntime),
    };
    (ctx, msg)
}

/// 构造一条 ask 信封（带 corr；`from` = 远端实例）。
pub(crate) fn ask_env<T: Any + Send + Sync>(kind: &'static str, value: T) -> Envelope {
    Envelope::new(
        service_core::Route::Instance(InstanceId(99)),
        Address::instance(InstanceId(99)),
        kind,
        Payload::new(value),
    )
    .with_corr(service_core::Corr(1))
}
