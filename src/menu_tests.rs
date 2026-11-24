//! 无头验收 (add-debug-console 6.1): 菜单内容在场 + Copy 可用性随选区灰显。
//!
//! 显式导入, 不用 glob — 防 `gpui_kit::*` 把 `#[test]` 宏遮蔽掉
//! (app.rs / pdm tests 同款教训)。

use crate::app::TerminalApp;
use crate::workspace::Workspace;
use agent_service::terminal::{CellPos, CellRange, TerminalCopy};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Focusable, OwnedMenuItem, Point, TestAppContext,
    WindowBounds, WindowOptions, px, size,
};

/// 起完整 app（隔离全局历史/外部配置, 模式同 pdm/tree tests）。
fn open_app(cx: &mut TestAppContext) -> (AnyWindowHandle, gpui_kit::Entity<TerminalApp>) {
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

fn render(handle: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
    })
    .expect("update window");
    cx.run_until_parked();
}

/// 注入工作区 + 会话 + 首帧 (选区哨兵的 live 文本来源 — 无会话则
/// `selection_text_now` 返 None, 直接注入的选区在下一帧被哨兵清掉)。
fn inject_session(cx: &mut TestAppContext, app: &gpui_kit::Entity<TerminalApp>, tag: &str) {
    use crate::service_layer::test_sessions;
    let ws = std::env::temp_dir().join(format!("menu-ws-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).unwrap();
    app.update(cx, |app, cx| {
        app.workspace = Some(Workspace::new(ws.clone()));
        app.notify_panels(cx);
    });
    let (id, _shared) = test_sessions::create_session(cx, app, &ws);
    app.update(cx, |app, cx| {
        app.terminal.update(cx, |t, cx| {
            t.feed(id, b"probe\r\n", agent_service::frame_gen::current(), cx)
        });
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn menu_bar_has_edit_copy(cx: &mut TestAppContext) {
    let (_handle, _app) = open_app(cx);

    let menus = cx.update(|cx| cx.get_menus()).expect("菜单已安装 (装配期 set_menus)");
    let edit = menus
        .iter()
        .find(|m| m.name.as_str() == "Edit")
        .expect("Edit 菜单在场");
    let copy = edit
        .items
        .iter()
        .find_map(|item| match item {
            OwnedMenuItem::Action { name, action, .. } if name.as_str() == "Copy" => Some(action),
            _ => None,
        })
        .expect("Copy 项在场");
    assert!(
        copy.partial_eq(&TerminalCopy),
        "Copy 项应接现有 TerminalCopy action (实际: {})",
        copy.name()
    );
}

#[gpui_kit::test]
fn copy_availability_follows_selection(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    inject_session(cx, &app, "menu-avail");
    let _ = render(handle, cx);

    // 焦点进终端面板 — 可用性沿焦点路径查 on_action 监听
    let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));
    cx.update_window(handle, |_, window, cx| term_focus.focus(window, cx))
        .expect("focus terminal");
    let _ = render(handle, cx);

    // 无选区 → 监听不挂 → Copy 不可用 (灰显, spec: disabled without a selection)
    let available = cx
        .update_window(handle, |_, window, cx| {
            window.is_action_available(&TerminalCopy, cx)
        })
        .expect("is_action_available");
    assert!(!available, "无选区时 Copy 应灰显");

    // 建立选区 (直接置字段; 快照 None = 拖动中语义, 哨兵不判漂) → 下帧挂监听
    app.update(cx, |app, cx| {
        app.terminal.update(cx, |t, cx| {
            t.selection = Some(CellRange {
                anchor: CellPos { row: 0, col: 0 },
                focus: CellPos { row: 0, col: 4 },
            });
            cx.notify();
        });
    });
    let _ = render(handle, cx);

    // 有选区 → 监听挂载 → Copy 可用
    let available = cx
        .update_window(handle, |_, window, cx| {
            window.is_action_available(&TerminalCopy, cx)
        })
        .expect("is_action_available");
    assert!(available, "有选区时 Copy 应可用");
}

/// task 6.2: Debug 子菜单从注册表产出 section 项 (spec: Menu bar exists in
/// debug builds → including the Debug menu)。
///
/// `#[cfg(debug_assertions)]` 随 `crate::debug` 门控 — `cargo test --release`
/// 下注册表不存在, 整测试不编译。注册表是进程全局 (与 debug 模块测试
/// 共享): 用独立 id, 断言"包含"而不假设全集, 并发注册不干扰。
#[cfg(debug_assertions)]
#[gpui_kit::test]
fn debug_submenu_lists_registered_sections(cx: &mut TestAppContext) {
    fn never_called(_: &'static str, _: &mut gpui_kit::Context<TerminalApp>) -> gpui_kit::AnyView {
        unimplemented!("菜单构建不调工厂 (开签才建视图)")
    }
    crate::debug::register(crate::debug::DebugSection {
        id: "test::menu-sec",
        title: "Test Menu Section",
        factory: never_called,
    });

    let (_handle, _app) = open_app(cx);
    let menus = cx.update(|cx| cx.get_menus()).expect("菜单已安装");

    // Edit 常驻 (design D6: 菜单栏常驻, Debug 只是追加)
    assert!(menus.iter().any(|m| m.name.as_ref() == "Edit"), "Edit 菜单仍在");
    // Debug 菜单在场
    let debug_menu = menus
        .iter()
        .find(|m| m.name.as_ref() == "Debug")
        .expect("Debug 菜单在场 (debug 构建)");
    // 节标题 → 菜单项 (注册表 → 菜单内容)
    assert!(
        debug_menu.items.iter().any(|item| matches!(
            item,
            OwnedMenuItem::Action { name, .. } if name == "Test Menu Section"
        )),
        "注册表 section 应产出 Debug 菜单项"
    );
}

/// task 6.3: 菜单项 action → `cx.on_action` 全局监听 → 5.1 开签入口。
///
/// 整块随 `crate::debug` 门控 (`OpenDebugSection`/`register` release 不存在,
/// `cargo test --release` 下不编译)。注册表进程全局 — 独立 id, 断言"命中"。
#[cfg(debug_assertions)]
mod dispatch_tests {
    use super::*;
    use crate::debug::{DebugSection, register};
    use gpui_kit::base::dock::PanelId;
    use gpui_kit::component::dock::DockPlacement;
    use gpui_kit::{AnyView, Context, Entity, IntoElement, ParentElement, Render, Window, div};

    /// 测试用 section 视图 (panel_tests StubSection 同款, 本地副本)。
    struct MenuStubSection;

    impl Render for MenuStubSection {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().child("menu-stub-section-body")
        }
    }

    fn stub_factory(_: &'static str, cx: &mut Context<TerminalApp>) -> AnyView {
        cx.new(|_| MenuStubSection).into()
    }

    /// 中心 tab 组 (panel_tests center_tabs 同款副本) — 签是否真进中心槽。
    fn center_tabs(cx: &mut TestAppContext, app: &Entity<TerminalApp>) -> Vec<PanelId> {
        cx.update(|cx| {
            let app = app.read(cx);
            let area = app.dock_area.read(cx);
            let tree = area.layout(DockPlacement::Center).expect("center 槽在场");
            let mut panels = Vec::new();
            tree.root().walk(&mut |node| {
                if let gpui_kit::base::dock::layout::PaneRef::Tabs { panels: ps, .. } = node.kind() {
                    if panels.is_empty() {
                        panels = ps.to_vec();
                    }
                }
            });
            panels
        })
    }

    #[gpui_kit::test]
    fn debug_menu_item_opens_section_tab(cx: &mut TestAppContext) {
        register(DebugSection {
            id: "test::menu-open",
            title: "Test Menu Open",
            factory: stub_factory,
        });

        let (handle, app) = open_app(cx);
        let _ = render(handle, cx);

        // 激活窗口 — 监听体读 active_window, 菜单派发走活动窗口路径
        cx.update_window(handle, |_, window, _| window.activate_window())
            .expect("activate");

        // 模拟点击菜单项: 平台 on_app_menu_action 回调 → cx.dispatch_action
        // (与真实点击同一接缝 — 仅平台原生点击本身无法无头驱动)
        cx.update(|cx| {
            cx.dispatch_action(&crate::debug::OpenDebugSection {
                id: "test::menu-open".to_string(),
            })
        });
        cx.run_until_parked();
        let _ = render(handle, cx); // 泵帧: defer effect flush + 新签渲染

        // 5.1 入口命中 (dedup 表登记) + 签真进了中心槽 (非只记表)
        let section_panel = cx.update(|cx| {
            *app.read(cx)
                .debug_tabs
                .get("test::menu-open")
                .expect("菜单项 action → 开签入口命中")
        });
        let panels = center_tabs(cx, &app);
        assert!(panels.contains(&section_panel), "section 签应在中心槽 (签打开)");
    }

    /// task 8.1: 装配期注册 (design D11) — debug 构建启动后 Debug 菜单
    /// 列出内置两节, 依次开签正常 (菜单项 action → 6.3 分发 → 5.1 入口
    /// → 中心槽依次尾插在 console 后)。
    #[gpui_kit::test]
    fn builtin_sections_listed_and_open_in_turn(cx: &mut TestAppContext) {
        // open_app = 真实装配链: TerminalApp::new → register_builtin →
        // menu::install (注册表先于菜单构建 — D11 顺序)。
        let (handle, app) = open_app(cx);

        let menus = cx.update(|cx| cx.get_menus()).expect("菜单已安装");
        let debug_menu = menus
            .iter()
            .find(|m| m.name.as_str() == "Debug")
            .expect("Debug 菜单在场 (debug 构建)");
        for title in ["term-rate", "threads/pipeline"] {
            assert!(
                debug_menu.items.iter().any(|item| matches!(
                    item,
                    OwnedMenuItem::Action { name, .. } if name == title
                )),
                "内置 section 应列出在 Debug 菜单: {title}"
            );
        }

        // 激活窗口 — 菜单 action 派发走活动窗口路径 (6.3 同款)
        cx.update_window(handle, |_, window, _| window.activate_window())
            .expect("activate");

        // 依次开签: 菜单项 action → 6.3 分发 → 5.1 入口
        let mut opened = Vec::new();
        for id in ["term-rate", "threads-pipeline"] {
            cx.update(|cx| {
                cx.dispatch_action(&crate::debug::OpenDebugSection { id: id.to_string() })
            });
            cx.run_until_parked();
            let _ = render(handle, cx); // 泵帧: defer flush + 新签渲染
            opened.push(
                cx.update(|cx| *app.read(cx).debug_tabs.get(id).expect("开签命中 dedup 表")),
            );
        }

        // 中心槽 = [console, term-rate, threads] — 依次尾插且顺序正确
        let console = cx.update(|cx| PanelId::from(app.read(cx).terminal.entity_id()));
        let mut expected = vec![console];
        expected.extend(opened);
        assert_eq!(center_tabs(cx, &app), expected, "依次开签尾插且顺序正确");
    }
}

/// task 8.2 + spec *No Debug menu in release*: release 构建菜单栏无 Debug
/// 菜单, Edit/Copy 仍在 (design D6 常驻)。仅 `cargo test --release`
/// 编译 (debug_assertions 关) — debug 测试构建里 Debug 菜单本就在场,
/// 反向断言会失败。
#[cfg(not(debug_assertions))]
#[gpui_kit::test]
fn release_menu_has_no_debug_and_keeps_edit_copy(cx: &mut TestAppContext) {
    let (_handle, _app) = open_app(cx);
    let menus = cx.update(|cx| cx.get_menus()).expect("菜单已安装");
    assert!(
        !menus.iter().any(|m| m.name.as_str() == "Debug"),
        "release 菜单不得有 Debug (spec: No Debug menu in release)"
    );
    let edit = menus
        .iter()
        .find(|m| m.name.as_str() == "Edit")
        .expect("Edit 菜单仍在 (design D6 常驻)");
    assert!(
        edit.items.iter().any(|item| matches!(
            item,
            OwnedMenuItem::Action { name, .. } if name == "Copy"
        )),
        "Edit/Copy 仍在 (spec: release 菜单栏含 Copy)"
    );
}
