//! debug-console change task 7.2: threads/pipeline section
//! (design D2/D9/D10, specs/debug-console *Threads and pipeline section*)。
//!
//! Entity 按秒采样并渲染五项 (GUI 手里直读, D8 注记零通路=直接读):
//! 1. **OS 线程数** — [`process_thread_count`] OS 真值 (design D9),
//!    `None` → 行文 `unavailable` (spec: 不给编造数字);
//! 2. **`dropped_frames`** — agent-service 进程级丢帧累计 (task 3.1);
//! 3. **`manager.pending_count()`** — router 在途请求数;
//! 4. **GUI listener 积压** — mailbox 件数 (service-manager `Listener::len`,
//!    task 7.2 新增);
//! 5. **`sessions_view` 会话数**。
//!
//! **采样仅签可见 (design D10)**: 每秒醒一拍先过
//! [`TerminalApp::is_section_visible`] — 关签或切走 → 不采不刷 (spec:
//! *Values SHALL be sampled only while the section is visible*); 未打开
//! = 无实体 = 无任务。1s 任务的 [`Task`] 持在实体字段: 实体亡 → Task 亡
//! → 取消。
//!
//! cfg(test) 注入缝: [`OVERRIDE`] 覆盖五项真值 (并发测试在跑, OS 线程数
//! 等真值抖动 — 注入优先)。"不可见即不采样" 的断言走 **OS 查询调用
//! 计数探针** (见 [`crate::debug::thread_count::os_query_calls`], 线程
//! 本地, 只数本测试线程)。

use std::time::Duration;

#[cfg(test)]
use std::sync::Mutex;

use agent_service::service::dropped_frames;
use gpui_kit::base::TestTextExt;
use gpui_kit::{
    AnyView, AppContext, AsyncApp, Context, IntoElement, ParentElement, Render, Styled, Task,
    WeakEntity, Window, div,
};

use crate::app::TerminalApp;
use crate::debug::thread_count::process_thread_count;

/// 一拍采样 (五项 — specs/debug-console 数据口径)。
#[derive(Clone, Debug)]
struct Snapshot {
    /// OS 真值线程数 (None = 平台查询失败 → 行文 unavailable)。
    threads: Option<u32>,
    /// 进程级丢帧累计 (agent-service 只读暴露)。
    dropped_frames: u64,
    /// router 在途请求数。
    pending: usize,
    /// GUI mailbox 积压 (未 poll + 已 poll 未 drain)。
    listener_backlog: usize,
    /// 当前会话数 (sessions_view 镜像; 无快照 = 0)。
    sessions: usize,
}

/// 注入已知五项的缝 (cfg(test)): 注入优先于真值 — OS 线程数随并发
/// 测试漂, dropped_frames 会涨, 真值断不了渲染。
#[cfg(test)]
static OVERRIDE: Mutex<Option<Snapshot>> = Mutex::new(None);

#[cfg(test)]
fn injected() -> Option<Snapshot> {
    OVERRIDE.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

#[cfg(not(test))]
fn injected() -> Option<Snapshot> {
    None
}

/// 采一拍 (五项直读; cfg(test) 注入优先)。
fn take_snapshot(app: &TerminalApp) -> Snapshot {
    if let Some(snapshot) = injected() {
        return snapshot;
    }
    Snapshot {
        threads: process_thread_count(),
        dropped_frames: dropped_frames(),
        pending: app.service_layer.manager.pending_count(),
        listener_backlog: app.service_layer.listener.len(),
        sessions: app
            .service_layer
            .sessions_view()
            .map_or(0, |v| v.sessions.len()),
    }
}

/// 五行行文 — 渲染与测试断言同源 (值内嵌行文; 测试另按注入值独立再断)。
fn rows(s: &Snapshot) -> [String; 5] {
    [
        match s.threads {
            Some(n) => format!("OS线程数={n}"),
            None => "OS线程数=unavailable".to_string(),
        },
        format!("dropped_frames={}", s.dropped_frames),
        format!("pending={}", s.pending),
        format!("listener积压={}", s.listener_backlog),
        format!("sessions={}", s.sessions),
    ]
}

/// threads/pipeline section 渲染体 (design D2: section = Entity)。
pub struct ThreadsPipelineSection {
    /// section id — D10 可见性门控的 dedup 键 (开签登记 / 关签摘除)。
    id: &'static str,
    /// 所属工作台 (门控查 dedup 表 + 中心激活签)。
    app: WeakEntity<TerminalApp>,
    /// 最近一拍采样 (None = 尚未采过)。
    current: Option<Snapshot>,
    /// 1s 采样任务 — 持即活; 实体亡 → Task 亡 → 取消 (关闭即停)。
    // never-read: 存活即语义 (parent 模块 allow(dead_code))
    task: Task<()>,
}

/// 注册表工厂 (task 8.1 消费): 建 Entity 并起 1s 采样任务, 类型擦除。
///
/// **无构造拍**: 工厂跑在 `open_debug_section` 的 `&mut self` 更新里,
/// 此刻 TerminalApp 正在 update — 读即 panic (entity_map 借用守卫);
/// 首拍由 1s 任务完成 (开签后 ≤1s 出值, spec: 可见时 ≥1/s 刷新)。
pub fn factory(id: &'static str, cx: &mut Context<TerminalApp>) -> AnyView {
    let app = cx.weak_entity();
    cx.new(|cx| ThreadsPipelineSection::new(id, app, cx)).into()
}

impl ThreadsPipelineSection {
    fn new(
        id: &'static str,
        app: WeakEntity<TerminalApp>,
        cx: &mut Context<Self>,
    ) -> Self {
        // spawn 闭包首参即 WeakEntity<Self> (Context::spawn 契约, app.rs
        // 同款) — 构造闭包内实体值未落位, 不调 cx.entity() (upgrade panic)。
        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(1))
                    .await;
                let Some(section) = this.upgrade() else {
                    break; // 实体亡 (关签) — 任务止
                };
                section.update(cx, |this, cx| this.tick(cx));
            }
        });
        Self {
            id,
            app,
            current: None, // 首拍 1s 内到 (见 factory 注)
            task,
        }
    }

    /// 1s 一拍: 可见性门控 (D10) → 采五项 → 刷帧。
    fn tick(&mut self, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return; // 工作台亡 (窗口拆除) — 止
        };
        if !app.read(cx).is_section_visible(cx, self.id) {
            return; // 不可见即不采样 (spec: sampled only while visible)
        }
        let snapshot = take_snapshot(app.read(cx));
        self.current = Some(snapshot);
        cx.notify();
    }
}

impl Render for ThreadsPipelineSection {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(s) = self.current.as_ref() else {
            return div().child("等待采样…").into_any_element();
        };
        // test_text 观测口 (pane/tree 同款): texts() 只收录 test_text —
        // 无 test-support 构建下是空操作穿透。
        div().flex()
            .flex_col()
            .children(
                rows(s)
                    .into_iter()
                    .map(|row| div().child(row.clone()).test_text(row)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "threads_pipeline_tests.rs"]
mod tests;
