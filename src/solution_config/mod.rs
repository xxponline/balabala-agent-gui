//! solution-config-tab (change tasks 1.x/2.x): 中槽 SolutionConfig 表单签。
//!
//! specs/center-tabs delta: SolutionConfig tab opens through the add-solution
//! entry — 左槽 add 入口开**应用内签**（不再弹系统 picker），dedup 身份 =
//! 固定 key，独立于文件 path 与 debug section id；specs/project-model delta:
//! Solution creation via the SolutionConfig tab — path 输入 + 三预检
//! （空 / 重名 / 合法=存在且为目录）门控提交，提交成功自关、未提交关闭零
//! 写入。
//!
//! 提交仍走 solution-service `Create` ask（app 缝 `request_create_solution`,
//! design D3）；成功回执由 `poll_sessions` 消费并 **defer 关签** —
//! 直接嵌套 `remove_panel` 会在 `on_removed` 里重入 TerminalApp，撞 gpui
//! `double_lease_panic`（cannot update … while it is already being updated）；
//! `App::defer` 效果只在外层 update 结束后 flush，届时租约已释放。

use std::path::{Path, PathBuf};

use gpui_kit::base::dock::{PanelEvent, PanelId};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{
    BasePanel, DockPlacement, Panel as ComponentPanel, panel_handle,
};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::base::TestTextExt;
use gpui_kit::{
    App, AppContext, AsyncApp, ClickEvent, Context, Entity, EventEmitter, Focusable, FocusHandle,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, TestSupportExt, WeakEntity, Window, div, px,
};

use crate::app::TerminalApp;

type ClickListener = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// dedup key（固定身份 — 与文件 path / section id 双双独立;
/// specs/center-tabs: Re-entry selects the single tab）。
pub(crate) const SOLUTION_CONFIG_TAB_KEY: &str = "solution-config";

/// 三预检（design D2; specs/project-model: Failed precheck blocks submission）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Prechecks {
    /// 非空。
    pub not_empty: bool,
    /// 不与既有 solution item / project root 重名（幂等预检与 Server 同源
    /// `create_is_noop`, design D2 — design D6 同一 ValidChecker）。
    pub unique: bool,
    /// 合法路径 = 存在且为目录（打字输入若不存在, 下次装载会触发
    /// `LoadError::MissingPath` 阻断开区 — spec: Solution load failure…）。
    /// **无 workspace 位置约束**（决策 2026-11-23: root 只需合法）。
    pub existing_dir: bool,
}

impl Prechecks {
    pub(crate) fn all_pass(self) -> bool {
        self.not_empty && self.unique && self.existing_dir
    }

    pub(crate) fn compute(path: &str, tree: Option<&solution_service::TreeSnapshot>) -> Self {
        let path = path.trim();
        let not_empty = !path.is_empty();
        let p = Path::new(path);
        let unique = not_empty
            && !tree.is_some_and(|t| {
                solution_service::create_is_noop(&t.solutions, p)
                    || t.tree.projects.iter().any(|proj| proj.root == p)
            });
        let existing_dir = not_empty && std::fs::metadata(p).is_ok_and(|m| m.is_dir());
        Self {
            not_empty,
            unique,
            existing_dir,
        }
    }
}

/// SolutionConfig 表单签实体（PanelView 形态对齐 debug section 签）。
pub struct SolutionConfigPanel {
    /// path 输入（焦点即签焦点 — spec: Keystrokes follow the active tab）。
    pub(crate) input: Entity<InputState>,
    /// tab 标题（`title()` 直读, 测试经此断言）。
    pub(crate) title: SharedString,
    /// 弱引用 app：读树快照预检 + 提交走 app 缝。
    app: WeakEntity<TerminalApp>,
    /// 自身面板身份（= entity_id 派生）, 供摘表比对。
    panel_id: PanelId,
    /// 保持对 input 的观察订阅（编辑 → 签重绘 → 预检重算; 丢弃即失联）。
    _input_sub: Subscription,
    /// 在途 browse 对话框（面板存续期内保活, drop 即取消）。
    _browse_task: Option<Task<()>>,
}

impl SolutionConfigPanel {
    pub fn new(
        app: WeakEntity<TerminalApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let watched = input.clone();
        let _input_sub = cx.observe(&watched, |_, _, cx| cx.notify());
        Self {
            input,
            title: SOLUTION_CONFIG_TAB_KEY.into(),
            app,
            panel_id: PanelId::from(cx.entity_id()),
            _input_sub,
            _browse_task: None,
        }
    }

    /// 当前输入（trim 后口径与预检一致）。
    pub(crate) fn path_value(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string().trim().to_string()
    }

    /// 三预检（每帧读树快照 — mirror 单份真值, view 不留副本）。
    pub(crate) fn prechecks(&self, cx: &App) -> Prechecks {
        let tree = self
            .app
            .upgrade()
            .and_then(|app| app.read(cx).service_layer.tree_view().cloned());
        Prechecks::compute(&self.path_value(cx), tree.as_ref())
    }

    /// 提交门控（spec: enable submission only while they all pass）。
    pub(crate) fn can_submit(&self, cx: &App) -> bool {
        self.prechecks(cx).all_pass()
    }

    /// 填入 path（测试缝: update_window 内直调; browse 回填走 with_window
    /// 变体 — Context<T> 不是 VisualContext, 用不了 update_in）。
    #[allow(dead_code)] // 非测构建不触碰 — panel_tests 专用缝
    pub(crate) fn set_path(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |state, cx| state.set_value(path, window, cx));
        cx.notify();
    }

    /// 提交（click 监听器与测试共用缝; 走 app 缝 → Create ask, design D3）。
    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.can_submit(cx) {
            return; // spec: submission is disabled … failing precheck shown
        }
        let raw = self.path_value(cx);
        // canonicalize 与旧 picker 流一致（存在性已由预检把门, 失败兜底原值）。
        let path = std::fs::canonicalize(&raw).unwrap_or_else(|_| PathBuf::from(raw));
        let app = self.app.clone();
        app.update(cx, |app, cx| app.request_create_solution(path, cx))
            .ok();
    }
}

impl BasePanel for SolutionConfigPanel {
    fn panel_name(&self) -> &'static str {
        "solution-config"
    }
    fn closable(&self, _: &App) -> bool {
        true // specs/center-tabs delta: 表单签可关
    }
    /// 签被关闭（用户 × 或成功自关的 remove 落地）→ 摘 dedup（open_tabs 按
    /// panel_id retain）+ 清面板句柄, 同签可重开 fresh（specs/center-tabs:
    /// Closing the form reopens fresh）。
    fn on_removed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let app = self.app.clone();
        let panel_id = self.panel_id;
        app.update(cx, |app, _| {
            app.open_tab_removed(panel_id);
            if app
                .sol_config_panel
                .as_ref()
                .is_some_and(|p| PanelId::from(p.entity_id()) == panel_id)
            {
                app.sol_config_panel = None;
            }
        })
        .ok();
    }
}

impl ComponentPanel for SolutionConfigPanel {
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().text_sm().child(self.title.clone())
    }
    fn title_bar(&self, _: &App) -> bool {
        false // 与 viewer/debug 同款 chrome 策略
    }
    fn inner_padding(&self, _: &App) -> bool {
        false // 表单自控布局
    }
}

impl EventEmitter<PanelEvent> for SolutionConfigPanel {}

impl Focusable for SolutionConfigPanel {
    /// 焦点 = 输入框焦点（签切换交焦即落进 path 输入 — 键盘/IME 随 active
    /// tab 走）。
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Render for SolutionConfigPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_muted = cx.theme().muted_foreground;
        let theme_danger = cx.theme().danger;
        let theme_hover = cx.theme().list_hover;
        let pre = self.prechecks(cx);
        let blocked = !pre.all_pass();
        let status = |ok: bool| if ok { "ok" } else { "fail" };

        // browse: rfd 只填输入, 不直连提交（旧 add_solution 直连链已退役,
        // task 2.2）。取消 picker → 早退, 输入与文件均不变（specs/project-model:
        // Cancelled picker writes nothing — headed 冒烟覆盖, task 4.2）。
        let on_browse: ClickListener = Box::new(cx.listener(move |this, _ev, _window, cx| {
            let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
                let Some(handle) = rfd::AsyncFileDialog::new().pick_folder().await else {
                    return; // 取消: 不改输入
                };
                let picked = handle.path().to_path_buf();
                let path = std::fs::canonicalize(&picked).unwrap_or(picked);
                let text = path.to_string_lossy().into_owned();
                this.update(cx, |this, cx| {
                    let panel_id = cx.entity().entity_id();
                    let input = this.input.clone();
                    // update_in 需 VisualContext (AsyncWindow 等专用上下文),
                    // Context<T> 走 with_window 拿 window + &mut App 直改
                    cx.with_window(panel_id, move |window, cx| {
                        input.update(cx, |state, cx| state.set_value(text, window, cx));
                    });
                    cx.notify();
                })
                .ok();
            });
            this._browse_task = Some(task);
        }));

        let on_submit: ClickListener = Box::new(cx.listener(move |this, _ev, _window, cx| {
            this.submit(cx);
        }));

        let row = |label: &str, ok: bool| -> String {
            format!("{label}: {}", status(ok))
        };
        // child 才是绘制, test_text 只是记录 (texts() 只收录 test_text) —
        // 双挂: 真机有字 + 测试可断言 (sidebar 同款; 只挂 test_text 会
        // 变空壳: 2026-11-23 headed 冒烟实测按钮无字)
        let row_empty = row("path not empty", pre.not_empty);
        let row_uniq = row("unique solution", pre.unique);
        let row_dir = row("existing directory", pre.existing_dir);

        div()
            .id("solution-config-pane")
            .test_support()
            .size_full()
            .bg(cx.theme().background)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_6()
                    .p_4()
                    // 左栏: path 输入 + browse
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .w(px(280.))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme_muted)
                                    // texts() 只收录 test_text (pane/tree 同款观测口);
                                    // child 绘制 + test_text 记录 双挂
                                    .child(
                                        div()
                                            .child("project path")
                                            .test_text("project path"),
                                    ),
                            )
                            .child(Input::new(&self.input).w_full())
                            .child(
                                div()
                                    .id("solution-config-browse")
                                    .cursor_pointer()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .hover(|style| style.bg(theme_hover))
                                    .text_xs()
                                    .text_color(theme_muted)
                                    .on_click(on_browse)
                                    .child(div().child("browse…").test_text("browse…")),
                            ),
                    )
                    // 右栏: 三预检 + 提交 + 行内错误
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().child(row_empty.clone()).test_text(row_empty))
                            .child(div().child(row_uniq.clone()).test_text(row_uniq))
                            .child(div().child(row_dir.clone()).test_text(row_dir))
                            .child(
                                div()
                                    .id("solution-config-submit")
                                    .cursor_pointer()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .hover(|style| style.bg(theme_hover))
                                    .text_xs()
                                    .text_color(if blocked { theme_muted } else { cx.theme().accent })
                                    .on_click(on_submit)
                                    .child(div().child("add solution").test_text("add solution")),
                            )
                            .children(
                                self.app
                                    .upgrade()
                                    .and_then(|app| app.read(cx).sol_config_error.clone())
                                    .map(|err| {
                                        div()
                                            .text_xs()
                                            .text_color(theme_danger)
                                            .child(div().test_text(err.clone()))
                                            .child(err)
                                    }),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl TerminalApp {
    /// 左槽 add-solution 入口的目标动作（specs/center-tabs delta: 打开应用内
    /// 表单签, 不再弹系统 picker）。dedup 命中 → select 既有签 + 焦点回交
    /// （单实例）；未命中 → 建签尾插（console 恒 index 0）。
    pub fn open_solution_config_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(&panel) = self.open_tabs.get(SOLUTION_CONFIG_TAB_KEY) {
            self.dock_area
                .update(cx, |area, cx| area.select_panel(panel, window, cx));
            self.focus_active_center_panel(window, cx);
            return;
        }
        let app = cx.weak_entity();
        let panel_entity = cx.new(|cx| SolutionConfigPanel::new(app, window, cx));
        let panel_id = PanelId::from(panel_entity.entity_id());
        self.dock_area.update(cx, |area, cx| {
            area.add_panel_view(
                panel_handle(panel_entity.clone()),
                DockPlacement::Center,
                None,
                window,
                cx,
            );
        });
        self.open_tabs
            .insert(SOLUTION_CONFIG_TAB_KEY.to_string(), panel_id);
        self.sol_config_panel = Some(panel_entity);
        self.sol_config_error = None; // 打开即清陈错（错误不跨签存续）
        self.focus_active_center_panel(window, cx);
    }

    /// 成功回执 → 表单签自关（specs/project-model: Successful creation closes
    /// the tab）。**defer 落关**：调用点在 `poll_sessions`（TerminalApp 外层
    /// 租约内），直接 `remove_panel` 会让 `on_removed` 重入 TerminalApp 撞
    /// double-lease panic；defer 效果只在外层 update 结束后 flush。
    pub(crate) fn close_solution_config_tab(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.sol_config_panel.take() else {
            return;
        };
        let dock = self.dock_area.clone();
        let entity_id = cx.entity().entity_id();
        cx.defer(move |app| {
            app.with_window(entity_id, |window, cx| {
                dock.update(cx, |area, cx| area.remove_panel(panel, window, cx));
            });
        });
    }
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
