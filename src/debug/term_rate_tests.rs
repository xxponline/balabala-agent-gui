//! term-rate section 无头契约 (task 7.1): 注入已知 `Report` → 13 项
//! 指标全渲染; 可见时按秒采样; 关闭页签 → 读取停止 (D10 可见性门控 +
//! 实体亡停任务)。
//!
//! 显式导入, 不用 glob — 防 `gpui_kit::*` 把 `#[test]` 解析到
//! `gpui_kit::test` (panel_tests 同款教训)。helper 均为 panel_tests
//! 同款本地副本 (codebase 惯例: 样板本地复制不抽公共)。
//!
//! **单测试多阶段** (不拆多个 `#[gpui_kit::test]`): 每个测试 = 一个完整
//! app 启动 = 一份 service 栈/tokio runtime — 高负载并行下多起 app 会
//! 放大既有 pdm 时序 flake (HEAD c4b1365 同类), 合并控制负载。
//!
//! [`super::READS`] 是**进程级**读取探针: 用 `TEST_LOCK` 串行
//! (agent-service `DROP_LOCK` 同款), 取基线差值, 不假设全局绝对值。

use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;

use agent_service::terminal::Report;
use gpui_kit::component::dock::PanelHandle;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions,
    px, size,
};

use crate::app::TerminalApp;
use crate::debug::DebugSection;
use crate::debug::panel::DebugSectionPanel;
use crate::debug::register;

use super::{OVERRIDE, READS, factory, rows};

/// 读取探针互斥 (DROP_LOCK 同款): 串行化本文件测试, 各取基线差值。
static TEST_LOCK: Mutex<()> = Mutex::new(());

// ---------- cfg(test) 注入缝 ----------

/// 已知 Report — 字段值互不为子串 (u64: 11..20; f64: 7.5/8.5/9.5),
/// 断"值在场"即断该字段渲染, 行文漏字段必败。
fn known_report() -> Report {
    Report {
        echo_avg_ms: 11,
        echo_max_ms: 12,
        echo_n: 13,
        echo_miss: 14,
        paint_avg_ms: 15,
        paint_max_ms: 16,
        paint_n: 17,
        feed_per_s: 18,
        render_per_s: 19,
        frame_kb_avg: 7.5,
        frame_kb_max: 8.5,
        since_feed_ms: 20,
        render_cost_avg_ms: 9.5,
    }
}

/// 注入 (注入优先于真实槽 — 避开 maybe_print 每秒覆写竞态)。
fn inject(report: Report) {
    *OVERRIDE.lock().unwrap_or_else(|p| p.into_inner()) = Some(report);
}

fn clear_inject() {
    *OVERRIDE.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

// ---------- helper (panel_tests 同款副本) ----------

fn open_app(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<TerminalApp>) {
    cx.update(gpui_kit::init);
    let (handle, app) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1000.), px(600.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| TerminalApp::new(window, cx).expect("app 启动")),
        )
        .expect("open test window")
    });
    app.update(cx, |app, _| {
        app.workspace = None;
        app.active = None;
    });
    (handle, app)
}

/// 走平行入口开一个 section (真实调用链: update_window → app.update)。
fn open_section_in(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    app: &Entity<TerminalApp>,
    id: &str,
) {
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| app.open_debug_section(id, window, cx));
    })
    .expect("update window");
    cx.run_until_parked();
}

/// section 面板实体 (dedup 表 → dock → 两跳 downcast)。
fn section_entity(
    cx: &mut TestAppContext,
    app: &Entity<TerminalApp>,
    id: &str,
) -> Entity<DebugSectionPanel> {
    cx.update(|cx| {
        let app = app.read(cx);
        let panel_id = *app.debug_tabs.get(id).expect("dedup 表登记 section");
        let area = app.dock_area.read(cx);
        let handle = area.panel(panel_id).expect("section 在 dock 中");
        let wrapped = PanelHandle::of(handle).expect("panel_handle 包装");
        wrapped
            .panel()
            .as_any()
            .downcast_ref::<Entity<DebugSectionPanel>>()
            .expect("inner 是 Entity<DebugSectionPanel>")
            .clone()
    })
}

/// 关闭 section 签 (库公开 API; on_removed 回摘 dedup 时 TerminalApp
/// 无 lease — 不得包在 app.update 内, pdm close_viewer 同款)。
fn close_section(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    app: &Entity<TerminalApp>,
    section: Entity<DebugSectionPanel>,
) {
    cx.update_window(handle, |_, window, cx| {
        let area = app.read(cx).dock_area.clone();
        area.update(cx, |area, cx| area.remove_panel(section, window, cx));
    })
    .expect("update window");
    cx.run_until_parked();
}

/// 渲染一帧并取全部文本 (app.rs render_texts 同款)。
fn render_texts(handle: AnyWindowHandle, cx: &mut TestAppContext) -> Vec<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        gpui_kit::base::test_support::texts(window)
            .iter()
            .map(|t| t.text().to_string())
            .collect()
    })
    .expect("update window")
}

// ---------- 7.1 验收 (单测试多阶段, 见文件头) ----------

/// 阶段 A: 注入已知 Report → 断言 13 项字段全渲染;
/// 阶段 B: 签可见时按秒采样 (读取计数涨);
/// 阶段 C: 关闭页签 → 读取计数冻结 (关闭后无采样)。
#[gpui_kit::test]
fn renders_fields_and_close_stops_sampling(cx: &mut TestAppContext) {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let known = known_report();
    inject(known.clone());
    let (handle, app) = open_app(cx);
    register(DebugSection {
        id: "test::term-rate/all",
        title: "term-rate (all)",
        factory,
    });
    open_section_in(cx, handle, &app, "test::term-rate/all");

    // ---- 阶段 A: 字段渲染 (构造拍即有值 → 无需推进时钟) ----
    let texts = render_texts(handle, cx);
    for row in rows(&known) {
        assert!(
            texts.contains(&row),
            "字段行须渲染: {row}\n实际 texts={texts:?}"
        );
    }
    // 13 项字段值按注入值独立再断一遍 (rows 若漏字段 → 值不在场 → 败)。
    let joined = texts.join("\n");
    let values = [
        known.echo_avg_ms.to_string(),
        known.echo_max_ms.to_string(),
        known.echo_n.to_string(),
        known.echo_miss.to_string(),
        known.paint_avg_ms.to_string(),
        known.paint_max_ms.to_string(),
        known.paint_n.to_string(),
        known.feed_per_s.to_string(),
        known.render_per_s.to_string(),
        known.since_feed_ms.to_string(),
        format!("{:.1}", known.frame_kb_avg),
        format!("{:.1}", known.frame_kb_max),
        format!("{:.1}", known.render_cost_avg_ms),
    ];
    for value in values {
        assert!(joined.contains(&value), "字段值须渲染: {value}");
    }

    // ---- 阶段 B: 签可见 → 推进 2s (>1s 周期) 至少一拍采样 ----
    // (基线取在开签之后 — 构造读不计入)
    let base = READS.load(Ordering::Relaxed);
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    let opened = READS.load(Ordering::Relaxed);
    assert!(opened > base, "签可见时按秒采样: {base} → {opened}");

    // ---- 阶段 C: 关闭 → 计数冻结 ----
    let panel = section_entity(cx, &app, "test::term-rate/all");
    close_section(cx, handle, &app, panel);
    let closed = READS.load(Ordering::Relaxed);
    // 再推 3 拍: 实体亡则任务亡 (无表可读), 实体尚在则门控空转
    // (dedup 已摘) — 两条路读取都停。
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    let after = READS.load(Ordering::Relaxed);
    assert_eq!(after, closed, "关闭页签后无采样 (读取停止)");
    clear_inject();
}
