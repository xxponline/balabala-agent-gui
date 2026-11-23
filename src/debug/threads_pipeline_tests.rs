//! threads/pipeline section 无头契约 (task 7.2): 注入已知五项 → 断言
//! 渲染; `threads: None` → unavailable; **不可见即不采样** (OS 查询
//! 调用计数探针: 可见时涨 / 切走冻结 / 关闭冻结)。
//!
//! 显式导入, 不用 glob — 防 `gpui_kit::*` 把 `#[test]` 解析到
//! `gpui_kit::test` (panel_tests 同款教训)。helper 均为 panel_tests
//! 同款本地副本 (codebase 惯例: 样板本地复制不抽公共)。
//!
//! **单测试多阶段** (不拆多个 `#[gpui_kit::test]`): 每个测试 = 一个完整
//! app 启动 = 一份 service 栈/tokio runtime — 高负载并行下多起 app 会
//! 放大既有 pdm 时序 flake (HEAD c4b1365 同类), 合并控制负载。
//!
//! [`super::OVERRIDE`] 是进程级注入缝 → 用 `TEST_LOCK` 串行, 每阶段先
//! 复位再按需注入; 探针 `os_query_calls` 是线程本地 (测试同步跑在本
//! 线程), 只数本测试自己的 OS 查询。

use std::sync::Mutex;
use std::time::Duration;

use gpui_kit::base::dock::PanelId;
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
use crate::debug::thread_count::os_query_calls;

use super::{OVERRIDE, Snapshot, factory, rows};

/// 注入缝互斥 (DROP_LOCK 同款): 串行化本文件测试, 每阶段先复位注入。
static TEST_LOCK: Mutex<()> = Mutex::new(());

// ---------- cfg(test) 注入缝 ----------

/// 已知五项 — 值互不为子串 (42/17/13/19/23), 断"值在场"即断字段渲染。
fn known_snapshot() -> Snapshot {
    Snapshot {
        threads: Some(42),
        dropped_frames: 17,
        pending: 13,
        listener_backlog: 19,
        sessions: 23,
    }
}

fn inject(snapshot: Snapshot) {
    *OVERRIDE.lock().unwrap_or_else(|p| p.into_inner()) = Some(snapshot);
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

/// 关闭 section 签 (on_removed 回摘 dedup 时 TerminalApp 无 lease —
/// 不得包在 app.update 内, pdm close_viewer 同款)。
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

/// 切到 console 签 (激活别的签 — D10 的 inactive 分支)。
fn select_console(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    app: &Entity<TerminalApp>,
) {
    cx.update_window(handle, |_, window, cx| {
        let console = PanelId::from(app.read(cx).terminal.entity_id());
        let area = app.read(cx).dock_area.clone();
        area.update(cx, |area, cx| area.select_panel(console, window, cx));
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

// ---------- 7.2 验收 (单测试多阶段, 见文件头) ----------

/// 阶段 A: 注入五项 → 断言渲染; 阶段 B: `threads: None` → unavailable;
/// 阶段 C: 清注入走真值 — 可见时 OS 查询探针涨, 切走冻结, 关闭再冻结
/// (spec: sampled only while visible)。
#[gpui_kit::test]
fn renders_metrics_unavailable_and_invisible_stops_sampling(cx: &mut TestAppContext) {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    clear_inject();
    let (handle, app) = open_app(cx);
    register(DebugSection {
        id: "test::threads/all",
        title: "threads (all)",
        factory,
    });

    // ---- 阶段 A: 注入五项 → 开签 → 推进首拍 (构造拍不可行, 见 factory
    // 注) → 断言行文 + 五值 ----
    let known = known_snapshot();
    inject(known.clone());
    open_section_in(cx, handle, &app, "test::threads/all");
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();

    let texts = render_texts(handle, cx);
    for row in rows(&known) {
        assert!(
            texts.contains(&row),
            "字段行须渲染: {row}\n实际 texts={texts:?}"
        );
    }
    let joined = texts.join("\n");
    let values = [
        known.threads.unwrap().to_string(),
        known.dropped_frames.to_string(),
        known.pending.to_string(),
        known.listener_backlog.to_string(),
        known.sessions.to_string(),
    ];
    for value in values {
        assert!(joined.contains(&value), "字段值须渲染: {value}");
    }

    // ---- 阶段 B: 注入 threads: None → 下一拍 → 行文 unavailable ----
    let mut unavailable = known_snapshot();
    unavailable.threads = None;
    inject(unavailable.clone());
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    let texts = render_texts(handle, cx);
    let marker = rows(&unavailable)[0].clone();
    assert_eq!(marker, "OS线程数=unavailable");
    assert!(
        texts.contains(&marker),
        "None 须显示 unavailable 行\n实际 texts={texts:?}"
    );

    // ---- 阶段 C: 清注入走真值 → 可见采样 / 切走冻结 / 关闭冻结 ----
    clear_inject();
    let base = os_query_calls();
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    let visible = os_query_calls();
    assert!(visible > base, "签可见时按秒采 OS 线程数: {base} → {visible}");

    select_console(cx, handle, &app); // inactive 分支 (实体仍活)
    let inactive_base = os_query_calls();
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(
        os_query_calls(),
        inactive_base,
        "切到别的签即不采样 (inactive)"
    );

    let panel = section_entity(cx, &app, "test::threads/all");
    close_section(cx, handle, &app, panel);
    let closed_base = os_query_calls();
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(os_query_calls(), closed_base, "关闭页签即不采样 (closed)");
}
