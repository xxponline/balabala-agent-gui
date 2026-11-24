//! debug section 开签入口与签包装的无头契约 (task 5.1/5.2)。
//!
//! 场景 (specs/center-tabs delta): 尾插在 console 后 / 二次打开聚焦不
//! 重复 / 关闭后可重开 / 与文件路径身份互不干扰; 加 5.2 的签可关 +
//! console 恒第一签。
//!
//! 显式导入, 不用 glob — 防 `gpui_kit::*` 把属性宏内层 `#[test]`
//! 解析到 `gpui_kit::test` → 递归爆 (app.rs 同款教训)。

use std::path::PathBuf;

use gpui_kit::base::dock::PanelId;
use gpui_kit::component::dock::{BasePanel, DockPlacement, PanelHandle};
use gpui_kit::test::TestWindowExt;
use gpui_kit::base::TestTextExt;
use gpui_kit::{
    AnyView, AnyWindowHandle, AppContext, Bounds, Context, Entity, Focusable, IntoElement,
    ParentElement, Point, Render, TestAppContext, Window, WindowBounds, WindowOptions, div, px,
    size,
};

use crate::app::TerminalApp;
use asset_service::contract::AssetKind;
use crate::debug::panel::DebugSectionPanel;
use crate::debug::{DebugSection, register};
use crate::property_detail_manager::OpenObject;

// ---------- 测试替身 ----------

/// 测试用 section 视图: 固定文本 (渲染断言观测点)。
struct StubSection;

impl Render for StubSection {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // texts() 只收录 test_text (pane/tree 同款观测口)
        div().child("stub-section-body").test_text("stub-section-body")
    }
}

fn stub_factory(_: &'static str, cx: &mut Context<TerminalApp>) -> AnyView {
    cx.new(|_| StubSection).into()
}

fn register_stub(id: &'static str, title: &'static str) {
    register(DebugSection {
        id,
        title,
        factory: stub_factory,
    });
}

// ---------- 样板 (property_detail_manager tests 同款, 本地副本) ----------

/// 起完整 app (隔离全局历史/外部配置, pdm open_app 同款)。
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
            |window, cx| cx.new(|cx| TerminalApp::new(window, cx, None).expect("app 启动")),
        )
        .expect("open test window")
    });
    app.update(cx, |app, _| {
        app.workspace = None;
    });
    (handle, app)
}

/// 中心 tab 组 (panels 预序, active_ix) — 签序断言的观测口。
fn center_tabs(
    cx: &mut TestAppContext,
    app: &Entity<TerminalApp>,
) -> (Vec<PanelId>, usize) {
    cx.update(|cx| {
        let app = app.read(cx);
        let area = app.dock_area.read(cx);
        let tree = area.layout(DockPlacement::Center).expect("center 槽在场");
        let mut found = (Vec::new(), 0usize);
        tree.root().walk(&mut |node| {
            if let gpui_kit::base::dock::layout::PaneRef::Tabs { panels, active_ix } =
                node.kind()
            {
                if found.0.is_empty() {
                    found = (panels.to_vec(), active_ix);
                }
            }
        });
        found
    })
}

fn console_id(cx: &mut TestAppContext, app: &Entity<TerminalApp>) -> PanelId {
    cx.update(|cx| PanelId::from(app.read(cx).terminal.entity_id()))
}

/// 写 hermetic 临时文件 (pid 隔离), 返回绝对路径字符串。
fn write_temp_file(tag: &str, ext: &str, content: &str) -> String {
    let path = std::env::temp_dir().join(format!("dbg-{tag}-{}.{ext}", std::process::id()));
    std::fs::write(&path, content).expect("写临时文件");
    path.to_string_lossy().into_owned()
}

/// 走统一后端开一个对象 (pdm open_object_in 同款: 真实调用链)。
fn open_object_in(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    app: &Entity<TerminalApp>,
    obj: OpenObject,
) {
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| app.open_object(obj, window, cx));
    })
    .expect("update window");
    cx.run_until_parked();
}

// ---------- debug section 专用 helper ----------

/// 走平行入口开一个 section (真实调用链: update_window → app.update)。
fn open_section_in(cx: &mut TestAppContext, handle: AnyWindowHandle, app: &Entity<TerminalApp>, id: &str) {
    cx.update_window(handle, |_, window, cx| {
        app.update(cx, |app, cx| app.open_debug_section(id, window, cx));
    })
    .expect("update window");
    cx.run_until_parked();
}

/// section 面板实体 (dedup 表 → dock → 两跳 downcast, pdm viewer_entity 同款)。
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

/// 关闭 section 签 (库公开 API; 栈为 App → DockArea, on_removed 回摘
/// dedup 时 TerminalApp 无 lease — 不得包在 app.update 内, 否则重入;
/// pdm close_viewer 同款)。
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

// ---------- 5.1 四场景 ----------

#[gpui_kit::test]
fn section_opens_after_console_and_focuses(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    register_stub("test::tail-insert", "Tail Insert");

    let (before, _) = center_tabs(cx, &app);
    assert_eq!(before.len(), 1, "初始仅 console 签");
    assert_eq!(before[0], console_id(cx, &app), "console 恒 index 0");

    open_section_in(cx, handle, &app, "test::tail-insert");

    let (after, active) = center_tabs(cx, &app);
    assert_eq!(after.len(), 2, "console + section 两签");
    assert_eq!(
        after[0],
        console_id(cx, &app),
        "尾插: console 仍 index 0 (spec: 尾插在 console 后)"
    );
    let section_id = cx.update(|cx| {
        *app.read(cx)
            .debug_tabs
            .get("test::tail-insert")
            .expect("dedup 登记")
    });
    assert_eq!(after[1], section_id, "section 尾插 index 1");
    assert_eq!(active, 1, "新开签 active");

    // 焦点: 开签入口显式交焦给 active 签 (focus_active_center_panel)
    let section = section_entity(cx, &app, "test::tail-insert");
    let section_focus = cx.update(|cx| section.read(cx).focus_handle(cx));
    let focused = cx
        .update_window(handle, |_, window, cx| window.focused(cx))
        .expect("focused");
    assert_eq!(
        focused,
        Some(section_focus),
        "开签即聚焦 section (spec: activated for display)"
    );

    // 标题 = 登记的 title (task 5.2: title() 直读)
    let title = cx.update(|cx| section.read(cx).title.to_string());
    assert_eq!(title, "Tail Insert", "tab 标题取登记 title");

    // section 视图经 AnyView 渲染进签 (task 5.2: Entity section 渲染进签)
    let texts = render_texts(handle, cx);
    assert!(
        texts.iter().any(|t| t == "stub-section-body"),
        "section 视图渲染进签, got {texts:?}"
    );
}

#[gpui_kit::test]
fn second_open_focuses_without_duplicate(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    register_stub("test::second-open", "Second Open");

    open_section_in(cx, handle, &app, "test::second-open");
    let after_first = center_tabs(cx, &app).0;
    assert_eq!(after_first.len(), 2, "首开两签");

    // 焦点移开 (模拟用户点回 console) — 先证明确实移走了
    let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));
    cx.update_window(handle, |_, window, cx| term_focus.focus(window, cx))
        .expect("focus console");
    let focused = cx
        .update_window(handle, |_, window, cx| window.focused(cx))
        .expect("focused");
    assert_eq!(focused, Some(term_focus), "前置: 焦点已移回 console");

    // 二次打开同 id → 聚焦既有签, 不重复建签
    open_section_in(cx, handle, &app, "test::second-open");
    let (after_second, active) = center_tabs(cx, &app);
    assert_eq!(
        after_second.len(),
        2,
        "二次打开不建新签 (spec: Re-selecting an open section focuses it)"
    );
    assert_eq!(after_second, after_first, "签集合不变");
    assert_eq!(active, 1, "active 仍是 section");

    let section = section_entity(cx, &app, "test::second-open");
    let section_focus = cx.update(|cx| section.read(cx).focus_handle(cx));
    let focused = cx
        .update_window(handle, |_, window, cx| window.focused(cx))
        .expect("focused");
    assert_eq!(
        focused,
        Some(section_focus),
        "二次打开把焦点拉回 section"
    );
}

#[gpui_kit::test]
fn closed_section_can_be_reopened(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    register_stub("test::reopen", "Reopen");

    open_section_in(cx, handle, &app, "test::reopen");
    let first_id = cx.update(|cx| {
        *app.read(cx)
            .debug_tabs
            .get("test::reopen")
            .expect("dedup 登记")
    });
    let first = section_entity(cx, &app, "test::reopen");

    close_section(cx, handle, &app, first);

    let gone = cx.update(|cx| !app.read(cx).debug_tabs.contains_key("test::reopen"));
    assert!(gone, "关闭摘 dedup (spec: Closing a tab removes its entry)");
    let (tabs, _) = center_tabs(cx, &app);
    assert_eq!(
        tabs,
        vec![console_id(cx, &app)],
        "关闭后仅剩 console, console 仍 index 0"
    );

    // 重开 → 全新签 (fresh PanelId)
    open_section_in(cx, handle, &app, "test::reopen");
    let second_id = cx.update(|cx| {
        *app.read(cx)
            .debug_tabs
            .get("test::reopen")
            .expect("重开后 dedup 重登记")
    });
    let (tabs2, active) = center_tabs(cx, &app);
    assert_eq!(tabs2.len(), 2, "重开成签 (spec: Reopen after close creates a fresh tab)");
    assert_ne!(second_id, first_id, "重开 = 全新 PanelId");
    assert_eq!(active, 1, "重开签 active");
}

#[gpui_kit::test]
fn section_id_independent_of_file_path_identity(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    let path = write_temp_file("secid", "rs", "fn probe() {}");
    // section id 恰等于文件路径 (spec 场景: a debug section whose id
    // happens to equal an open file path) — 注册表 id 是 &'static str,
    // 测试路径泄漏成 'static (进程生命周期, 测试可接受)。
    let id: &'static str = Box::leak(path.clone().into_boxed_str());
    register_stub(id, "Path Section");

    // 文件走 file backend (key = 路径) → viewer 签
    open_object_in(
        cx,
        handle,
        &app,
        OpenObject {
            path: PathBuf::from(&path),
            kind: AssetKind::File,
        },
    );
    // 同字符串走 section backend (key = section id) → 独立 section 签
    open_section_in(cx, handle, &app, id);

    let (tabs, _) = center_tabs(cx, &app);
    assert_eq!(
        tabs.len(),
        3,
        "console + viewer + section 三签 (身份独立, spec: a separate section tab)"
    );

    let (file_panel, section_panel) = cx.update(|cx| {
        let app = app.read(cx);
        (
            app.open_tabs.get(&path).copied(),
            app.debug_tabs.get(id).copied(),
        )
    });
    let file_panel = file_panel.expect("文件 dedup 登记");
    let section_panel = section_panel.expect("section dedup 登记");
    assert_ne!(
        file_panel, section_panel,
        "同字符串两套身份 → 不同 PanelId"
    );
    assert!(
        tabs.contains(&file_panel) && tabs.contains(&section_panel),
        "两签都在 dock 中"
    );
}

// ---------- 5.2: 签可关 + console 恒第一签 ----------

#[gpui_kit::test]
fn section_tab_is_closable_and_console_stays_first(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    register_stub("test::closable", "Closable");

    open_section_in(cx, handle, &app, "test::closable");
    let section = section_entity(cx, &app, "test::closable");

    let closable = cx.update(|cx| section.read(cx).closable(cx));
    assert!(closable, "spec: debug section tab SHALL be closable");

    let (tabs, _) = center_tabs(cx, &app);
    assert_eq!(tabs[0], console_id(cx, &app), "开签后 console 仍 index 0");

    close_section(cx, handle, &app, section);

    let (after, _) = center_tabs(cx, &app);
    assert_eq!(
        after,
        vec![console_id(cx, &app)],
        "关后 console 恒第一签且为唯一签 (spec: console 恒第一签不变)"
    );
}
