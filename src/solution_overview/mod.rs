//! solution-overview 面板：单工作区的左槽 — Solution/Project/Agent 树。
//!
//! change: solution-overview (design D2) 三段式骨架:
//! `[solution 行 (▾ + 右键配置)] → [project 行 (▾ + [+ agent])] →
//! [零 solution 动作]`。
//! 事件仍经 **SolutionOverviewEvent 漏斗**进
//! `TerminalApp::on_solution_overview_event` 单一入口 (design D3); 载荷只带
//! 稳定身份 (solution = `solution://` 节点 id、project = root PathBuf、
//! session = SessionId — 服务分配, change agent-service 3.2)。行数据每帧
//! 快照成拥有值, listener 在调用点预构建后以值传入 (design D7 — RPIT
//! 生命周期纪律不变)。
//!
//! **改名自 `sidebar`** (change solution-overview 5.1): 模块/类型/锚点
//! 全量跟进, spec 层同步 (design D8)。

use gpui_kit::base::TestTextExt;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::menu::{ContextMenu, ContextMenuExt, PopupMenuItem};
use gpui_kit::{InteractiveElement, *};
use std::path::PathBuf;

use agent_service::contract::{SessionId, SessionView, SessionStatus};
use solution_service::{ActionDesc, ProjectDesc, solution_node_id};
use crate::app::TerminalApp;
use crate::ui::widgets::TextTooltip;

/// 点击回调的统一类型 (拥有; 可存进元素树)。
type ClickListener = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// solution-overview → TerminalApp 事件漏斗（design D3）。
/// 载荷只携带稳定身份：project = root PathBuf，session = SessionId。
///
/// 5 变体 (launcher-startup 4.3): 工作区打开/切换两个事件变体已随切换口
/// 删除 — 工作区选择只剩启动期的 Launcher。
///
/// +1 = `ToggleSolutionCollapse` (change solution-overview 4.2) → 6 变体。
#[derive(Debug)]
pub enum SolutionOverviewEvent {
    /// 空工作区的 add-solution (spec: Solution creation via path picker)。
    AddSolution,
    /// 点 Solution 行: 只切该行折叠 (change solution-overview D2)。
    ToggleSolutionCollapse(String),
    /// 点 project 行: 只切该行折叠 (spec: Project row collapse toggle)。
    ToggleProjectCollapse(PathBuf),
    /// 点某 project 行下的 "[+ agent]" (载荷 = project root = cwd)。
    NewAgent(PathBuf),
    SelectAgent(SessionId),
    CloseAgent(SessionId),
}

/// 左 dock 面板。
pub struct SolutionOverviewPanel {
    app: WeakEntity<TerminalApp>,
    /// `Focusable` trait 要求；spec: solution-overview 不抢焦点，此 handle 不被
    /// track_focus 挂载，仅满足面板契约（一步切焦点目标是 terminal pane）。
    focus: FocusHandle,
}

impl SolutionOverviewPanel {
    pub fn new(app: WeakEntity<TerminalApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app,
            focus: cx.focus_handle(),
        }
    }

    /// 右键菜单项点击 → 转发给 app (change solution-overview 4.3)。
    ///
    /// 面板只转发 — 参数构造 (甲: 点击时由 `checked` 取反) 与 ask 都在
    /// app 侧, 面板不碰契约细节。
    pub(crate) fn invoke_action(
        &mut self,
        node: String,
        action_id: String,
        label: String,
        checked: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        app.update(cx, |app, cx| {
            app.invoke_solution_action(&node, &action_id, &label, checked, cx)
        });
    }
}

impl BasePanel for SolutionOverviewPanel {
    fn panel_name(&self) -> &'static str {
        "solution-overview"
    }
    fn closable(&self, _: &App) -> bool {
        false
    }
    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for SolutionOverviewPanel {
    fn title_bar(&self, _: &App) -> bool {
        false // spec: 切换入口/树自带布局, 不要 DockSkin 标题栏
    }
    fn inner_padding(&self, _: &App) -> bool {
        false // 面板自带 padding
    }
}

impl EventEmitter<PanelEvent> for SolutionOverviewPanel {}

impl Focusable for SolutionOverviewPanel {
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

/// 当前工作区视图 (None = 未打开 → 无行)。
///
/// `display_name` / `full_path` 已随切换口删除 (launcher-startup 4.1)。
pub struct WsView {
    /// solution 行 — **每个 solution 一个**, 其下挂自己的 project 行
    /// (change solution-overview D1: 层级来自 `TreeSnapshot::hierarchy`)。
    pub solutions: Vec<SolutionRow>,
    /// 零 solution 时渲染 add-solution 动作 (project-model delta:
    /// 空态条件从零 project 改为零 solution)。
    pub show_add_solution: bool,
}

/// 一个 Solution 行的渲染数据 (拥有值)。
///
/// 分组头 + 配置挂载点 (design D2): 左键纯折叠、无打开语义; 右键菜单
/// 由服务声明的 `actions` 构建。
pub struct SolutionRow {
    /// `solution://<path>` — 命名空间化身份 (D2)。v1 的 solution path 与
    /// project root 是同一目录, 裸路径会撞 id。
    pub id: String,
    /// `foo · simple` — 类型标记是与下方同名 project 行的唯一视觉区分。
    pub label: String,
    /// 完整 path (tooltip)。
    pub tooltip: String,
    /// 是否折叠 (折叠隐藏其下 project/agent 行)。
    pub collapsed: bool,
    /// 服务为该节点声明的动作 (右键菜单源, design D3 — 带 `checked` 供
    /// 开关项渲染勾选态, 所以用契约的 `ActionDesc` 而非 asset 侧
    /// 只有 id/label 的 `AssetMenu`)。
    pub actions: Vec<ActionDesc>,
    /// 该 solution 下的 project 行。
    pub projects: Vec<ProjectRow>,
}

/// 每帧快照 (拥有值, 拆开借用后再建 listener)。
pub struct SidebarSnapshot {
    pub save_error: Option<String>,
    pub notice: Option<String>,
    pub ws: Option<WsView>,
}

/// 组装一个 project 行 (含其 agent 行; project 折叠时 agent 为空)。
fn project_row(
    p: &ProjectDesc,
    ws: &crate::workspace::Workspace,
    sessions: Option<&SessionView>,
) -> ProjectRow {
    let collapsed = ws.collapsed_projects.contains(&p.root);
    let agents = if collapsed {
        Vec::new()
    } else {
        sessions
            .map(|v| {
                v.sessions
                    .iter()
                    .filter(|s| s.project_root == p.root)
                    .map(|s| AgentRow {
                        id: s.id,
                        name: s.name.clone(),
                        status: s.status,
                        is_active: v.active == Some(s.id),
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
}

/// 从服务投影镜像快照 (render 内调用; 会话归 AgentService, solution/project
/// 树归 solution 服务 — change solution-service 5.4: 视图读镜像, 无本地副本)。
///
/// 层级来自 `TreeSnapshot::hierarchy` (change solution-overview D1):
/// solution 行 → 其 project 行 → agent 行。
fn snapshot(app: &TerminalApp, _cx: &App) -> SidebarSnapshot {
    let ws = app.workspace.as_ref().map(|ws| {
        let sessions = app.service_layer.sessions_view();
        let solutions = app
            .service_layer
            .tree_view()
            .map(|t| {
                t.hierarchy
                    .iter()
                    .map(|node| {
                        let id = solution_node_id(node.item.path());
                        let collapsed = ws.collapsed_solutions.contains(&id);
                        // 折叠时 project 行也不入树 (与 project 折叠隐藏 agent 同构)
                        let projects = if collapsed {
                            Vec::new()
                        } else {
                            node.projects
                                .iter()
                                .map(|p| project_row(&p.desc, ws, sessions))
                                .collect::<Vec<_>>()
                        };
                        SolutionRow {
                            id,
                            label: format!("{} · {}", node_name(node.item.path()), node.item.type_name()),
                            tooltip: node.item.path().to_string_lossy().into_owned(),
                            collapsed,
                            actions: node.actions.to_vec(),
                            projects,
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // 空态条件 = 零 solution (project-model delta)
        let show_add_solution = solutions.is_empty();
        WsView {
            solutions,
            show_add_solution,
        }
    });
    SidebarSnapshot {
        save_error: app.save_error.clone(),
        notice: app.notice.clone(),
        ws,
    }
}

/// 行内短名: 路径末段 (e.g. "foo" for "/Users/me/code/foo")。
fn node_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string())
}

impl Render for SolutionOverviewPanel {
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
        let theme_danger = cx.theme().danger;
        let theme_warning = cx.theme().warning;
        let theme_muted = cx.theme().muted_foreground;

        // 统一装进 AnyElement: `.context_menu()` 会把 solution 行的返回类型
        // 从 `Stateful<Div>` 变成 `ContextMenu<..>`, 同一 Vec 必须同型。
        let mut rows: Vec<AnyElement> = Vec::new();
        if let Some(ws) = &snap.ws {
            for sol in &ws.solutions {
                rows.push(build_solution_row(sol, cx).into_any_element());
                if sol.collapsed {
                    continue; // 分组头折叠 → 隐藏其下全部 (D2)
                }
                for project in &sol.projects {
                    rows.push(build_project_row(project, cx).into_any_element());
                    if !project.collapsed {
                        for agent in &project.agents {
                            rows.push(build_agent_row(agent, cx).into_any_element());
                        }
                        rows.push(
                            build_new_agent_button(project.root.clone(), cx).into_any_element(),
                        );
                    }
                }
            }
            if ws.show_add_solution {
                rows.push(build_add_solution_button(cx).into_any_element());
            }
        }

        div()
            .id("solution-overview-root")
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

/// 构建一行 Solution (分组头 + 配置挂载点, change solution-overview D2)。
///
/// - **元素 id 嵌节点 id** (`solution://<path>`) — 与 project 行的裸路径
///   不同命名空间, 观测锚点与节点身份都能断言到前缀 (D2)。
/// - 左键 = 纯折叠, **无打开语义** (对齐 asset 树 `project://` 分组头)。
/// - 右键菜单由服务声明的 `actions` 构建 (4.3)。
fn build_solution_row(
    sol: &SolutionRow,
    cx: &mut Context<SolutionOverviewPanel>,
) -> ContextMenu<Stateful<Div>> {
    let element_id = format!("solution-row-{}", sol.id);
    let group = format!("solution-{}", sol.id);
    let toggle_id = sol.id.clone();
    let on_toggle: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, _window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_solution_overview_event(
                    SolutionOverviewEvent::ToggleSolutionCollapse(toggle_id.clone()),
                    _window,
                    cx,
                )
            })
            .ok();
        },
    ));
    // 语义 token 快照 (Hsla: Copy) — 闭包捕获值 (design D3)
    let theme_hover = cx.theme().list_hover;
    let tooltip = sol.tooltip.clone();
    let collapsed = sol.collapsed;
    let label = sol.label.clone();

    div()
        .id(element_id)
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
            let text = tooltip.clone();
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
                .id(format!("solution-name-{}", sol.id))
                .test_support()
                .flex_1()
                .text_sm()
                .child(label.clone())
                .test_text(label),
        )
        // 右键菜单 (4.3): **从节点携带的 actions 构建** — 同步读快照,
        // 零往返 (design D3)。label/勾选态都来自服务声明。
        .context_menu({
            let weak = cx.entity().downgrade();
            let actions = sol.actions.clone();
            let node = sol.id.clone();
            move |menu, _window, _cx| {
                let mut menu = menu;
                for desc in actions.iter() {
                    let mut item = PopupMenuItem::new(desc.label.clone());
                    if let Some(v) = desc.checked {
                        item = item.checked(v);
                    }
                    // 点击闭包拿**自己的 clone** — 直接 move `for` 绑定会撞
                    // "move out of a captured variable in an Fn closure"。
                    let act_id = desc.id.clone();
                    let act_label = desc.label.clone();
                    let act_checked = desc.checked;
                    let weak = weak.clone();
                    let node = node.clone();
                    item = item.on_click(move |_, _window, cx| {
                        weak
                            .update(cx, |panel, cx| {
                                panel.invoke_action(
                                    node.clone(),
                                    act_id.clone(),
                                    act_label.clone(),
                                    act_checked,
                                    cx,
                                )
                            })
                            .ok();
                    });
                    menu = menu.item(item);
                }
                menu
            }
        })
}

/// 构建一行 project。listener 在这里用 cx 预构建 (值捕获 root),
/// 返回**具体类型** `Stateful<Div>` 而非 opaque: Rust 2024 的 RPIT
/// 会捕获 cx 的借用生命周期, 导致循环里无法多次调用。
fn build_project_row(p: &ProjectRow, cx: &mut Context<SolutionOverviewPanel>) -> Stateful<Div> {
    // 载荷用 root PathBuf 稳定身份 (D3): 闭包捕获不随删除错位
    let root = PathBuf::from(&p.root);
    let group = format!("project-{}", p.root);
    let toggle_root = root.clone();
    let on_toggle: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, _window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_solution_overview_event(
                    SolutionOverviewEvent::ToggleProjectCollapse(toggle_root.clone()),
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
fn build_agent_row(agent: &AgentRow, cx: &mut Context<SolutionOverviewPanel>) -> Stateful<Div> {
    let id = agent.id;
    let group = format!("agent-{id}");

    let on_select: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, window, cx| {
            let app = this.app.clone();
            // 一步切焦点在 SelectAgent handler 内 (3.2): active + 焦点一次完成
            app.update(cx, |app, cx| {
                app.on_solution_overview_event(SolutionOverviewEvent::SelectAgent(id), window, cx)
            })
            .ok();
        },
    ));
    let on_close: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_solution_overview_event(SolutionOverviewEvent::CloseAgent(id), window, cx)
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
fn build_new_agent_button(root: String, cx: &mut Context<SolutionOverviewPanel>) -> Stateful<Div> {
    // 语义 token 快照 (Hsla: Copy) — 闭包捕获值 (design D3)
    let theme_hover = cx.theme().list_hover;
    let theme_muted = cx.theme().muted_foreground;
    let path = PathBuf::from(&root);
    let on_new: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, _window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_solution_overview_event(SolutionOverviewEvent::NewAgent(path.clone()), _window, cx)
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
fn build_add_solution_button(cx: &mut Context<SolutionOverviewPanel>) -> Stateful<Div> {
    let theme_hover = cx.theme().list_hover;
    let theme_muted = cx.theme().muted_foreground;
    let on_add: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, _window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_solution_overview_event(SolutionOverviewEvent::AddSolution, _window, cx)
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
