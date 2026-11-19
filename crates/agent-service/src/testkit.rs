//! crate 内单测替身（`service-core` `TestMessaging` 的本地等价物）：
//! 出站信封全量捕获，供控制/帧/投影类断言使用。
//!
//! task 1.2 先行建置，多数辅助函数由 task 2.x 的测试首用——
//! 此处整体 `allow(dead_code)`，非死码而是未到用期。

#![allow(dead_code)] // 见上: group 2 测试消费的预置件

use service_core::{
    Address, CallFuture, Envelope, InstanceId, Messaging, Payload, RouteError, TaskFuture,
};
use std::sync::Mutex;

/// 捕获型出站能力：`tell`/`call`/`reply`/`reject` 全部落 `sent`。
pub(crate) struct TestMessaging {
    pub id: InstanceId,
    sent: Mutex<Vec<Envelope>>,
}

impl TestMessaging {
    pub fn new(id: InstanceId) -> Self {
        Self {
            id,
            sent: Mutex::new(Vec::new()),
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
        self.sent.lock().expect("test sent poisoned").push(env.clone());
        Ok(())
    }

    fn spawn_task(&self, _fut: TaskFuture) {
        // 1.2 骨架: 无 runtime。task 2.6 接 drain 循环时按需扩展。
    }
}

/// 无 runtime 的 RuntimeToken 替身（裸指针契约见 core 文档；测试内不消费）。
pub(crate) struct NoToken;

impl service_core::RuntimeToken for NoToken {
    fn handle_ptr(&self) -> *const () {
        std::ptr::null()
    }
}

/// 构造测试用 `ServiceCtx`（出站捕获能力由调用方自持 `Arc<TestMessaging>` 以便断言）。
pub(crate) fn test_ctx_with(msg: std::sync::Arc<TestMessaging>) -> service_core::ServiceCtx {
    service_core::ServiceCtx {
        instance: msg.id,
        service: "agent".into(),
        messaging: msg,
        runtime_token: std::sync::Arc::new(NoToken),
    }
}

/// 无需断言出站时的便捷构造。
pub(crate) fn test_ctx(id: InstanceId) -> service_core::ServiceCtx {
    test_ctx_with(std::sync::Arc::new(TestMessaging::new(id)))
}
