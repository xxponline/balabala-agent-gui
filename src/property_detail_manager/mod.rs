//! 默认打开系统（change: center-slot-multi-tab, design D1）。
//!
//! 统一后端接口：任意槽（当前=右槽文件树）把**中立封装对象**
//! [`OpenObject`] 交给 [`TerminalApp::open_object`]，由静态 handler
//! 注册表 [`HANDLERS`] 按序认领，未命中 no-op；命中则查 dedup 表
//! （key = 绝对路径字符串，与 `AssetNode.id` 同一身份体系）：
//! 已有 → 聚焦既有签，没有 → `add_panel_view(Center)` 尾插新签。
//!
//! console 恒为第一签：尾插天然保位 + `AgentTerminal::closable()=false`
//! （specs/center-tabs: Console is the permanent first center tab）。

pub mod editor_panel;
pub mod md_panel;

use std::sync::Arc;

use gpui_kit::base::dock::{DockArea, PanelId, PanelView};
use gpui_kit::component::dock::DockPlacement;
use gpui_kit::{Context, Window};

use crate::app::TerminalApp;
use crate::assets::AssetKind;
use editor_panel::RustViewerHandler;
use md_panel::MarkdownViewerHandler;

/// 待打开对象的中立封装——不含 gpui 类型，可跨插件边界
/// （design D2; `kind` 由 asset_manager 查视图给出）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenObject {
    /// 对象的绝对路径（去重 key = 其字符串形式）。
    pub path: std::path::PathBuf,
    /// 对象种类（Dir 在认领阶段被天然拒绝）。
    pub kind: AssetKind,
}

/// 打开处理器契约（dyn-safe, design D1）。
///
/// v1 静态注册表 [`HANDLERS`]；未来热插拔只换注册表构造方式。
pub trait OpenHandler: Send + Sync + 'static {
    /// 是否认领该对象（按序第一个 true 胜出）。
    fn claims(&self, obj: &OpenObject) -> bool;

    /// 为认领的对象创建面板（handler 负责自己的 `on_removed` 摘 dedup）。
    fn create_panel(
        &self,
        obj: &OpenObject,
        window: &mut Window,
        cx: &mut Context<TerminalApp>,
    ) -> Arc<dyn PanelView>;
}

/// 静态 handler 注册表（按序第一个认领者胜出; `.rs` 与 `.md` 只读查看器）。
static HANDLERS: &[&dyn OpenHandler] = &[&RustViewerHandler, &MarkdownViewerHandler];

/// 按注册顺序取第一个认领 `obj` 的 handler。
pub(crate) fn handler_for(obj: &OpenObject) -> Option<&'static dyn OpenHandler> {
    HANDLERS.iter().copied().find(|h| h.claims(obj))
}

/// 当前中心槽 active tab 的面板 id（焦点交回目标, design D6-3）。
///
/// 遍历 center 布局树取第一个 tab 组的 active；单 console 签时即 console。
pub fn active_center_panel_id(area: &DockArea) -> Option<PanelId> {
    let tree = area.layout(DockPlacement::Center)?;
    let mut active = None;
    tree.root().walk(&mut |node| {
        if active.is_none() {
            if let gpui_kit::base::dock::layout::PaneRef::Tabs { panels, active_ix } = node.kind()
            {
                active = panels.get(active_ix).copied();
            }
        }
    });
    active
}

#[cfg(test)]
mod tests {
    //! 无头测试 (center-slot-multi-tab tasks 1.x/2.1/3.x/5.x):
    //! 签序不变量 / dedup 生命周期 / 只读 viewer / 焦点键盘跟随 active tab。
    //! 显式导入, 不用 glob — 防 `gpui_kit::*` 把 `#[test]` 宏遮蔽掉
    //! (app.rs tests 同款教训)。

    use super::*;
    use crate::app::TerminalApp;
    use crate::sidebar::SidebarEvent;
    use crate::project_model::WorkspaceModel;
    use crate::workspace::Workspace;
    use gpui_kit::component::dock::BasePanel;
    use gpui_kit::component::dock::PanelHandle;
    use gpui_kit::test::{TestQueryExt, TestWindowExt};
    use gpui_kit::{
        AnyWindowHandle, AppContext, Bounds, Focusable, Point, TestAppContext, WindowBounds,
        WindowOptions, px, size,
    };
    use std::path::PathBuf;

    /// 起完整 app（隔离全局历史/外部配置, 模式同 tree tests）。
    fn open_app(
        cx: &mut TestAppContext,
    ) -> (AnyWindowHandle, gpui_kit::Entity<TerminalApp>) {
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

    fn render(handle: AnyWindowHandle, cx: &mut TestAppContext) {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
        })
        .expect("update window");
        cx.run_until_parked();
    }

    /// 中心 tab 组 (panels 预序, active_ix) — 签序断言的观测口。
    fn center_tabs(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
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

    fn console_id(cx: &mut TestAppContext, app: &gpui_kit::Entity<TerminalApp>) -> PanelId {
        cx.update(|cx| PanelId::from(app.read(cx).terminal.entity_id()))
    }

    /// 写 hermetic 临时文件（pid 隔离）, 返回绝对路径字符串。
    fn write_temp_file(tag: &str, ext: &str, content: &str) -> String {
        let path = std::env::temp_dir().join(format!("pdm-{tag}-{}.{ext}", std::process::id()));
        std::fs::write(&path, content).expect("写临时文件");
        path.to_string_lossy().into_owned()
    }

    /// 走统一后端开一个对象（真实调用链: update_window → app.update）。
    fn open_object_in(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        app: &gpui_kit::Entity<TerminalApp>,
        obj: OpenObject,
    ) {
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| app.open_object(obj, window, cx));
        })
        .expect("update window");
        cx.run_until_parked();
    }

    /// viewer 面板实体（dedup 表 → dock → 两跳 downcast: PanelHandle →
    /// 裸 Entity<EditorPanel>）。
    fn viewer_entity(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
        key: &str,
    ) -> gpui_kit::Entity<crate::property_detail_manager::editor_panel::EditorPanel> {
        cx.update(|cx| {
            let app = app.read(cx);
            let panel_id = *app.open_tabs.get(key).expect("dedup 表登记 viewer");
            let area = app.dock_area.read(cx);
            let handle = area.panel(panel_id).expect("viewer 在 dock 中");
            let wrapped = PanelHandle::of(handle).expect("panel_handle 包装");
            wrapped
                .panel()
                .as_any()
                .downcast_ref::<gpui_kit::Entity<crate::property_detail_manager::editor_panel::EditorPanel>>()
                .expect("inner 是 Entity<EditorPanel>")
                .clone()
        })
    }

    /// 关闭 viewer 签（库公开 API; 栈为 App → DockArea, on_removed 回摘
    /// dedup 时 TerminalApp 无 lease — 不得包在 app.update 内, 否则重入）。
    fn close_viewer(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        app: &gpui_kit::Entity<TerminalApp>,
        viewer: gpui_kit::Entity<crate::property_detail_manager::editor_panel::EditorPanel>,
    ) {
        cx.update_window(handle, |_, window, cx| {
            let area = app.read(cx).dock_area.clone();
            area.update(cx, |area, cx| area.remove_panel(viewer, window, cx));
        })
        .expect("update window");
        cx.run_until_parked();
    }



    /// 注入 workspace + 经服务创建计数会话为 active（task 3.2: 会话归
    /// AgentService，计数读共享测试会话 — `.get()` 读法不变）。
    fn inject_session(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
        tag: &str,
    ) -> (
        crate::service_layer::test_sessions::NumCounter,
        crate::service_layer::test_sessions::NumCounter,
        crate::service_layer::test_sessions::NumCounter,
        crate::service_layer::test_sessions::ClickCounter,
    ) {
        use crate::service_layer::test_sessions;
        let ws = std::env::temp_dir().join(format!("pdm-ws-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();
        app.update(cx, |app, cx| {
            app.workspace = Some(Workspace::new(ws.clone(), WorkspaceModel::default()));
            app.notify_panels(cx);
        });
        let (id, shared) = test_sessions::create_session(cx, app, &ws);
        let shared = shared.expect("计数测试会话 (非 real-pi 路径)");
        // 喂首帧 → 面板建本地 parser (grid 行/hover/点击/选区的在场前提)
        app.update(cx, |app, cx| {
            app.terminal.update(cx, |t, cx| {
                t.feed(id, b"probe\r\n", agent_service::frame_gen::current(), cx)
            });
        });
        cx.run_until_parked();
        test_sessions::counters(&shared)
    }

    #[gpui_kit::test]
    fn top_bar_always_visible(cx: &mut TestAppContext) {
        let (handle, _app) = open_app(cx);
        let _ = render(handle, cx);

        // 单 console 签 (方案 B): 顶栏常驻为 simple title 形态 (30px, 标题
        // console), 不是 tab 条; 多签时才切 tab 条 (Auto 样式, 用户选定 B,
        // 左右槽不受影响)。
        let (title_there, not_tab_strip) = cx
            .update_window(handle, |_, window, _cx| {
                // render_title 顶栏的专属锚点: 我们 title() 里的 console-title
                // (探针实证: tab-panel → tab → console-title)
                let title = window.elements().iter().any(|e| {
                    let path = format!("{:?}", e.path());
                    path.contains("dock-area-center") && path.contains("\"console-title\"")
                });
                let tab_strip = window.elements().iter().any(|e| {
                    let path = format!("{:?}", e.path());
                    path.contains("dock-area-center") && path.contains("\"tabs-inner\"")
                });
                (title, !tab_strip)
            })
            .expect("update window");
        assert!(title_there, "单 console 签时顶栏常驻 (render_title 顶栏在场)");
        assert!(not_tab_strip, "单签形态为 simple title 而非 tab 条 (Auto 样式)");
    }

    // ---------- 2.2 滚轮/点击分发 ----------
    // ---------- 3.2 滚轮链路 ----------
    // ---------- 4.1 Cmd+C 复制 ----------

    /// 拖选辅助: row0 中心 → row2 中心 (建立选区, on_click 定界建快照)。
    fn drag_select_rows(
        cx: &mut TestAppContext,
        handle: gpui_kit::AnyWindowHandle,
        from_row: usize,
        to_row: usize,
    ) {
        let (from, to) = cx
            .update_window(handle, |_, window, _| {
                let b0 = window.find(("grid-row", from_row)).bounds();
                let b2 = window.find(("grid-row", to_row)).bounds();
                (
                    gpui_kit::Point::new(
                        b0.origin.x + b0.size.width / 2.,
                        b0.origin.y + b0.size.height / 2.,
                    ),
                    gpui_kit::Point::new(
                        b2.origin.x + b2.size.width / 2.,
                        b2.origin.y + b2.size.height / 2.,
                    ),
                )
            })
            .expect("行 bounds");
        cx.update_window(handle, |_, window, cx| window.drag(from, to, cx))
            .expect("drag");
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn cmd_c_copies_selection_to_clipboard(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let (_keys, _texts, _wheels, _clicks) = inject_session(cx, &app, "copy");
        let _ = render(handle, cx);

        // 预置剪贴板标记 (隔离 + 可断言被覆盖)
        cx.update(|cx| {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("MARKER".to_string()))
        });

        // 拖选建立选区 (真实定界路径)
        drag_select_rows(cx, handle, 0, 2);
        let range = cx.update(|cx| app.read(cx).terminal.read(cx).selection);
        assert!(range.is_some(), "拖选建立选区");

        // Cmd+C → 剪贴板 == 实时选区文本 (spec: 复制内容实时读, 哨兵保证一致)
        cx.update_window(handle, |_, window, cx| {
            let mut scope = window.within("terminal-pane");
            scope.press("cmd-c", cx);
        })
        .expect("press cmd-c");
        cx.run_until_parked();

        let expected = cx.update(|cx| app.read(cx).terminal.read(cx).selection_text_now(cx));
        let clip = cx.update(|cx| cx.read_from_clipboard())
            .and_then(|i| i.text());
        assert_eq!(
            clip.as_deref(),
            expected.as_deref(),
            "剪贴板内容 == 实时选区文本 (spec: Copy selected text)"
        );
        assert_ne!(clip.as_deref(), Some("MARKER"), "标记被覆盖");
    }

    #[gpui_kit::test]
    fn cmd_c_without_selection_leaves_clipboard(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let (_keys, _texts, _wheels, _clicks) = inject_session(cx, &app, "copy-none");
        let _ = render(handle, cx);

        cx.update(|cx| {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("MARKER".to_string()))
        });
        // press 需要 scope 内观测焦点 (本测试无 drag, 手动聚焦终端面板)
        let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));
        cx.update_window(handle, |_, window, cx| {
            term_focus.clone().focus(window, cx);
        })
        .expect("focus");
        cx.run_until_parked();

        // 无选区按 Cmd+C → 剪贴板不变 (spec: SHALL NOT write to the clipboard)
        cx.update_window(handle, |_, window, cx| {
            let mut scope = window.within("terminal-pane");
            scope.press("cmd-c", cx);
        })
        .expect("press cmd-c");
        cx.run_until_parked();

        let clip = cx.update(|cx| cx.read_from_clipboard())
            .and_then(|i| i.text());
        assert_eq!(clip.as_deref(), Some("MARKER"), "无选区不动剪贴板");
    }

    // ---------- 3.1 快照哨兵 ----------

    #[gpui_kit::test]
    fn selection_survives_unrelated_frames(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let (_keys, _texts, _wheels, _clicks) = inject_session(cx, &app, "sentinel");
        let _ = render(handle, cx);

        // 拖选 (真实路径: on_click 定界 → 建立快照守卫)
        let (from, to) = cx
            .update_window(handle, |_, window, _| {
                let b0 = window.find(("grid-row", 0usize)).bounds();
                let b2 = window.find(("grid-row", 2usize)).bounds();
                (
                    gpui_kit::Point::new(
                        b0.origin.x + b0.size.width / 2.,
                        b0.origin.y + b0.size.height / 2.,
                    ),
                    gpui_kit::Point::new(
                        b2.origin.x + b2.size.width / 2.,
                        b2.origin.y + b2.size.height / 2.,
                    ),
                )
            })
            .expect("行 bounds");
        cx.update_window(handle, |_, window, cx| window.drag(from, to, cx))
            .expect("drag");
        cx.run_until_parked();
        let before = cx.update(|cx| app.read(cx).terminal.read(cx).selection);
        assert!(before.is_some(), "拖选建立选区 (守卫应已建立)");

        // 无关帧到达: 同内容重绘 (notify → render → 哨兵比对通过) → 保留
        // (spec: Selection survives unrelated frame updates)
        cx.update(|cx| {
            app.update(cx, |app, cx| app.notify_panels(cx));
        });
        let _ = render(handle, cx);
        let after = cx.update(|cx| app.read(cx).terminal.read(cx).selection);
        assert_eq!(
            after, before,
            "选区文本未变 → 无关帧不清 (spec: Unrelated output keeps the selection)"
        );
    }

    // ---------- 2.2 手势分离 ----------

    #[gpui_kit::test]
    fn drag_selection_sends_no_click_coordinate(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let (_keys, _texts, _wheels, clicks) = inject_session(cx, &app, "gesture");
        let _ = render(handle, cx);

        // 拖选 (down → move 越阈值 → up) → 不发 TUI 坐标 (spec)
        let (from, to) = cx
            .update_window(handle, |_, window, _| {
                let b0 = window.find(("grid-row", 0usize)).bounds();
                let b3 = window.find(("grid-row", 3usize)).bounds();
                (
                    gpui_kit::Point::new(
                        b0.origin.x + b0.size.width / 2.,
                        b0.origin.y + b0.size.height / 2.,
                    ),
                    gpui_kit::Point::new(
                        b3.origin.x + b3.size.width / 2.,
                        b3.origin.y + b3.size.height / 2.,
                    ),
                )
            })
            .expect("行 bounds");
        cx.update_window(handle, |_, window, cx| window.drag(from, to, cx))
            .expect("drag");
        cx.run_until_parked();
        assert_eq!(
            clicks.get(),
            None,
            "拖选手势不发坐标 (spec: Drag selection delivers no coordinate)"
        );
        let cf = cx.update(|cx| app.read(cx).click_forwards);
        assert_eq!(cf, 0, "入口零计数");

        // 纯点击 (无位移) → 维持现状转发
        cx.update_window(handle, |_, window, cx| {
            window.click("terminal-grid", cx);
        })
        .expect("click");
        cx.run_until_parked();
        assert!(
            clicks.get().is_some(),
            "纯点击仍转发坐标 (精确 cell 换算由 4.1 测试覆盖, 此处断言发生)"
        );
        let cf = cx.update(|cx| app.read(cx).click_forwards);
        assert_eq!(cf, 1, "入口 +1");
    }

    // ---------- 3.3 滚轮 hover 归属 ----------
    // ---------- 2.1 选区手势与 overlay ----------

    #[gpui_kit::test]
    fn drag_builds_selection_overlay(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let (_keys, _texts, _wheels, _clicks) = inject_session(cx, &app, "sel");
        let _ = render(handle, cx);

        // 拖动: row0 中心 → row3 中心 (跨 4 行, 距离 >> 3px 阈值)
        let (from, to) = cx
            .update_window(handle, |_, window, _| {
                let b0 = window.find(("grid-row", 0usize)).bounds();
                let b3 = window.find(("grid-row", 3usize)).bounds();
                (
                    gpui_kit::Point::new(
                        b0.origin.x + b0.size.width / 2.,
                        b0.origin.y + b0.size.height / 2.,
                    ),
                    gpui_kit::Point::new(
                        b3.origin.x + b3.size.width / 2.,
                        b3.origin.y + b3.size.height / 2.,
                    ),
                )
            })
            .expect("行 bounds");
        cx.update_window(handle, |_, window, cx| window.drag(from, to, cx))
            .expect("drag");
        cx.run_until_parked();

        // 状态: 选区建立且跨 row0..row3 (spec: 拖动扩展选区)
        let range = cx.update(|cx| app.read(cx).terminal.read(cx).selection);
        let range = range.expect("拖选建立选区");
        let (r0, r1) = (
            range.anchor.row.min(range.focus.row),
            range.anchor.row.max(range.focus.row),
        );
        assert_eq!((r0, r1), (0, 3), "选区跨 row0..row3 (anchor↔focus 规范化)");

        // overlay: 按行分 4 段矩形在场 (spec: overlay highlight per row)
        let rects = cx
            .update_window(handle, |_, window, _| {
                window
                    .elements()
                    .iter()
                    .filter(|e| format!("{:?}", e.path()).contains("sel-rect"))
                    .count()
            })
            .expect("elements");
        assert_eq!(rects, 4, "overlay 按行分 4 段");

        // 首段矩形贴行顶: top ≈ 0 (相对 grid), 高 = 行高
        let first = cx
            .update_window(handle, |_, window, _| {
                window
                    .elements()
                    .iter()
                    .find(|e| format!("{:?}", e.path()).contains("sel-rect"))
                    .map(|e| e.bounds())
            })
            .expect("elements");
        // 4 段按 path 序, 首段未必 row0 — 只断言高度一致为行高
        if let Some(b) = first {
            let line_h = cx
                .update_window(handle, |_, window, _| {
                    f32::from(agent_service::terminal::grid_metrics(window).1)
                })
                .expect("metrics");
            assert!((f32::from(b.size.height) - line_h).abs() < 1.0, "段高=行高");
        }
    }

    // ---------- 4.1 点击坐标转发 ----------

    #[gpui_kit::test]
    fn click_inside_grid_delivers_cell_coordinate(cx: &mut TestAppContext) {
        use gpui_kit::point;

        let (handle, app) = open_app(cx);
        let (_keys, _texts, _wheels, clicks) = inject_session(cx, &app, "clickcell");
        let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));
        let _ = render(handle, cx);

        let (cell_w, line_h) = cx
            .update_window(handle, |_, window, _| agent_service::terminal::grid_metrics(window))
            .expect("grid_metrics");
        // 点网格内第 10 列第 5 行的格子中心 → (10, 5)
        let target = point(
            gpui_kit::px(f32::from(cell_w) * 10.5),
            gpui_kit::px(f32::from(line_h) * 5.5),
        );
        cx.update_window(handle, |_, window, cx| {
            window.click_at("terminal-grid", target, cx);
        })
        .expect("click_at");
        cx.run_until_parked();

        assert_eq!(
            clicks.get(),
            Some((10, 5)),
            "网格内左键 → 0-based 单元格坐标 (窗口坐标 − grid origin ÷ cell 尺寸)"
        );
        let cf = cx.update(|cx| app.read(cx).click_forwards);
        assert_eq!(cf, 1, "入口计数");
        let focused = cx
            .update_window(handle, |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(
            focused,
            Some(term_focus),
            "点击仍一步聚焦 console pane (既有语义保持)"
        );
    }

    #[gpui_kit::test]
    fn click_outside_grid_sends_nothing(cx: &mut TestAppContext) {
        use gpui_kit::point;

        let (handle, app) = open_app(cx);
        let (_keys, _texts, _wheels, clicks) = inject_session(cx, &app, "clickout");
        let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));
        let _ = render(handle, cx);

        // 点 terminal-pane 顶部 (status bar 区, grid 上方) → 坐标为负 → 不发
        cx.update_window(handle, |_, window, cx| {
            window.click_at("terminal-pane", point(gpui_kit::px(5.), gpui_kit::px(5.)), cx);
        })
        .expect("click_at");
        cx.run_until_parked();

        assert_eq!(clicks.get(), None, "grid 外点击不发坐标 (spec)");
        let cf = cx.update(|cx| app.read(cx).click_forwards);
        assert_eq!(cf, 0, "入口零计数");
        let focused = cx
            .update_window(handle, |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(
            focused,
            Some(term_focus),
            "grid 外点击仍聚焦 pane (status bar 点击语义不变)"
        );
    }


    #[gpui_kit::test]
    fn wheel_unreachable_while_viewer_active(cx: &mut TestAppContext) {
        use gpui_kit::ScrollDelta;
        use gpui_kit::point;

        let (handle, app) = open_app(cx);
        let (_keys, _texts, wheels, _clicks) = inject_session(cx, &app, "wheelhover");
        let _ = render(handle, cx);

        // 开 viewer → console 面板不再渲染 (spec: Wheel while a viewer tab
        // is active reaches no session)
        let path = write_temp_file("hover", "rs", "fn hover_probe() {}");
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&path),
                kind: AssetKind::File,
            },
        );
        let _ = render(handle, cx);
        let console_gone = cx
            .update_window(handle, |_, window, _cx| window.try_find("terminal-grid").is_none())
            .expect("update window");
        assert!(console_gone, "viewer active → terminal-grid 不渲染 (handler 不存在)");

        // 滚 viewer 区域 (console 无 handler) → session 零收
        cx.update_window(handle, |_, window, cx| {
            window.scroll(
                "viewer-pane",
                ScrollDelta::Pixels(point(gpui_kit::px(0.), gpui_kit::px(-42.))),
                cx,
            );
        })
        .expect("scroll");
        cx.run_until_parked();

        assert_eq!(wheels.get(), 0, "viewer active 滚轮不进 session");
        let wf = cx.update(|cx| app.read(cx).wheel_forwards);
        assert_eq!(wf, 0, "入口零计数");
    }


    #[gpui_kit::test]
    fn wheel_accumulates_pixels_through_panel(cx: &mut TestAppContext) {
        use gpui_kit::ScrollDelta;
        use gpui_kit::point;

        let (handle, app) = open_app(cx);
        let (_keys, _texts, wheels, _clicks) = inject_session(cx, &app, "wheelchain");
        let _ = render(handle, cx);

        let line_h = cx
            .update_window(handle, |_, window, _| f32::from(agent_service::terminal::grid_metrics(window).1))
            .expect("grid_metrics");
        assert!(line_h > 0., "行高可测得");

        let scroll_px = |cx: &mut TestAppContext, dy: f32| {
            cx.update_window(handle, |_, window, cx| {
                window.scroll(
                    "terminal-grid",
                    ScrollDelta::Pixels(point(gpui_kit::px(0.), gpui_kit::px(dy))),
                    cx,
                );
            })
            .expect("scroll");
            cx.run_until_parked();
        };

        // 慢滚 gpui 正 delta 0.4 行 ×3 → 前两次无发送, 第三次发 -1 行
        // (符号翻转: gpui +y 上滑 → pi 负 = 上滚看更早, 2026-11-10 验收)
        scroll_px(cx, line_h * 0.4);
        scroll_px(cx, line_h * 0.4);
        assert_eq!(wheels.get(), 0, "未凑满一行不发送 (余数携带)");
        scroll_px(cx, line_h * 0.4);
        crate::service_layer::test_sessions::wait_until(cx, || wheels.get() == -1, "wheel(-1) 经服务送达");
        assert_eq!(wheels.get(), -1, "-1.2 行 → 发 -1 行 (负=看更早)");

        // 快滚一次 5 行 → 一次发 -5 (累积 -0.2-5=-5.2 → -5)
        scroll_px(cx, line_h * 5.);
        crate::service_layer::test_sessions::wait_until(cx, || wheels.get() == -6, "wheel 累计经服务送达");
        assert_eq!(wheels.get(), -6, "整数行不丢");

        let wf = cx.update(|cx| app.read(cx).wheel_forwards);
        assert_eq!(wf, 2, "入口计数 = 两次发送");
    }


    #[gpui_kit::test]
    fn grid_origin_tracks_terminal_grid_bounds(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let _ = inject_session(cx, &app, "origin");
        let _ = render(handle, cx); // render_frame + run_until_parked → defer 已 flush

        let (observed, stored) = cx
            .update_window(handle, |_, window, cx| {
                let bounds = window
                    .try_find("terminal-grid")
                    .expect("terminal-grid 应渲染 (有 active session)")
                    .bounds();
                let origin = app.read(cx).terminal.read(cx).grid_bounds.get().origin;
                (bounds.origin, origin)
            })
            .expect("update window");
        assert_eq!(
            observed, stored,
            "grid_origin 必须等于 terminal-grid 实际 bounds 原点 (D4 换算减项)"
        );
        assert!(
            observed.x > px(0.) && observed.y > px(0.),
            "原点非零 (状态条/内边距在网格上方左侧, 实测 {observed:?})"
        );
    }

    #[gpui_kit::test]
    fn wheel_and_click_reach_active_session(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let (_keys, _texts, wheels, clicks) = inject_session(cx, &app, "wheel");
        let _ = handle;

        // 转发: 载荷累计 + 入口计数 (task 2.2 观测口)
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                app.send_wheel(-3, cx);
                app.send_wheel(5, cx);
                app.send_click(12, 7, cx);
            });
        });
        crate::service_layer::test_sessions::wait_until(cx, || wheels.get() == 2 && clicks.get() == Some((12, 7)), "输入经服务送达");
        assert_eq!(wheels.get(), 2, "行数载荷累计 (-3+5)");
        assert_eq!(clicks.get(), Some((12, 7)), "坐标载荷原样送达");
        let (wf, cf) = cx.update(|cx| {
            let app = app.read(cx);
            (app.wheel_forwards, app.click_forwards)
        });
        assert_eq!((wf, cf), (2, 1), "入口计数 = 转发次数");

        // 无 active session: 不 panic、不计数、不转发 (task 2.2 边界)
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                app.active = None;
                app.send_wheel(9, cx);
                app.send_click(0, 0, cx);
            });
        });
        let wf = cx.update(|cx| app.read(cx).wheel_forwards);
        assert_eq!(wf, 2, "无会话不计数");
        assert_eq!(wheels.get(), 2, "无会话不转发");
        assert_eq!(clicks.get(), Some((12, 7)), "无会话坐标不变");
    }

    // ---------- 1.x dedup 生命周期 ----------

    #[gpui_kit::test]
    fn open_object_without_handler_is_noop(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let (before, active_before) = center_tabs(cx, &app);
        assert_eq!(before.len(), 1, "初始仅 console 签");
        assert_eq!(before[0], console_id(cx, &app), "console 恒 index 0");

        // 无 handler 认领: 非 rs/md 文件 + 目录 (md 现由 md viewer 认领)
        let unclaimed = write_temp_file("noop", "toml", "k = 1");
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&unclaimed),
                kind: AssetKind::File,
            },
        );
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from("/pdm-definitely-missing-dir"),
                kind: AssetKind::Dir,
            },
        );
        cx.run_until_parked();

        let (after, active_after) = center_tabs(cx, &app);
        assert_eq!(after, before, "未认领对象不建签 (spec: Unclaimed opens nothing)");
        assert_eq!(active_after, active_before, "active 不变");
        let _ = render(handle, cx);
    }

    #[gpui_kit::test]
    fn open_object_twice_yields_one_tab(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let path = write_temp_file("dedup", "rs", "fn dedup_probe() {}");
        let obj = || OpenObject {
            path: PathBuf::from(&path),
            kind: AssetKind::File,
        };

        open_object_in(cx, handle, &app, obj());
        let (tabs1, active1) = center_tabs(cx, &app);
        assert_eq!(tabs1.len(), 2, "console + viewer");
        assert_eq!(active1, 1, "新签尾插即 activate");

        // 切回 console, 再开同一对象 → 不新建, 而是聚焦既有 viewer
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.dock_area
                    .update(cx, |area, cx| area.select_panel(console_id_raw(app), window, cx));
            });
        })
        .expect("update window");
        cx.run_until_parked();
        let (_, active_console) = center_tabs(cx, &app);
        assert_eq!(active_console, 0, "已切回 console");

        open_object_in(cx, handle, &app, obj());
        let (tabs2, active2) = center_tabs(cx, &app);
        assert_eq!(tabs2.len(), 2, "重复对象不重开 (spec: Duplicate objects)");
        assert_eq!(active2, 1, "既有 viewer 被 select_panel 聚焦");
        let _ = render(handle, cx);
    }

    /// console 的 PanelId（在 TerminalApp 上下文外取, 供闭包内用）。
    fn console_id_raw(app: &TerminalApp) -> PanelId {
        PanelId::from(app.terminal.entity_id())
    }

    #[gpui_kit::test]
    fn closed_tab_can_be_reopened(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let path = write_temp_file("reopen", "rs", "fn reopen_probe() {}");
        let obj = || OpenObject {
            path: PathBuf::from(&path),
            kind: AssetKind::File,
        };

        open_object_in(cx, handle, &app, obj());
        assert_eq!(center_tabs(cx, &app).0.len(), 2, "开 viewer");
        let viewer = viewer_entity(cx, &app, &path);
        close_viewer(cx, handle, &app, viewer);
        let (after_close, _) = center_tabs(cx, &app);
        assert_eq!(after_close.len(), 1, "viewer 已关, 仅剩 console");
        assert!(
            cx.update(|cx| app.read(cx).open_tabs.is_empty()),
            "on_removed 必须摘 dedup (design D5)"
        );

        open_object_in(cx, handle, &app, obj());
        let (after_reopen, active) = center_tabs(cx, &app);
        assert_eq!(after_reopen.len(), 2, "摘表后同对象可重开 (spec: Reopen)");
        assert_eq!(active, 1, "重开的签 active");
        let _ = render(handle, cx);
    }

    // ---------- 2.1 console 第一签不变量 ----------

    #[gpui_kit::test]
    fn console_is_first_and_unclosable(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let path = write_temp_file("first", "rs", "fn first_probe() {}");
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&path),
                kind: AssetKind::File,
            },
        );

        let (tabs, _) = center_tabs(cx, &app);
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0], console_id(cx, &app), "console 恒第一签 (尾插保位)");
        assert!(
            !cx.update(|cx| app.read(cx).terminal.read(cx).closable(cx)),
            "console 不可关 (spec: NOT closable)"
        );
        let viewer = viewer_entity(cx, &app, &path);
        assert!(
            cx.update(|cx| viewer.read(cx).closable(cx)),
            "viewer 签可关"
        );

        // 关 viewer → 回落单 console 签
        close_viewer(cx, handle, &app, viewer);
        let (tabs_after, _) = center_tabs(cx, &app);
        assert_eq!(tabs_after, vec![console_id(cx, &app)], "关 viewer 后仅剩 console");
        let _ = render(handle, cx);
    }

    // ---------- 3.x 只读 viewer ----------

    #[gpui_kit::test]
    fn viewer_shows_content_readonly(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let path = write_temp_file("readonly", "rs", "fn readonly_probe() { let n = 1; }");
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&path),
                kind: AssetKind::File,
            },
        );

        let viewer = viewer_entity(cx, &app, &path);
        let (value, readonly, lang, cursor) = cx.update(|cx| {
            let state = viewer.read(cx).state.clone();
            let state = state.read(cx);
            (
                state.value().to_string(),
                !state.is_editable(),
                state.language_name(),
                state.cursor(),
            )
        });
        assert_eq!(
            cursor, 0,
            "打开即文件开头 (headed 验收修复: default_value, 非 replace 的尾部光标)"
        );
        assert!(
            value.contains("fn readonly_probe()"),
            "viewer 灌入文件内容: {value:?}"
        );
        assert!(readonly, "状态侧 readonly (design D4 双保险, !is_editable)");
        assert_eq!(lang, "rust", "language(\"rust\") 生效");

        // 键入不得改变 buffer（spec scenario: Viewer is read-only）
        let viewer_focus = cx.update(|cx| viewer.read(cx).focus_handle(cx));
        cx.update_window(handle, |_, window, cx| {
            viewer_focus.clone().focus(window, cx);
        })
        .expect("update window");
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let mut scope = window.within("viewer-pane");
            scope.input("HACKED", cx);
        })
        .expect("update window");
        cx.run_until_parked();

        let value_after = cx.update(|cx| viewer.read(cx).state.read(cx).value().to_string());
        assert_eq!(value_after, value, "readonly viewer 键入后内容不变");
        let _ = render(handle, cx);
    }

    #[gpui_kit::test]
    fn rs_file_opens_viewer_non_rs_noop(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let rs = write_temp_file("claim", "rs", "fn claim_probe() {}");
        let unclaimed = write_temp_file("claim", "toml", "k = 1");

        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&rs),
                kind: AssetKind::File,
            },
        );
        assert_eq!(center_tabs(cx, &app).0.len(), 2, ".rs → 开 viewer 签");
        let viewer_rs = viewer_entity(cx, &app, &rs);
        let title = cx.update(|cx| viewer_rs.read(cx).title.clone());
        let expected = std::path::Path::new(&rs)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(title.to_string(), expected, "tab 标题 = 文件名");

        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&unclaimed),
                kind: AssetKind::File,
            },
        );
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from("/pdm-claim-dir"),
                kind: AssetKind::Dir,
            },
        );
        assert_eq!(
            center_tabs(cx, &app).0.len(),
            2,
            ".toml / 目录不认领 → 无新签 (spec scenario: Non-viewer file has no viewer)"
        );
        let _ = render(handle, cx);
    }

    // ---------- md-viewer-toggle (1.3 / 2.x / 3.x / 4.x) ----------

    use crate::property_detail_manager::md_panel::{MdPanel, ViewMode};

    /// md 面板实体（dedup 表 → dock → 两跳 downcast, viewer_entity 同款）。
    fn md_panel_entity(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
        key: &str,
    ) -> gpui_kit::Entity<MdPanel> {
        cx.update(|cx| {
            let app = app.read(cx);
            let panel_id = *app.open_tabs.get(key).expect("dedup 表登记 md viewer");
            let area = app.dock_area.read(cx);
            let handle = area.panel(panel_id).expect("md viewer 在 dock 中");
            let wrapped = PanelHandle::of(handle).expect("panel_handle 包装");
            wrapped
                .panel()
                .as_any()
                .downcast_ref::<gpui_kit::Entity<MdPanel>>()
                .expect("inner 是 Entity<MdPanel>")
                .clone()
        })
    }

    fn open_md(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        app: &gpui_kit::Entity<TerminalApp>,
        path: &str,
    ) {
        open_object_in(
            cx,
            handle,
            app,
            OpenObject {
                path: PathBuf::from(path),
                kind: AssetKind::File,
            },
        );
    }

    /// 关闭 md 签（remove_panel; on_removed 摘 dedup — close_viewer 同款）。
    fn close_md(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        app: &gpui_kit::Entity<TerminalApp>,
        md: gpui_kit::Entity<MdPanel>,
    ) {
        cx.update_window(handle, |_, window, cx| {
            let area = app.read(cx).dock_area.clone();
            area.update(cx, |area, cx| area.remove_panel(md, window, cx));
        })
        .expect("update window");
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn md_opens_rendered_with_dual_states(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let md = write_temp_file("md-open", "md", "# 标题\n\n正文 `code`");
        open_md(cx, handle, &app, &md);

        assert_eq!(center_tabs(cx, &app).0.len(), 2, ".md → 开 md viewer 签 (2.1)");
        let panel = md_panel_entity(cx, &app, &md);
        let (mode, title, has_md, has_editor) = cx.update(|cx| {
            let p = panel.read(cx);
            (
                p.view_mode,
                p.title.clone(),
                p.md_state.entity_id(),
                p.editor.entity_id(),
            )
        });
        assert_eq!(mode, ViewMode::Rendered, "默认渲染视图 (1.3, proposal 定案)");
        let expected = std::path::Path::new(&md)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(title.to_string(), expected, "tab 标题 = 文件名");
        // 双态 state 构造期都在 (D3): entity_id 稳定即两态在场
        assert_ne!(has_md, has_editor, "两态是不同实体 (1.3)");
        let _ = render(handle, cx);
    }

    #[gpui_kit::test]
    fn md_dedup_close_reopen_resets_to_rendered(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let md = write_temp_file("md-dedup", "md", "# dedup");
        open_md(cx, handle, &app, &md);
        open_md(cx, handle, &app, &md);
        assert_eq!(
            center_tabs(cx, &app).0.len(),
            2,
            "同一 md 开两次仍单签 (2.2 dedup)"
        );

        // 切到 source (按钮点击, 端到端)
        let panel = md_panel_entity(cx, &app, &md);
        let _ = render(handle, cx);
        cx.update_window(handle, |_, window, cx| window.click("md-toggle", cx))
            .expect("click toggle");
        cx.run_until_parked();
        let mode = cx.update(|cx| panel.read(cx).view_mode);
        assert_eq!(mode, ViewMode::Source, "按钮切换到源码 (3.2)");

        // 关签 → 摘 dedup → 重开 → 回渲染 (4.2 不记忆)
        close_md(cx, handle, &app, panel);
        assert!(
            cx.update(|cx| !app.read(cx).open_tabs.contains_key(&md)),
            "关签摘 dedup (2.2)"
        );
        open_md(cx, handle, &app, &md);
        let reopened = md_panel_entity(cx, &app, &md);
        let mode = cx.update(|cx| reopened.read(cx).view_mode);
        assert_eq!(
            mode,
            ViewMode::Rendered,
            "重开回渲染视图 (spec: Reopen resets to rendered)"
        );
        let _ = render(handle, cx);
    }

    #[gpui_kit::test]
    fn md_toggle_action_dispatches_in_place(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let md = write_temp_file("md-action", "md", "# action");
        open_md(cx, handle, &app, &md);
        let panel = md_panel_entity(cx, &app, &md);
        let _ = render(handle, cx);

        // action 路径 (3.1): on_action 沿焦点路径冒泡 → 先聚焦面板再 dispatch
        let focus = cx.update(|cx| panel.read(cx).focus_handle(cx));
        cx.update_window(handle, |_, window, cx| {
            focus.clone().focus(window, cx);
        })
        .expect("focus");
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let mut scope = window.within("md-pane");
            scope.press("cmd-shift-m", cx);
        })
        .expect("press");
        cx.run_until_parked();
        let mode = cx.update(|cx| panel.read(cx).view_mode);
        assert_eq!(mode, ViewMode::Source, "action 切到源码 (3.1)");
        assert_eq!(
            center_tabs(cx, &app).0.len(),
            2,
            "切换不重建 tab (spec: in place, 无第二签)"
        );

        // 按钮路径同 body: 再点回渲染
        cx.update_window(handle, |_, window, cx| window.click("md-toggle", cx))
            .expect("click");
        cx.run_until_parked();
        let mode = cx.update(|cx| panel.read(cx).view_mode);
        assert_eq!(mode, ViewMode::Rendered, "按钮切回渲染 (3.2)");
        let _ = render(handle, cx);
    }

    #[gpui_kit::test]
    fn md_toggle_absent_on_console_and_rs(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        // console 恒为首签且默认 active (spec: permanent first tab)
        let _ = render(handle, cx);
        let has_toggle = cx.update_window(handle, |_, window, _| {
            window.try_find("md-toggle").is_some()
        })
        .expect("find");
        assert!(!has_toggle, "console 无切换按钮 (spec: no toggle)");

        // 开 rs viewer (激活该签): 同样无
        let rs = write_temp_file("md-none", "rs", "fn none() {}");
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&rs),
                kind: AssetKind::File,
            },
        );
        let _ = render(handle, cx);
        let has_toggle = cx.update_window(handle, |_, window, _| {
            window.try_find("md-toggle").is_some()
        })
        .expect("find");
        assert!(!has_toggle, "rs viewer 无切换按钮 (spec: no toggle)");
    }

    #[gpui_kit::test]
    fn md_readonly_source_mode(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let md = write_temp_file("md-ro", "md", "# readonly probe");
        open_md(cx, handle, &app, &md);
        let panel = md_panel_entity(cx, &app, &md);
        let value = cx.update(|cx| panel.read(cx).editor.read(cx).value().to_string());
        assert!(value.contains("# readonly probe"), "source 灌入: {value:?}");

        // 切到 source (按钮), 键入不改内容 (spec: Viewer is read-only)
        let _ = render(handle, cx);
        cx.update_window(handle, |_, window, cx| window.click("md-toggle", cx))
            .expect("click");
        cx.run_until_parked();
        let focus = cx.update(|cx| panel.read(cx).focus_handle(cx));
        cx.update_window(handle, |_, window, cx| {
            focus.clone().focus(window, cx);
        })
        .expect("focus");
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let mut scope = window.within("md-source-pane");
            scope.input("HACKED", cx);
        })
        .expect("input");
        cx.run_until_parked();
        let after = cx.update(|cx| panel.read(cx).editor.read(cx).value().to_string());
        assert_eq!(after, value, "source 模式键入后内容不变 (4.1)");
        let _ = render(handle, cx);
    }

    #[gpui_kit::test]
    fn md_rendered_view_renders_format(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let md = write_temp_file("md-render", "md", "# 大标题\n\n- 项目一");
        open_md(cx, handle, &app, &md);
        let _ = render(handle, cx);

        // 默认渲染: md-pane 在树, source pane 不在 (spec: opens rendered)
        let (in_rendered, in_source) = cx.update_window(handle, |_, window, _| {
            (window.try_find("md-pane").is_some(), window.try_find("md-source-pane").is_some())
        })
        .expect("find");
        assert!(in_rendered, "默认渲染视图元素在场 (4.1)");
        assert!(!in_source, "源码元素未挂载 (4.1)");

        // 切 source: 反转
        let panel = md_panel_entity(cx, &app, &md);
        cx.update_window(handle, |_, window, cx| window.click("md-toggle", cx))
            .expect("click");
        cx.run_until_parked();
        let _ = render(handle, cx);
        let (in_rendered, in_source) = cx.update_window(handle, |_, window, _| {
            (window.try_find("md-pane").is_some(), window.try_find("md-source-pane").is_some())
        })
        .expect("find");
        assert!(!in_rendered && in_source, "切换后源码视图元素在场 (3.2/4.1)");
        let _ = panel;
        let _ = render(handle, cx);
    }

    // ---------- 5.x 焦点/键盘跟随 active tab ----------

    #[gpui_kit::test]
    fn sidebar_select_returns_to_console_tab(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let path = write_temp_file("sidebar", "rs", "fn sidebar_probe() {}");
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&path),
                kind: AssetKind::File,
            },
        );
        let (tabs, active) = center_tabs(cx, &app);
        assert_eq!(active, 1, "viewer active");

        // 注入 workspace + 经服务建会话 (task 3.2: select ask, 成功才切 console)
        let ws = std::env::temp_dir().join(format!("pdm-sel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();
        app.update(cx, |app, _cx| {
            app.workspace = Some(Workspace::new(ws.clone(), WorkspaceModel::default()));
        });
        let (agent_id, _shared) =
            crate::service_layer::test_sessions::create_session(cx, &app, &ws);

        // 点 sidebar agent 行 → ask 经服务 → 成功回执经泵切 console + 焦点
        let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.on_sidebar_event(SidebarEvent::SelectAgent(agent_id), window, cx);
            });
        })
        .expect("update window");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            cx.run_until_parked();
            app.update(cx, |app, cx| {
                app.poll_sessions(cx);
            });
            let (t, a) = center_tabs(cx, &app);
            if a == 0 {
                assert_eq!(t, tabs, "签不增减");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "select 成功回执未切 console"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let (tabs_after, active_after) = center_tabs(cx, &app);
        assert_eq!(tabs_after, tabs, "签不增减");
        assert_eq!(active_after, 0, "切回 console 签 (spec: sidebar select)");
        let focused = cx
            .update_window(handle, |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(focused, Some(term_focus), "一步聚焦: 焦点在 AgentTerminal");
        let _ = render(handle, cx);
    }

    #[gpui_kit::test]
    fn keystrokes_follow_active_tab(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let (keys, texts, _wheels, _clicks) = inject_session(cx, &app, "keys");
        let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));
        let _ = render(handle, cx);

        // console active: 按键 → session 收键
        cx.update_window(handle, |_, window, cx| {
            term_focus.clone().focus(window, cx);
        })
        .expect("update window");
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            let mut scope = window.within("terminal-pane");
            scope.press("a", cx);
        })
        .expect("update window");
        cx.run_until_parked();
        crate::service_layer::test_sessions::wait_until(cx, || keys.get() == 1, "按键经服务送达");
        assert_eq!(keys.get(), 1, "console active → session 收键");

        // 开 viewer → viewer active: 按键不进 session
        let path = write_temp_file("keys", "rs", "fn keys_probe() {}");
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&path),
                kind: AssetKind::File,
            },
        );
        let viewer = viewer_entity(cx, &app, &path);
        let viewer_focus = cx.update(|cx| viewer.read(cx).focus_handle(cx));
        let focused = cx
            .update_window(handle, |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(
            focused,
            Some(viewer_focus.clone()),
            "开签 activate → 焦点随 active tab 到 viewer (open_object 显式交焦)"
        );

        cx.update_window(handle, |_, window, cx| {
            let mut scope = window.within("viewer-pane");
            scope.press("b", cx);
        })
        .expect("update window");
        cx.run_until_parked();
        assert_eq!(
            keys.get(),
            1,
            "viewer active → 按键不进 session (spec: Keystrokes follow the active tab)"
        );
        assert_eq!(texts.get(), 0, "无 IME commit 进 session");
        let focused_after = cx
            .update_window(handle, |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(focused_after, Some(viewer_focus), "viewer 焦点保持");
        let _ = render(handle, cx);
    }

    #[gpui_kit::test]
    fn ime_commit_follows_active_tab(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let (keys, texts, _wheels, _clicks) = inject_session(cx, &app, "ime");
        let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));
        let _ = render(handle, cx);

        // console active: IME 挂载点 (terminal-grid canvas → ElementInputHandler
        // 挂 terminal focus) 在场, 焦点在 terminal → commit 路由到 session
        cx.update_window(handle, |_, window, cx| {
            term_focus.clone().focus(window, cx);
        })
        .expect("update window");
        cx.run_until_parked();
        let (grid_there, focused_console) = cx
            .update_window(handle, |_, window, cx| {
                let grid = window.try_find("terminal-grid").is_some();
                let focused = window.focused(cx);
                (grid, focused)
            })
            .expect("update window");
        assert!(grid_there, "console active → terminal-grid 在场");
        assert_eq!(focused_console, Some(term_focus), "IME 焦点在 terminal");

        // viewer active: console 不渲染 → IME 挂载点消失, 焦点在 viewer →
        // 平台 commit 无从路由到 session (spec: 失焦/tab 跟随分支)
        let path = write_temp_file("ime", "rs", "fn ime_probe() {}");
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&path),
                kind: AssetKind::File,
            },
        );
        let viewer = viewer_entity(cx, &app, &path);
        let viewer_focus = cx.update(|cx| viewer.read(cx).focus_handle(cx));
        let (viewer_there, grid_gone, focused_viewer) = cx
            .update_window(handle, |_, window, cx| {
                let viewer = window.try_find("viewer-pane").is_some();
                let grid = window.try_find("terminal-grid").is_none();
                let focused = window.focused(cx);
                (viewer, grid, focused)
            })
            .expect("update window");
        assert!(viewer_there, "viewer 签渲染");
        assert!(
            grid_gone,
            "viewer active → console 面板不渲染, IME 挂载点不在 (spec: 挂载点在 console 焦点上)"
        );
        assert_eq!(focused_viewer, Some(viewer_focus), "焦点在 viewer");
        assert_eq!(texts.get(), 0, "session 未收到任何 IME commit");
        assert_eq!(keys.get(), 0, "session 未收到按键");
        let _ = render(handle, cx);
    }
}
