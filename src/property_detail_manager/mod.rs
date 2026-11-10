//! 默认打开系统（change: center-slot-multi-tab, design D1）。
//!
//! 统一后端接口：任意槽（当前=右槽文件树）把**中立封装对象**
//! [`OpenObject`] 交给 [`TerminalApp::open_object`]，由静态 handler
//! 注册表 [`HANDLERS`] 按序认领，未命中 no-op；命中则查 dedup 表
//! （key = 绝对路径字符串，与 `SourceNode.id` 同一身份体系）：
//! 已有 → 聚焦既有签，没有 → `add_panel_view(Center)` 尾插新签。
//!
//! console 恒为第一签：尾插天然保位 + `TerminalPanel::closable()=false`
//! （specs/center-tabs: Console is the permanent first center tab）。

pub mod editor_panel;

use std::sync::Arc;

use gpui_kit::base::dock::{DockArea, PanelId, PanelView};
use gpui_kit::component::dock::DockPlacement;
use gpui_kit::{Context, Window};

use crate::app::TerminalApp;
use crate::workspace_property_source::SourceNodeKind;
use editor_panel::RustViewerHandler;

/// 待打开对象的中立封装——不含 gpui 类型，可跨插件边界
/// （design D2; `kind` 由数据源查权威树给出）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenObject {
    /// 对象的绝对路径（去重 key = 其字符串形式）。
    pub path: std::path::PathBuf,
    /// 对象种类（Dir 在认领阶段被天然拒绝）。
    pub kind: SourceNodeKind,
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

/// 静态 handler 注册表（v1 仅 `.rs` 只读查看器）。
static HANDLERS: &[&dyn OpenHandler] = &[&RustViewerHandler];

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
    use crate::agent_entities::{AgentEntity, AgentSession, SessionStatus};
    use crate::app::TerminalApp;
    use crate::sidebar::SidebarEvent;
    use crate::workspace::Workspace;
    use gpui_kit::component::dock::BasePanel;
    use gpui_kit::component::dock::PanelHandle;
    use gpui_kit::test::{TestQueryExt, TestWindowExt};
    use gpui_kit::{
        AnyWindowHandle, AppContext, Bounds, Focusable, Point, TestAppContext, WindowBounds,
        WindowOptions, px, size,
    };
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::rc::Rc;

    /// 起完整 app（隔离 `.workspaces.json`, 模式同 tree tests）。
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
                |window, cx| cx.new(|cx| TerminalApp::new(window, cx).expect(".workspaces.json")),
            )
            .expect("open test window")
        });
        app.update(cx, |app, _| app.workspaces.clear());
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

    /// 计数假会话: 记录 send_key / send_text 次数（5.2/5.3 观测口）。
    struct CountingAgent {
        name: String,
        ws: PathBuf,
        parser: vt100::Parser,
        keys: Rc<Cell<usize>>,
        texts: Rc<Cell<usize>>,
    }

    impl CountingAgent {
        fn new(name: &str, ws: &Path, keys: Rc<Cell<usize>>, texts: Rc<Cell<usize>>) -> Self {
            Self {
                name: name.to_string(),
                ws: ws.to_path_buf(),
                parser: vt100::Parser::new(
                    std::num::NonZeroU16::new(24).unwrap(),
                    std::num::NonZeroU16::new(80).unwrap(),
                    0,
                ),
                keys,
                texts,
            }
        }
    }

    impl AgentEntity for CountingAgent {
        fn name(&self) -> &str {
            &self.name
        }
        fn ws_path(&self) -> &Path {
            &self.ws
        }
        fn screen(&self) -> &vt100::Screen {
            self.parser.screen()
        }
        fn poll(&mut self) -> bool {
            false
        }
        fn send_key(&mut self, _key: crossterm::event::KeyEvent) {
            self.keys.set(self.keys.get() + 1);
        }
        fn send_text(&mut self, text: &str) {
            let _ = text;
            self.texts.set(self.texts.get() + 1);
        }
        fn shutdown(&mut self) {}
        fn status(&self) -> SessionStatus {
            SessionStatus::Running
        }
        fn resize(&mut self, _cols: u16, _rows: u16) -> bool {
            false
        }
    }

    use std::path::Path;

    /// 注入 workspace + 计数会话为 active。
    fn inject_session(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
        tag: &str,
    ) -> (Rc<Cell<usize>>, Rc<Cell<usize>>) {
        let ws = std::env::temp_dir().join(format!("pdm-ws-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(&ws).unwrap();
        let keys = Rc::new(Cell::new(0usize));
        let texts = Rc::new(Cell::new(0usize));
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                app.workspaces.push(Workspace::new(ws.clone()));
                let agent = cx.new(|_| {
                    AgentSession(Box::new(CountingAgent::new(
                        "pi-1",
                        &ws,
                        keys.clone(),
                        texts.clone(),
                    )))
                });
                let id = agent.entity_id();
                app.workspaces[0].agents.push(agent);
                app.active = Some(id);
                app.notify_panels(cx);
            });
        });
        cx.run_until_parked();
        (keys, texts)
    }

    // ---------- 2.2 tab 条常驻 ----------

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

    // ---------- 1.x dedup 生命周期 ----------

    #[gpui_kit::test]
    fn open_object_without_handler_is_noop(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let (before, active_before) = center_tabs(cx, &app);
        assert_eq!(before.len(), 1, "初始仅 console 签");
        assert_eq!(before[0], console_id(cx, &app), "console 恒 index 0");

        // 无 handler 认领: .md 文件 + 目录
        let md = write_temp_file("noop", "md", "# 无 handler");
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&md),
                kind: SourceNodeKind::File,
            },
        );
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from("/pdm-definitely-missing-dir"),
                kind: SourceNodeKind::Dir,
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
            kind: SourceNodeKind::File,
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
            kind: SourceNodeKind::File,
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
                kind: SourceNodeKind::File,
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
                kind: SourceNodeKind::File,
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
        let md = write_temp_file("claim", "md", "# no claim");

        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&rs),
                kind: SourceNodeKind::File,
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
                path: PathBuf::from(&md),
                kind: SourceNodeKind::File,
            },
        );
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from("/pdm-claim-dir"),
                kind: SourceNodeKind::Dir,
            },
        );
        assert_eq!(
            center_tabs(cx, &app).0.len(),
            2,
            ".md / 目录不认领 → 无新签 (spec scenario: Non-rs file has no viewer)"
        );
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
                kind: SourceNodeKind::File,
            },
        );
        let (tabs, active) = center_tabs(cx, &app);
        assert_eq!(active, 1, "viewer active");

        // 点 sidebar agent 行 → 一步回 console 签 + session 焦点
        let agent_id = cx.update(|cx| cx.new(|_| 0u8).entity_id());
        let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));
        cx.update_window(handle, |_, window, cx| {
            app.update(cx, |app, cx| {
                app.on_sidebar_event(SidebarEvent::SelectAgent(agent_id), window, cx);
            });
        })
        .expect("update window");
        cx.run_until_parked();

        let (tabs_after, active_after) = center_tabs(cx, &app);
        assert_eq!(tabs_after, tabs, "签不增减");
        assert_eq!(active_after, 0, "切回 console 签 (spec: sidebar select)");
        let focused = cx
            .update_window(handle, |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(focused, Some(term_focus), "一步聚焦: 焦点在 TerminalPanel");
        let _ = render(handle, cx);
    }

    #[gpui_kit::test]
    fn keystrokes_follow_active_tab(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let (keys, texts) = inject_session(cx, &app, "keys");
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
        assert_eq!(keys.get(), 1, "console active → session 收键");

        // 开 viewer → viewer active: 按键不进 session
        let path = write_temp_file("keys", "rs", "fn keys_probe() {}");
        open_object_in(
            cx,
            handle,
            &app,
            OpenObject {
                path: PathBuf::from(&path),
                kind: SourceNodeKind::File,
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
        let (keys, texts) = inject_session(cx, &app, "ime");
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
                kind: SourceNodeKind::File,
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
