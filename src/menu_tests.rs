//! 无头验收 (add-debug-console 6.1): 菜单内容在场 + Copy 可用性随选区灰显。
//!
//! 显式导入, 不用 glob — 防 `gpui_kit::*` 把 `#[test]` 宏遮蔽掉
//! (app.rs / pdm tests 同款教训)。

use crate::app::TerminalApp;
use crate::workspace::Workspace;
use agent_service::terminal::{CellPos, CellRange, TerminalCopy};
use gpui_kit::test::{TestQueryExt, TestWindowExt};
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
            |window, cx| cx.new(|cx| TerminalApp::new(window, cx).expect("app 启动")),
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

/// win-in-window-app-menu-bar task 2.3 (spec 场景: Windowed menu bar is
/// visible above the dock on Windows): 帧内 `Edit` 菜单在场, debug 构建另含
/// `Debug`, 且菜单栏落在 dock 之上。
///
/// 取元素用 accessibility label (`find_by_label`) 而非 `texts()` ——
/// Button 的 label 走 a11y label 路径, `texts()` 只收 `ObservedText` 绘的
/// 字串 (与 assets/panel_tests 同款取法)。
///
/// cfg 门控 (design D6): 窗口内菜单栏只在非 mac 挂载 — macOS 走原生
/// `set_menus`, 帧里本就没有这些元素, 同一份断言在 macOS 必挂。
#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn windowed_menu_bar_renders_above_dock(cx: &mut TestAppContext) {
    let (handle, _app) = open_app(cx);
    render(handle, cx); // 建观测注册表 (find_by_label 查上一帧)

    let (edit_y, dock_y, debug_present) = cx
        .update_window(handle, |_, window, _cx| {
            let edit = window.find_by_label("Edit");
            let dock = window.find("dock-slot");
            // debug 构建才产 Debug 菜单 (design D5); release 反向断言缺席
            let debug_present = window.try_find_by_label("Debug").is_some();
            (
                f32::from(edit.bounds().origin.y),
                f32::from(dock.bounds().origin.y),
                debug_present,
            )
        })
        .expect("query menu bar");

    // spec: 至少一个菜单, 且 Debug 子菜单只在 debug 构建 (design D5)
    #[cfg(debug_assertions)]
    assert!(debug_present, "debug 构建帧内应含 Debug 菜单");
    #[cfg(not(debug_assertions))]
    assert!(!debug_present, "release 帧内不应含 Debug 菜单");

    // 位置: 菜单栏必须落在 dock 之上 (spec: rendered inside the window
    // above the dock layout) — 菜单栏 wrapper 高 28px, dock 紧接其下
    assert!(
        edit_y < dock_y,
        "Edit 应画在 dock 之上: edit.y={edit_y} dock.y={dock_y}"
    );
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

/// GlobalState 快照 (菜单名, action 项名列表) — 非 Action 项（分隔线/子菜单）
/// 不参与顺序断言。
fn global_menus(cx: &mut TestAppContext) -> Vec<(String, Vec<String>)> {
    cx.update(|cx| {
        gpui_kit::base::GlobalState::global(cx)
            .app_menus()
            .iter()
            .map(|m| {
                (
                    m.name.to_string(),
                    m.items
                        .iter()
                        .filter_map(|i| match i {
                            OwnedMenuItem::Action { name, .. } => Some(name.clone()),
                            _ => None,
                        })
                        .collect(),
                )
            })
            .collect()
    })
}

fn menu_names(s: &[(String, Vec<String>)]) -> Vec<&str> {
    s.iter().map(|(n, _)| n.as_str()).collect()
}

fn menu_items<'a>(s: &'a [(String, Vec<String>)], name: &str) -> &'a [String] {
    &s.iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("{name} 菜单在场"))
        .1
}

/// win-in-window-app-menu-bar task 3.1 (design D3): 统一重建函数两步通路 ——
/// 先 `set_app_menus` 喂 GlobalState, 再 `AppMenuBar::reload`。
///
/// 断言 ① 源变化后第一次重建让 GlobalState **内容更新**;
/// 断言 ② 第二次重建**不改变已有项的顺序**。
///
/// 注册表是**进程全局**的（与 panel_tests / dispatch_tests 并发共享）,
/// `register` 只追加或同 id 原地覆盖 → 已有项的顺序与内容恒定。故用
/// **前缀**断言而非全等: 并发注册只会往后追加, 打不破前缀; 全等断言
/// 会被并发追加打 flake。
#[cfg(debug_assertions)]
#[gpui_kit::test]
fn rebuild_menus_updates_content_then_keeps_order(cx: &mut TestAppContext) {
    /// 只建菜单、不打开 section — 本测试不碰视图工厂
    fn probe_factory(
        _: &'static str,
        _: &mut gpui_kit::Context<TerminalApp>,
    ) -> gpui_kit::AnyView {
        unimplemented!("本测试只重建菜单, 不打开 section")
    }

    let (_handle, app) = open_app(cx);

    // 源侧变化: 注册新 section。GlobalState 此刻仍是 install 时的旧快照
    crate::debug::register(crate::debug::DebugSection {
        id: "test::rebuild-probe",
        title: "Rebuild Probe",
        factory: probe_factory,
    });
    assert!(
        !global_menus(&mut *cx)
            .iter()
            .any(|(_, items)| items.iter().any(|i| i == "Rebuild Probe")),
        "注册后重建前 GlobalState 应仍是旧快照 (否则下面的内容更新断言无意义)"
    );

    // ① 第一次重建 → 内容更新
    app.update(cx, |app, cx| app.rebuild_menus(cx));
    let first = global_menus(cx);
    assert_eq!(
        menu_names(&first),
        vec!["Edit", "Debug"],
        "菜单序列 = build() 产出 (design D6: Edit 常驻 + Debug debug-only)"
    );
    let debug1 = menu_items(&first, "Debug").to_vec();
    assert!(
        debug1.iter().any(|i| i == "Rebuild Probe"),
        "第一次重建后新 section 应进 GlobalState, 得 {debug1:?}"
    );

    // ② 第二次重建 → 已有项顺序不变 (前缀)
    app.update(cx, |app, cx| app.rebuild_menus(cx));
    let second = global_menus(cx);
    let debug2 = menu_items(&second, "Debug");
    assert!(
        debug2.starts_with(&debug1),
        "第二次重建不得改变已有项顺序:\n  before = {debug1:?}\n  after  = {debug2:?}"
    );
    assert_eq!(
        menu_items(&second, "Edit"),
        menu_items(&first, "Edit"),
        "Edit/Copy 不受 Debug 注册影响"
    );
}


/// 打开下标 `ix` 的菜单栏项 — **仅在 popup 未开时点击**。
///
/// 作用域用 `within(ix)`: `AppMenu` 根 div (`div().id(self.ix)`) 自身
/// **无** `.test_support()`, 但被观测 Button 的 `global_id` 路径里含它,
/// 故 `observation::scope` 能以它收窄。
///
/// 「只在未开时点」是硬约束: popup 开着时项的 `MenuItemElement` 序号也是
/// `Integer(ix)`, 会撞 `observation::scope` 的唯一性断言 (paths.len()==1)。
fn open_menu_at(cx: &mut TestAppContext, handle: AnyWindowHandle, ix: usize) {
    let is_open = cx
        .update_window(handle, |_, window, _cx| window.try_find("popup-menu").is_some())
        .expect("query popup");
    if !is_open {
        cx.update_window(handle, |_, window, cx| window.within(ix).click("menu", cx))
            .expect("click menu-bar item");
        cx.run_until_parked();
        render(handle, cx);
    }
}

/// 打开 Edit 菜单, 断言 popup 内 Copy 项在场; 返回 popup 是否开着。
///
/// 「未开会 panic」本身就是菜单没开的信号 —— `find_by_label` 找不到即报
/// missing accessible label, 不用另设布尔。
fn open_edit_menu(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    open_menu_at(cx, handle, 0);
    cx.update_window(handle, |_, window, _cx| {
        assert!(
            window.try_find_by_label("Copy").is_some(),
            "popup 应含 Copy 项 (未开时 find_by_label 会 panic, 故兼证菜单已开)"
        );
        window.try_find("popup-menu").is_some()
    })
    .expect("query popup after open")
}

/// Copy 项的灰显**源值** — 读 `GlobalState` 的 `OwnedMenuItem::Action.disabled`。
///
/// **为什么不读 popup 项的 a11y `disabled()`**: `MenuItemElement::render`
/// 只写 `role`/`aria_label`/`aria_selected`/`aria_toggled`, **不把
/// `disabled` 写进 a11y node** (`self.disabled` 只控 hover/click 分支与前景色)
/// —— gpui-component 0.9.0 的观测盲区, `find_by_label("Copy").disabled()`
/// 恒 `None`, 与灰不灰无关。
///
/// 源值即渲染值: `PopupMenu::with_menu_items` 把该 `disabled` **逐字**拷进
/// popup 项 (popup_menu.rs:811-817), 无二次计算。
fn copy_disabled(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| {
        gpui_kit::base::GlobalState::global(cx)
            .app_menus()
            .iter()
            .find(|m| m.name.as_str() == "Edit")
            .expect("Edit 菜单在 GlobalState 内")
            .items
            .iter()
            .find_map(|item| match item {
                OwnedMenuItem::Action { name, disabled, .. } if name == "Copy" => Some(*disabled),
                _ => None,
            })
            .expect("Copy 项在 Edit 菜单内")
    })
}

/// win-in-window-app-menu-bar task 3.2 (spec 场景: Copy item enabled state
/// follows selection changes): Copy 灰显只随**选区**翻转, 且重建不打乱
/// 菜单/菜单项顺序 (spec: Rebuild preserves unchanged items)。
///
/// 观测口径分两半 (见 helper 文档):
/// - **菜单真的开了 + Copy 项在场** → `open_edit_menu` (popup a11y);
/// - **灰显值** → `copy_disabled` (GlobalState 源, `with_menu_items` 逐字拷
///   进 popup) — gpui-component 的 MenuItemElement 不把 `disabled` 写进
///   a11y node, 故 popup 侧的 `.disabled()` 恒 None, 不能作断言依据。
#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn copy_disabled_follows_selection(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    // 会话 + 首帧 — 选区哨兵要 live 文本, 无会话则注入的选区下一帧被清
    inject_session(cx, &app, "copy-dis");
    render(handle, cx);

    let first = global_menus(cx);
    let baseline = menu_names(&first).to_vec();
    // 菜单序列按**构建 profile** 给期望 — design D5: Debug 菜单整块
    // `cfg(debug_assertions)`, release 不产出; design D6: Edit 恒常驻。
    // 断言保留两 profile 都跑: 灰显逻辑属 Edit/Copy, release 也必须覆盖,
    // 直接 cfg 掉整个测试会让 release 侧无人测。
    #[cfg(debug_assertions)]
    assert_eq!(
        baseline,
        vec!["Edit", "Debug"],
        "debug 构建菜单序列 = Edit 常驻 + Debug (design D5/D6)"
    );
    #[cfg(not(debug_assertions))]
    assert_eq!(
        baseline,
        vec!["Edit"],
        "release 构建不得含 Debug 菜单 (design D5)"
    );

    // ① 无选区 → 菜单可开 + Copy 灰显
    assert!(open_edit_menu(cx, handle), "无选区时 Edit 菜单应能打开");
    assert!(
        copy_disabled(cx),
        "无选区 Copy 应灰显 (spec: Copy disabled without a selection)"
    );

    // ② 注入选区 → 翻转 → 重建 → Copy 可用
    app.update(cx, |app, cx| {
        app.terminal.update(cx, |t, cx| {
            t.selection = Some(CellRange {
                anchor: CellPos { row: 0, col: 0 },
                focus: CellPos { row: 0, col: 4 },
            });
            cx.notify();
        });
    });
    render(handle, cx); // TerminalApp::render 重跑 → 比对翻转 → 重建
    assert!(open_edit_menu(cx, handle), "有选区时 Edit 菜单应能打开");
    assert!(
        !copy_disabled(cx),
        "有选区 Copy 应可用 (spec: Copy enabled with an active selection)"
    );

    // ③ 清选区 → 再次翻转 → 灰显
    app.update(cx, |app, cx| {
        app.terminal.update(cx, |t, cx| {
            t.selection = None;
            cx.notify();
        });
    });
    render(handle, cx);
    assert!(open_edit_menu(cx, handle), "清选区后 Edit 菜单应能打开");
    assert!(copy_disabled(cx), "清选区 Copy 应再次灰显");

    // ④ 全程菜单与已有项顺序不变 (spec: Rebuild preserves unchanged items)
    //
    // **前缀而非全等**: Debug 注册表是**进程全局**的, 本测试三步渲染 +
    // 两次开菜单耗时较长, 其它并行测试会往里 append 自己的 section ——
    // register 只追加或同 id 原地覆盖, 故 first 项必是 last 的前缀,
    // 而全等断言会被并发追加打 flake (与 rebuild_menus_... 同款口径)。
    // spec 要的是「已有项不被打乱」, 不是「总数不变」。
    let last = global_menus(cx);
    assert_eq!(
        menu_names(&last),
        baseline,
        "重建不得改变菜单序列"
    );
    assert_eq!(
        menu_items(&last, "Edit"),
        menu_items(&first, "Edit"),
        "Edit/Copy 项不得变"
    );
    #[cfg(debug_assertions)]
    assert!(
        menu_items(&last, "Debug").starts_with(menu_items(&first, "Debug")),
        "Debug 已有项顺序不得变:\n  before = {:?}\n  after  = {:?}",
        menu_items(&first, "Debug"),
        menu_items(&last, "Debug")
    );
}

/// win-in-window-app-menu-bar task 3.3 (design Risks: TerminalApp::render 是
/// memoized 的, 未必每次选区变化都重跑): 验证 `AgentTerminal::notify` 经
/// `mark_view_dirty` + `view_path_reversed` **把祖先链上的 TerminalApp 标脏**,
/// 使 `TerminalApp::render` 在选区变化后自动重跑并推进 GlobalState。
///
/// **本测试在选区变化后不调 `render()`、也不开菜单** —— `click_target` 内部
/// 自带 `render_frame`, 一点菜单就把被测机制掩盖了; `run_until_parked` 只跑
/// dispatcher 不绘制, 所以画面推进**只能**来自 notify 标脏后的重渲。
///
/// 观测点是 `copy_disabled` (GlobalState), 它**只**在 `TerminalApp::render`
/// 里被 `rebuild_menus` 更新 —— 不重跑就不会变。
#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn render_reruns_on_selection_change_without_manual_render(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    inject_session(cx, &app, "notify-chain");
    render(handle, cx); // 首帧 (装配期 install 已 feed: 无选区 → 灰显)

    assert!(
        copy_disabled(cx),
        "前置: 装配期无选区, Copy 应已灰显"
    );

    // 选区变化: 只 notify (AgentTerminal 的 cx.notify), **不 render、不开菜单**
    app.update(cx, |app, cx| {
        app.terminal.update(cx, |t, cx| {
            t.selection = Some(CellRange {
                anchor: CellPos { row: 0, col: 0 },
                focus: CellPos { row: 0, col: 4 },
            });
            cx.notify();
        });
    });
    cx.run_until_parked();

    assert!(
        !copy_disabled(cx),
        "notify 应使 TerminalApp::render 重跑并重建菜单 (design Risks 末条); \
         若此处失败 = 祖先标脏链断了, 需在选区变化处补 TerminalApp 显式 notify"
    );
}

/// win-in-window-app-menu-bar task 3.4 (spec 场景: Focus outside the console
/// does not disable Copy): 灰显判定**只看选区, 焦点不参与** (design D2)。
///
/// 关键是**反向对照** —— 焦点移出 console 后, gpui 的动态可用性路径
/// (`window.is_action_available`) 此刻确实报「不可用」(Copy 监听挂在 terminal
/// pane 上, 焦点不在其派发路径内)。若只断言「菜单仍可用」, 无法区分
/// 「焦点真的不参与」与「断言压根没生效」。两条一起断言才把这个分叉钉死。
///
/// 代价 (design D2 已接受): 焦点在侧栏 + 有选区时菜单显示可用, 点下去
/// `dispatch_confirm_action` 恢复 `action_context` 后路径上无监听 → 静默无操作;
/// `handle_copy` 本身无选区即返回, 不会误写剪贴板。
#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn focus_outside_console_does_not_disable_copy(cx: &mut TestAppContext) {
    let (handle, app) = open_app(cx);
    inject_session(cx, &app, "focus-out");
    render(handle, cx);

    // 有选区 → 应可用 (前置)
    app.update(cx, |app, cx| {
        app.terminal.update(cx, |t, cx| {
            t.selection = Some(CellRange {
                anchor: CellPos { row: 0, col: 0 },
                focus: CellPos { row: 0, col: 4 },
            });
            cx.notify();
        });
    });
    render(handle, cx);
    assert!(!copy_disabled(cx), "前置: 有选区 + 焦点在 console, Copy 应可用");

    // 焦点移至 sidebar
    let sidebar_focus = cx.update(|cx| app.read(cx).sidebar.read(cx).focus_handle(cx));
    cx.update_window(handle, |_, window, cx| sidebar_focus.focus(window, cx))
        .expect("focus sidebar");
    render(handle, cx);

    // 反向对照: 动态可用性路径此刻报不可用 — 证明下面的断言非空转
    let available = cx
        .update_window(handle, |_, window, cx| {
            window.is_action_available(&TerminalCopy, cx)
        })
        .expect("is_action_available");
    assert!(
        !available,
        "反向对照: 焦点已移出 console, is_action_available 应为 false — \
         否则本测试的『焦点不参与』断言就没有区分度"
    );

    // 正向: 灰显仍只看选区 → 焦点在外仍可用
    assert!(
        open_edit_menu(cx, handle),
        "焦点在侧栏时 Edit 菜单仍应能打开"
    );
    assert!(
        !copy_disabled(cx),
        "焦点不参与灰显判定 (spec: Focus outside the console does not disable Copy)"
    );
}

/// win-in-window-app-menu-bar task 4.3 (spec 场景: Shortcut resolves to
/// Ctrl+C on Windows / Cmd+C on macOS): `secondary-` 键位必须按**平台**解析。
///
/// **一个测试覆盖两平台分支** —— `cfg!(target_os = "macos")` 编译期定值,
/// macOS 跑 platform 分支、Windows 跑 control 分支, 免两份重复测试;
/// 也免 cross-compile `cargo test --target x86_64-apple-darwin`
/// (design Risks: 本机无 Apple SDK, 见 task 2.2 同款限制)。
///
/// 读 `inner().modifiers` 而非 `modifiers()`: 后者在 Windows 返回
/// **display** modifiers (`Keystroke::modifiers` 文档), 与参与匹配的
/// GPUI 表示可能差 shift —— 断言语义应看 `inner`。
#[gpui_kit::test]
fn terminal_copy_binding_resolves_per_platform(cx: &mut TestAppContext) {
    let (_handle, _app) = open_app(cx);

    let (control, platform, key, has_ctx) = cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let mut out = (false, false, String::new(), false);
        for binding in keymap.bindings_for_action(&TerminalCopy) {
            let Some(keystroke) = binding.keystrokes().first() else {
                continue;
            };
            out = (
                keystroke.inner().modifiers.control,
                keystroke.inner().modifiers.platform,
                keystroke.inner().key.to_string(),
                binding.predicate().is_some(),
            );
            break;
        }
        out
    });

    assert_eq!(key, "c", "TerminalCopy 主键应为 c");
    assert!(
        has_ctx,
        "绑定应带 Terminal context (design D5: 不与文本框的复制键抢)"
    );

    if cfg!(target_os = "macos") {
        assert!(
            platform && !control,
            "macOS 上 secondary 应解析为 platform(⌘), 得 control={control} platform={platform}"
        );
    } else {
        assert!(
            control && !platform,
            "Windows/Linux 上 secondary 应解析为 control(Ctrl), 得 control={control} platform={platform}"
        );
    }
}

/// win-in-window-app-menu-bar task 5.1 (spec: Menu content rebuild on change):
/// 启动后 `debug::register` 新 section → 走 **3.1 的同一重建函数** → 帧内
/// Debug 菜单列出该项, 且 `Edit`/`Debug` 顺序不变。
///
/// **补的是 3.1 未覆盖的后半段链路**: 3.1 断言到 `GlobalState` 为止, 本测试
/// 一路打到 popup 渲染 —— `GlobalState` → `AppMenuBar::reload` → `AppMenu`
/// 快照 → `PopupMenu::with_menu_items` → a11y label。
///
/// **含负向段 (证明 rebuild 必需而非碰巧)**: `AppMenu.menu` 是 `reload()`
/// 时刻从 GlobalState 拷的快照, popup 构建读该快照 —— 故注册后**不** rebuild
/// 时 popup 必不含新项; rebuild 后 `reload()` 会把 `selected_index` 清成
/// `None` (design D3 记录的行为) 自动关掉 popup, 重开即可看到新项。
#[cfg(debug_assertions)]
#[cfg(not(target_os = "macos"))]
#[gpui_kit::test]
fn register_then_rebuild_shows_section_in_debug_menu(cx: &mut TestAppContext) {
    /// 只建菜单、不打开 section — 本测试不碰视图工厂
    fn never_opened(
        _: &'static str,
        _: &mut gpui_kit::Context<TerminalApp>,
    ) -> gpui_kit::AnyView {
        unimplemented!("本测试只重建菜单, 不打开 section")
    }

    const TITLE: &str = "Registry Rebuild Probe";
    let (handle, app) = open_app(cx);
    render(handle, cx);

    let first = global_menus(cx);
    let baseline = menu_names(&first).to_vec();
    assert_eq!(
        baseline,
        vec!["Edit", "Debug"],
        "菜单序列 = build() 产出 (design D5)"
    );

    // 启动后注册 — 尚未 rebuild
    crate::debug::register(crate::debug::DebugSection {
        id: "test::registry-rebuild",
        title: TITLE,
        factory: never_opened,
    });

    // 负向: popup 快照是 reload 时刻的, 不含新 section
    open_menu_at(cx, handle, 1);
    assert!(
        cx.update_window(handle, |_, window, _| window.try_find_by_label(TITLE).is_none())
            .expect("query"),
        "未 rebuild 时 popup 不应含新 section (AppMenu 缓存 reload 时刻快照)"
    );

    // 走 3.1 的同一重建函数 → reload() 清 selected_index → popup 自动关
    app.update(cx, |app, cx| app.rebuild_menus(cx));
    render(handle, cx);
    assert!(
        cx.update_window(handle, |_, window, _| window.try_find("popup-menu").is_none())
            .expect("query"),
        "rebuild 应经 reload() 关掉开着的 popup (design D3)"
    );

    // 正向: 重开 Debug 菜单 → 新 section 帧级可见
    open_menu_at(cx, handle, 1);
    assert!(
        cx.update_window(handle, |_, window, _| window.try_find_by_label(TITLE).is_some())
            .expect("query"),
        "重建后 Debug 菜单应列出新 section (spec: Registered section appears in the menu)"
    );

    // 菜单顺序不变 (spec: Rebuild preserves unchanged items)
    let last = global_menus(cx);
    assert_eq!(
        menu_names(&last).to_vec(),
        baseline,
        "重建不得改变菜单序列"
    );
    assert_eq!(
        menu_items(&last, "Edit"),
        menu_items(&first, "Edit"),
        "Edit/Copy 项不得变"
    );
}
