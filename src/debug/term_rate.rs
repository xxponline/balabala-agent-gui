//! debug-console change task 7.1: term-rate section (design D2/D7/D10)。
//!
//! Entity 按秒读 [`Report`] 槽 (agent-service [`latest_report`] — 只读
//! clone, spec: *reads without consuming*), 渲染与 stderr 周期报告
//! **同源同值**的全部 13 项指标: 回显 (avg/max/n/miss) / 帧→画
//! (avg/max/n) / feed·render 速率 / 帧 KB (avg/max) / since_feed /
//! render_cost avg。
//!
//! **采样仅签可见 (design D10)**: 每秒醒一拍先过
//! [`TerminalApp::is_section_visible`] — 关签 (dedup 摘除) 或切走
//! (激活别的签) → 不读不刷; 未打开 = 无实体 = 无任务 (硬约束: 不打开
//! 零开销)。1s 任务的 [`Task`] 持在实体字段: 实体亡 → Task 亡 → 任务
//! 取消 (关闭即停); 实体仍在 (切走未关) 时门控空转 (µs 级检查) —
//! 两种情形读取均停 (7.1 验收: 关闭页签后无采样)。
//!
//! cfg(test) 注入缝: [`OVERRIDE`] 覆盖真实槽 (真实槽每秒被 `maybe_print`
//! 并发覆写, 测试直注必被竞态抹掉 — 注入优先), [`READS`] 读取计数
//! 探针 (开着时涨 / 关闭后冻结)。

use std::time::Duration;

#[cfg(test)]
use std::sync::Mutex;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

use agent_service::terminal::{Report, latest_report};
use gpui_kit::base::TestTextExt;
use gpui_kit::{
    AnyView, AppContext, AsyncApp, Context, IntoElement, ParentElement, Render, Styled, Task,
    WeakEntity, Window, div,
};

use crate::app::TerminalApp;

/// 读取计数探针 (7.1 验收: 开着时涨 / 关闭后冻结) — cfg(test) 缝,
/// 进程级静态 (测试文件用 TEST_LOCK 串行后取基线差值)。
#[cfg(test)]
static READS: AtomicU64 = AtomicU64::new(0);

/// 注入已知 [`Report`] 的缝 (cfg(test)): 注入优先于真实槽 — 槽被
/// `maybe_print` 每秒并发覆写, 测试直接写槽会被竞态抹掉。
#[cfg(test)]
static OVERRIDE: Mutex<Option<Report>> = Mutex::new(None);

/// 读槽 (测试构建: 计数探针 + 注入优先)。
#[cfg(test)]
fn read_report() -> Option<Report> {
    READS.fetch_add(1, Ordering::Relaxed);
    if let Some(injected) = OVERRIDE.lock().unwrap_or_else(|p| p.into_inner()).clone() {
        return Some(injected);
    }
    latest_report()
}

/// 读槽 (生产: 直读槽 — 只读 clone, 不消费 rate_log 计数器)。
#[cfg(not(test))]
fn read_report() -> Option<Report> {
    latest_report()
}

/// 五行行文 = 13 项指标 — 渲染与测试断言同源 (字段值内嵌行文, 断言
/// 行文即断字段; 测试另按注入值独立再断一遍, 行文漏字段必败)。
fn rows(r: &Report) -> [String; 5] {
    [
        format!(
            "回显 avg={}ms max={}ms (n={} miss={})",
            r.echo_avg_ms, r.echo_max_ms, r.echo_n, r.echo_miss
        ),
        format!(
            "帧→画 avg={}ms max={}ms (n={})",
            r.paint_avg_ms, r.paint_max_ms, r.paint_n
        ),
        format!("feed/s={} render/s={}", r.feed_per_s, r.render_per_s),
        format!("帧KB avg={:.1} max={:.1}", r.frame_kb_avg, r.frame_kb_max),
        format!(
            "since_feed={}ms render_cost avg={:.1}ms",
            r.since_feed_ms, r.render_cost_avg_ms
        ),
    ]
}

/// term-rate section 渲染体 (design D2: section = Entity)。
pub struct TermRateSection {
    /// section id — D10 可见性门控的 dedup 键 (开签登记 / 关签摘除)。
    id: &'static str,
    /// 所属工作台 (门控查 dedup 表 + 中心激活签)。
    app: WeakEntity<TerminalApp>,
    /// 最近一拍采样 (None = 尚无汇总 — 还没喂过帧)。
    current: Option<Report>,
    /// 1s 采样任务 — 持即活; 实体亡 → Task 亡 → 取消 (关闭即停)。
    // never-read: 存活即语义 (parent 模块 allow(dead_code))
    task: Task<()>,
}

/// 注册表工厂 (task 8.1 消费): 建 Entity 并起 1s 采样任务, 类型擦除。
pub fn factory(id: &'static str, cx: &mut Context<TerminalApp>) -> AnyView {
    let app = cx.weak_entity();
    cx.new(|cx| TermRateSection::new(id, app, cx)).into()
}

impl TermRateSection {
    fn new(id: &'static str, app: WeakEntity<TerminalApp>, cx: &mut Context<Self>) -> Self {
        // spawn 闭包首参即 WeakEntity<Self> (Context::spawn 契约, app.rs
        // 同款 `|this, cx: &mut AsyncApp|`) — 构造闭包内实体值未落位,
        // 不调 cx.entity() (内部 upgrade 会 panic)。
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
            current: read_report(),
            task,
        }
    }

    /// 1s 一拍: 可见性门控 (D10) → 读槽 → 刷帧。
    fn tick(&mut self, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return; // 工作台亡 (窗口拆除) — 止
        };
        if !app.read(cx).is_section_visible(cx, self.id) {
            return; // 关签或切走 — 不读不刷 (7.1: 关闭后无采样)
        }
        let Some(report) = read_report() else {
            return; // 尚无汇总 (还没喂过帧)
        };
        self.current = Some(report);
        cx.notify();
    }
}

impl Render for TermRateSection {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let Some(r) = self.current.as_ref() else {
            return div().child("等待首份汇总…").into_any_element();
        };
        // test_text 观测口 (pane/tree 同款): texts() 只收录 test_text —
        // 无头测试按行断言字段; 无 test-support 构建下是空操作穿透。
        div().flex()
            .flex_col()
            .children(
                rows(r)
                    .into_iter()
                    .map(|row| div().child(row.clone()).test_text(row)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "term_rate_tests.rs"]
mod tests;
