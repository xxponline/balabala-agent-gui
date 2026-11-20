//! sidebar 面板：单工作区的左槽 — 顶栏切换入口 + Project/Agent 树 + 空态。
//!
//! change: single-workspace-project-model (design D7 三段式骨架):
//! `[switch entry] → [project 行 (▾ + [+ agent])] → [空态/加 solution 动作]`。
//! 事件仍经 **SidebarEvent 漏斗**进 `TerminalApp::on_sidebar_event` 单一
//! 入口 (design D3); 载荷只带稳定身份 (project root = PathBuf,
//! session = SessionId — 服务分配, change agent-service 3.2)。行数据每帧
//! 快照成拥有值, listener 在调用点预构建后以值传入 (design D7 — RPIT
//! 生命周期纪律不变)。

use gpui_kit::base::TestTextExt;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::{InteractiveElement, *};
use std::path::PathBuf;

use agent_service::contract::{SessionId, SessionStatus};
use crate::app::TerminalApp;
use crate::ui::widgets::TextTooltip;

/// 点击回调的统一类型 (拥有; 可存进元素树)。
type ClickListener = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// sidebar → TerminalApp 事件漏斗（design D3）。
/// 载荷只携带稳定身份：project = root PathBuf，session = SessionId。
#[derive(Debug)]
pub enum SidebarEvent {
    /// 空态选择器: 无门控, 直接走打开流程 (spec: picker at startup)。
    OpenWorkspace,
    /// 顶栏切换入口: 先过三支门控 (spec: Gated workspace switching)。
    SwitchWorkspace,
    /// 空工作区的 add-solution (spec: Solution creation via path picker)。
    AddSolution,
    /// 点 project 行: 只切该行折叠 (spec: Project row collapse toggle)。
    ToggleProjectCollapse(PathBuf),
    /// 点某 project 行下的 "[+ agent]" (载荷 = project root = cwd)。
    NewAgent(PathBuf),
    SelectAgent(SessionId),
    CloseAgent(SessionId),
}

/// 左 dock 面板。
pub struct SidebarPanel {
    app: WeakEntity<TerminalApp>,
    /// `Focusable` trait 要求；spec: sidebar 不抢焦点，此 handle 不被
    /// track_focus 挂载，仅满足面板契约（一步切焦点目标是 terminal pane）。
    focus: FocusHandle,
}

impl SidebarPanel {
    pub fn new(app: WeakEntity<TerminalApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app,
            focus: cx.focus_handle(),
        }
    }
}

impl BasePanel for SidebarPanel {
    fn panel_name(&self) -> &'static str {
        "sidebar"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for SidebarPanel {
    fn title_bar(&self, _: &App) -> bool {
        false // spec: 切换入口/树自带布局, 不要 DockSkin 标题栏
    }
    fn inner_padding(&self, _: &App) -> bool {
        false // 面板自带 padding
    }
}

impl EventEmitter<PanelEvent> for SidebarPanel {}

impl Focusable for SidebarPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// 一个 project 行的渲染数据 (拥有值)。
pub struct ProjectRow {
    /// project root 绝对路径 (事件载荷 / 锚点 id; spec: 身份 = root)。
    pub root: String,
    /// 显示名 (project name = root 末段)。
    pub name: String,
    /// 是否折叠 (折叠隐藏 agent 行与 [+ agent], spec)
    pub collapsed: bool,
    /// 展开时渲染的 agent 行 (collapsed 时为空)
    pub agents: Vec<AgentRow>,
}

/// 一个 agent 行的渲染数据 (拥有值)。
pub struct AgentRow {
    /// 稳定身份 (服务分配 SessionId; active 比较/选中/关闭用)。
    pub id: SessionId,
    /// 显示名 (如 "foo/pi-1")
    pub name: String,
    pub status: SessionStatus,
    /// 是否是当前 active (高亮 + 粗体)
    pub is_active: bool,
}

/// 当前工作区视图 (None = 未打开 → 空态)。
pub struct WsView {
    pub display_name: String,
    /// 完整绝对路径 (切换入口 tooltip — 保留旧 workspace 行的 hover 信息)。
    pub full_path: String,
    pub projects: Vec<ProjectRow>,
    /// 零 project 时渲染 add-solution 动作 (spec: zero projects offer …)
    pub show_add_solution: bool,
}

/// 每帧快照 (拥有值, 拆开借用后再建 listener)。
pub struct SidebarSnapshot {
    pub save_error: Option<String>,
    pub notice: Option<String>,
    pub ws: Option<WsView>,
}

/// 从 model + 服务投影镜像快照 (render 内调用; 会话归 AgentService,
/// GUI 读 `sessions_view` 镜像 — change agent-service 3.2, D4)。
fn snapshot(app: &TerminalApp, _cx: &App) -> SidebarSnapshot {
    let ws = app.workspace.as_ref().map(|ws| {
        let view = app.service_layer.sessions_view();
        let projects = ws
            .model
            .projects
            .iter()
            .map(|p| {
                let collapsed = ws.collapsed_projects.contains(&p.root);
                let agents = if collapsed {
                    Vec::new()
                } else {
                    view.map(|v| {
                        v.sessions
                            .iter()
                            .filter(|s| s.project_root == p.root)
                            .map(|s| AgentRow {
                                id: s.id,
                                name: s.name.clone(),
                                status: s.status,
                                is_active: app.active == Some(s.id),
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
                };
                ProjectRow {
                    root: p.root.to_string_lossy().into_owned(),
                    name: p.name.clone(),
                    collapsed,
                    agents,
                }
            })
            .collect::<Vec<_>>();
        let show_add_solution = projects.is_empty();
        WsView {
            display_name: ws.display_name().to_string(),
            full_path: ws.full_path_str(),
            projects,
            show_add_solution,
        }
    });
    SidebarSnapshot {
        save_error: app.save_error.clone(),
        notice: app.notice.clone(),
        ws,
    }
}

impl Render for SidebarPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        // 快照先行 (拥有值), 之后再建 listener (借用 cx)
        let snap = {
            let app_ref = app.read(cx);
            snapshot(app_ref, cx)
        };
        // 语义 token 快照 (Hsla: Copy) — 闭包捕获值而非 &cx (design D3)
        let theme_border = cx.theme().border;
        let theme_hover = cx.theme().list_hover;
        let theme_danger = cx.theme().danger;
        let theme_warning = cx.theme().warning;
        let theme_muted = cx.theme().muted_foreground;

        // 切换入口 listener: 当前无工作区 → OpenWorkspace (空态选择器);
        // 有 → SwitchWorkspace (过门控)。事件在 app 端按状态分派。
        let on_switch = cx.listener(|this, _event: &ClickEvent, window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                let ev = if app.workspace.is_some() {
                    SidebarEvent::SwitchWorkspace
                } else {
                    SidebarEvent::OpenWorkspace
                };
                app.on_sidebar_event(ev, window, cx)
            })
            .ok();
        });
        let switch_label = snap
            .ws
            .as_ref()
            .map(|w| format!("{} ▾", w.display_name))
            .unwrap_or_else(|| "open workspace…".to_string());
        // 已开态: hover 显示完整路径 (旧 workspace 行的 tooltip 信息);
        // 未开态退化为入口文案。闭包捕获值 (design D3 纪律)
        let tooltip_text = snap
            .ws
            .as_ref()
            .map(|w| w.full_path.clone())
            .unwrap_or_else(|| switch_label.clone());

        let mut rows = Vec::new();
        if let Some(ws) = &snap.ws {
            for project in &ws.projects {
                rows.push(build_project_row(project, cx));
                if !project.collapsed {
                    for agent in &project.agents {
                        rows.push(build_agent_row(agent, cx));
                    }
                    rows.push(build_new_agent_button(project.root.clone(), cx));
                }
            }
            if ws.show_add_solution {
                rows.push(build_add_solution_button(cx));
            }
        }

        div()
            .id("sidebar-root")
            .test_support() // 观测锚点 (无 test-support 特性时为 no-op)
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child("workspace"),
            )
            .child(
                div()
                    .id("workspace-switch") // Stateful: on_click 需要 element id
                    .cursor_pointer()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(theme_border)
                    .hover(|style| style.bg(theme_hover))
                    .tooltip(move |_window, cx: &mut App| {
                        let text = tooltip_text.clone();
                        cx.new(move |_| TextTooltip { text }).into()
                    })
                    .on_click(on_switch)
                    .child({
                        div()
                            // 观测锚点: 空态/已开态标签可断言 (no-op)
                            .id("workspace-switch-label")
                            .test_support()
                            .child(switch_label.clone())
                            // texts() 只收录 test_text (pane/tree 同款)
                            .test_text(switch_label)
                    }),
            )
            .children(snap.save_error.map(|err| {
                div()
                    .text_xs()
                    .text_color(theme_danger)
                    .child(format!("save error: {err}"))
            }))
            .children(snap.notice.map(|msg| {
                div()
                    .id("workspace-notice")
                    .text_xs()
                    .text_color(theme_warning)
                    .child(msg)
            }))
            // 空态 (spec: in-window empty state offering a directory picker):
            // 切换入口此时即选择器 (OpenWorkspace), 提示文案引导点击。
            .children(snap.ws.is_none().then(|| {
                div()
                    .id("workspace-empty-hint")
                    .text_xs()
                    .text_color(theme_muted)
                    .child("no workspace open — choose a directory to start")
                    .test_text("no workspace open — choose a directory to start")
            }))
            // 零 project: add-solution 动作 (spec: Solution creation path picker)
            .children(snap.ws.as_ref().and_then(|ws| {
                ws.show_add_solution.then(|| {
                    div()
                        .id("solution-empty-hint")
                        .text_xs()
                        .text_color(theme_muted)
                        .child("no solutions yet — add a project directory")
                        .test_text("no solutions yet — add a project directory")
                })
            }))
            .children(rows)
            .into_any_element()
    }
}

/// 构建一行 project。listener 在这里用 cx 预构建 (值捕获 root),
/// 返回**具体类型** `Stateful<Div>` 而非 opaque: Rust 2024 的 RPIT
/// 会捕获 cx 的借用生命周期, 导致循环里无法多次调用。
fn build_project_row(p: &ProjectRow, cx: &mut Context<SidebarPanel>) -> Stateful<Div> {
    // 载荷用 root PathBuf 稳定身份 (D3): 闭包捕获不随删除错位
    let root = PathBuf::from(&p.root);
    let group = format!("project-{}", p.root);
    let toggle_root = root.clone();
    let on_toggle: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, _window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_sidebar_event(
                    SidebarEvent::ToggleProjectCollapse(toggle_root.clone()),
                    _window,
                    cx,
                )
            })
            .ok();
        },
    ));
    // 语义 token 快照 (Hsla: Copy) — 闭包捕获值 (design D3)
    let theme_hover = cx.theme().list_hover;
    let full_path = p.root.clone();
    let collapsed = p.collapsed;
    let name = p.name.clone();

    div()
        .id(format!("project-row-{}", p.root))
        .group(group)
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(theme_hover))
        .tooltip(move |_window, cx: &mut App| {
            let text = full_path.clone();
            cx.new(move |_| TextTooltip { text }).into()
        })
        .on_click(on_toggle)
        .child(
            div()
                .w(px(14.))
                .text_sm()
                .child(if collapsed { "▶" } else { "▼" }),
        )
        .child(
            div()
                // 观测/点击锚点 (no-op): 挂在内层子元素上, 点击冒泡
                .id(format!("project-name-{}", p.root))
                .test_support()
                .flex_1()
                .text_sm()
                .child(name.clone())
                .test_text(name),
        )
}

/// 构建一个 agent 行: 状态点 + 名字 (active 高亮) + hover 显隐 ×。
fn build_agent_row(agent: &AgentRow, cx: &mut Context<SidebarPanel>) -> Stateful<Div> {
    let id = agent.id;
    let group = format!("agent-{id}");

    let on_select: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, window, cx| {
            let app = this.app.clone();
            // 一步切焦点在 SelectAgent handler 内 (3.2): active + 焦点一次完成
            app.update(cx, |app, cx| {
                app.on_sidebar_event(SidebarEvent::SelectAgent(id), window, cx)
            })
            .ok();
        },
    ));
    let on_close: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_sidebar_event(SidebarEvent::CloseAgent(id), window, cx)
            })
            .ok();
            cx.stop_propagation(); // 别连带触发选中
        },
    ));

    // 语义 token 快照 (Hsla: Copy) — 闭包捕获值 (design D3)
    let theme_danger = cx.theme().danger;
    let theme_warning = cx.theme().warning;
    let theme_success = cx.theme().success;
    let theme_hover = cx.theme().list_hover;
    let theme_active = cx.theme().list_active;
    // 状态点颜色 (warning/success/danger 语义)
    let dot_color = match agent.status {
        SessionStatus::Starting => theme_warning,
        SessionStatus::Running => theme_success,
        SessionStatus::Exited => theme_danger,
    };
    let name = agent.name.clone();
    let is_active = agent.is_active;

    let close = div()
        .id(format!("agent-close-{id}"))
        .px_1()
        .opacity(0.)
        .group_hover(group.clone(), |style| style.opacity(1.))
        .child(
            div()
                .id(format!("agent-close-btn-{id}"))
                .cursor_pointer()
                .hover(|style| style.text_color(theme_danger))
                .on_click(on_close)
                .child("×"),
        );

    let row = div()
        .id(format!("agent-{id}"))
        .group(group)
        .ml(px(16.)) // 缩进 (project 行内二级)
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(theme_hover))
        .on_click(on_select);
    // active 高亮: 底色 + 粗体
    let row = if is_active {
        row.bg(theme_active).font_weight(FontWeight::BOLD)
    } else {
        row
    };

    row.child(div().text_color(dot_color).child("●"))
        .child(
            div()
                // 观测/点击锚点 (no-op): 内层子元素, 点击冒泡到行 on_select
                .id(format!("agent-name-{id}"))
                .test_support()
                .text_sm()
                .child(name),
        )
        .child(close)
}

/// 展开的 project 底部的 "[+ agent]" 按钮 (spec: at the bottom of each
/// project row's agent list; 无 project 则整体不渲染该行)。
fn build_new_agent_button(root: String, cx: &mut Context<SidebarPanel>) -> Stateful<Div> {
    // 语义 token 快照 (Hsla: Copy) — 闭包捕获值 (design D3)
    let theme_hover = cx.theme().list_hover;
    let theme_muted = cx.theme().muted_foreground;
    let path = PathBuf::from(&root);
    let on_new: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, _window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_sidebar_event(SidebarEvent::NewAgent(path.clone()), _window, cx)
            })
            .ok();
        },
    ));

    div()
        .id(format!("new-agent-{root}"))
        .ml(px(16.))
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(theme_hover))
        .text_xs()
        .text_color(theme_muted)
        .on_click(on_new)
        .child(
            div()
                // 存在性检查锚点 (no-op): 内层子元素, 不跨函数边界
                .id(format!("new-agent-label-{root}"))
                .test_support()
                .child("+ agent")
                .test_text("+ agent"),
        )
}

/// 零 project 工作区的 "+ add solution" 动作 (spec: Solution creation
/// via path picker — 目录选择器 → simple item → project 行出现)。
fn build_add_solution_button(cx: &mut Context<SidebarPanel>) -> Stateful<Div> {
    let theme_hover = cx.theme().list_hover;
    let theme_muted = cx.theme().muted_foreground;
    let on_add: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, _window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_sidebar_event(SidebarEvent::AddSolution, _window, cx)
            })
            .ok();
        },
    ));

    div()
        .id("add-solution")
        .cursor_pointer()
        .px_2()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .hover(|style| style.bg(theme_hover))
        .text_xs()
        .text_color(theme_muted)
        .on_click(on_add)
        .child(
            div()
                // 存在性检查锚点 (no-op)
                .id("add-solution-label")
                .test_support()
                .child("+ add solution")
                .test_text("+ add solution"),
        )
}
