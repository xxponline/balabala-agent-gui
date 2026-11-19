//! crate 内单测替身（`service-core` `TestMessaging` 的本地等价物）：
//! 出站信封全量捕获，供控制/帧/投影类断言使用；另含会话桩
//! [`StubSession`] + [`stub_factory`]（task 2.1 起各测试共用）。
//!
//! 多数辅助函数由后续 task 的测试首用——此处整体 `allow(dead_code)`，
//! 非死码而是未到用期。

#![allow(dead_code)] // 见上: 后续 task 测试消费的预置件

use crate::session::{SessionFactory, SessionIo};
use pi_shell_rs::app::theme::TuiTheme;
use std::any::Any;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use service_core::{
    Address, CallFuture, DeadLetterReason, Envelope, InstanceId, Messaging, Payload, RouteError,
    TaskFuture,
};

/// 捕获型出站能力：`tell`/`call`/`reply`/`reject` 全部落 `sent`；
/// `fail_tells` 打开后 `tell` 模拟出口满（ChannelFull）失败。
pub(crate) struct TestMessaging {
    pub id: InstanceId,
    sent: Mutex<Vec<Envelope>>,
    fail_tells: AtomicBool,
    runtime: Mutex<Option<tokio::runtime::Handle>>,
}

impl TestMessaging {
    pub fn new(id: InstanceId) -> Self {
        Self {
            id,
            sent: Mutex::new(Vec::new()),
            fail_tells: AtomicBool::new(false),
            runtime: Mutex::new(None),
        }
    }

    /// 模拟 mailbox 满: 后续 `tell` 返回 Err (task 2.4 丢帧计数断言)。
    pub fn set_fail_tells(&self, fail: bool) {
        self.fail_tells.store(fail, Ordering::SeqCst);
    }

    /// 注入宿主 runtime 句柄（`spawn_task` 起才真实驱动后台 future）。
    pub fn set_runtime(&self, handle: tokio::runtime::Handle) {
        *self.runtime.lock().expect("test runtime") = Some(handle);
    }

    /// 单线程测试 runtime（`block_on` 期间驱动已 spawn 的循环任务）。
    pub fn runtime_handle_of(rt: &tokio::runtime::Runtime) -> tokio::runtime::Handle {
        rt.handle().clone()
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
        if self.fail_tells.load(Ordering::SeqCst) {
            return Err(RouteError::DeadLetter(DeadLetterReason::ChannelFull));
        }
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

    fn spawn_task(&self, fut: TaskFuture) {
        let guard = self.runtime.lock().expect("test runtime");
        if let Some(handle) = guard.as_ref() {
            handle.spawn(fut); // 真实上 runtime (task 2.6 循环测试)
        }
        // 无 runtime: 丢弃 — 不驱动循环的测试不受影响
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

// ---------- 会话桩 (task 2.1) ----------

/// 桩会话的共享剧本：测试直改字段驱动行为、回读记录断言转发。
/// 默认**存活**（`alive: true`）——模拟正常 embed 线程；终态测试显式置 false。
pub(crate) struct StubScript {
    /// 下次 `drain` 返回并清空的字节。
    pub pending_bytes: Vec<u8>,
    /// 存活标志。
    pub alive: bool,
    pub keys: Vec<crossterm::event::KeyEvent>,
    pub texts: Vec<String>,
    pub wheels: Vec<i16>,
    pub clicks: Vec<(u16, u16)>,
    pub resized: Vec<(u16, u16)>,
    pub themes: Vec<TuiTheme>,
    /// 每次 create 工厂收到的注入主题（快照断言用）。
    pub create_themes: Vec<TuiTheme>,
    pub shutdowns: u32,
}

impl Default for StubScript {
    fn default() -> Self {
        Self {
            pending_bytes: Vec::new(),
            alive: true,
            keys: Vec::new(),
            texts: Vec::new(),
            wheels: Vec::new(),
            clicks: Vec::new(),
            resized: Vec::new(),
            themes: Vec::new(),
            create_themes: Vec::new(),
            shutdowns: 0,
        }
    }
}

/// 可脚本化会话桩（与测试共享 `Arc<Mutex<StubScript>>`）。
pub(crate) struct StubSession {
    script: Arc<Mutex<StubScript>>,
}

impl StubSession {
    pub fn new(script: Arc<Mutex<StubScript>>) -> Self {
        Self { script }
    }
}

impl SessionIo for StubSession {
    fn drain(&mut self) -> Vec<u8> {
        let mut s = self.script.lock().expect("stub script");
        std::mem::take(&mut s.pending_bytes)
    }

    fn alive(&self) -> bool {
        self.script.lock().expect("stub script").alive
    }

    fn send_key(&mut self, key: crossterm::event::KeyEvent) {
        self.script.lock().expect("stub script").keys.push(key);
    }

    fn send_text(&mut self, text: &str) {
        self.script.lock().expect("stub script").texts.push(text.into());
    }

    fn send_wheel(&mut self, lines: i16) {
        self.script.lock().expect("stub script").wheels.push(lines);
    }

    fn send_click(&mut self, x: u16, y: u16) {
        self.script.lock().expect("stub script").clicks.push((x, y));
    }

    fn resize(&mut self, cols: u16, rows: u16) -> bool {
        self.script.lock().expect("stub script").resized.push((cols, rows));
        true
    }

    fn set_theme(&mut self, theme: &TuiTheme) {
        self.script.lock().expect("stub script").themes.push(theme.clone());
    }

    fn shutdown(&mut self) {
        self.script.lock().expect("stub script").shutdowns += 1;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// 共享同一剧本的桩工厂（`create` 产出的桩与测试互见；记录注入主题）。
pub(crate) fn stub_factory(script: Arc<Mutex<StubScript>>) -> SessionFactory {
    Arc::new(move |_name, _root, theme| {
        script
            .lock()
            .expect("stub script")
            .create_themes
            .push(theme);
        Ok(Box::new(StubSession::new(Arc::clone(&script))) as Box<dyn SessionIo>)
    })
}
