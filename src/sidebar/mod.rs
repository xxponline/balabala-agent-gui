//! sidebar 面板：workspace/agent 列表渲染（gpui 版）。
//!
//! 本 change 将原 `ui/sidebar.rs` 自由函数收为 **SidebarPanel entity**
//! （design D2: `Panel` trait 面板；当前先作为普通 child 挂进现有布局，
//! 步骤 5 接入 DockArea 左槽）。事件经 **SidebarEvent 漏斗**
//! （design D3）进 `TerminalApp::on_sidebar_event` 单一入口——
//! 载荷全是稳定身份（PathBuf/EntityId），闭包捕获不随删除错位。
//!
//! gpui 元素树要求数据被拥有（`AnyElement: 'static`），所以行数据在
//! render 内快照成值；回调经 `cx.listener` 构建（design D7）。
//! 注意: 辅助构建函数不接收 `cx` — Rust 2024 的 RPIT 会捕获输入
//! 生命周期, 返回的元素会一直借住 cx, 循环里无法复用。listener 在
//! 调用点预构建后以值传入。

use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::{InteractiveElement, *};
use std::path::PathBuf;

use crate::agent_entities::SessionStatus;
use crate::app::TerminalApp;
use crate::ui::widgets::TextTooltip;

/// 点击回调的统一类型 (拥有; 可存进元素树)。
type ClickListener = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// sidebar → TerminalApp 事件漏斗（design D3）。
/// 载荷只携带稳定身份：workspace = PathBuf，session = EntityId。
#[derive(Debug)]
pub enum SidebarEvent {
    NewWorkspace,
    ToggleCollapse(PathBuf),
    CloseWorkspace(PathBuf),
    NewAgent(PathBuf),
    SelectAgent(EntityId),
    CloseAgent(EntityId),
}

/// 左 dock 面板（当前为普通 child；步骤 5 注册进 DockArea 左槽）。
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
        false // D8: 保住 spec 的 `+ new workspace` 置顶
    }
    fn inner_padding(&self, _: &App) -> bool {
        false // D8: 面板自带 padding, 不要 DockSkin 再加
    }
}

impl EventEmitter<PanelEvent> for SidebarPanel {}

impl Focusable for SidebarPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// 一个 workspace 行的渲染数据 (拥有值)。
pub struct WorkspaceRow {
    /// 完整绝对路径 (tooltip 显示; 由 `Workspace::full_path_str` 生成)
    pub full_path: String,
    /// 行内短名: 路径末段
    pub display_name: String,
    /// 是否折叠
    pub collapsed: bool,
    /// agent 数量 (>0 时 × 禁用)
    pub agent_count: usize,
    /// 展开时渲染的 agent 行 (collapsed 时为空)
    pub agents: Vec<AgentRow>,
}

/// 一个 agent 行的渲染数据 (拥有值)。
pub struct AgentRow {
    /// 稳定身份 (D4: EntityId; active 比较/选中/关闭用, 不受兄弟删除影响)
    pub id: EntityId,
    /// 显示名 (如 "foo/pi-1")
    pub name: String,
    pub status: SessionStatus,
    /// 是否是当前 active (高亮 + 粗体)
    pub is_active: bool,
}

/// 从 model 快照出行数据 (render 内调用; 穿透读 session, 每帧现读)。
fn snapshot(app: &TerminalApp, cx: &App) -> (Option<String>, Vec<WorkspaceRow>) {
    let mut ws_rows = Vec::with_capacity(app.workspaces.len());
    for w in app.workspaces.iter() {
        let mut agents = Vec::new();
        if !w.collapsed {
            for session in w.agents.iter() {
                let s = session.read(cx);
                agents.push(AgentRow {
                    id: session.entity_id(),
                    name: s.0.name().to_string(),
                    status: s.0.status(),
                    is_active: app.active == Some(session.entity_id()),
                });
            }
        }
        ws_rows.push(WorkspaceRow {
            full_path: w.full_path_str(),
            display_name: w.display_name().to_string(),
            collapsed: w.collapsed,
            agent_count: w.agents.len(),
            agents,
        });
    }
    (app.save_error.clone(), ws_rows)
}

impl Render for SidebarPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        // 快照先行 (拥有值), 之后再建 listener (借用 cx)
        let (save_error, workspaces) = {
            let app_ref = app.read(cx);
            snapshot(app_ref, cx)
        };

        // 所有 listener 经 SidebarEvent 漏斗进 TerminalApp (D3)
        let on_new_workspace = cx.listener(|this, _event: &ClickEvent, window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_sidebar_event(SidebarEvent::NewWorkspace, window, cx)
            })
            .ok();
        });

        let mut rows = Vec::with_capacity(workspaces.len() * 3);
        for ws in workspaces.iter() {
            rows.push(build_workspace_row(ws, cx));
            if !ws.collapsed {
                for agent in &ws.agents {
                    rows.push(build_agent_row(agent, cx));
                }
                rows.push(build_new_agent_button(ws.full_path.clone(), cx));
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
                    .child("workspaces"),
            )
            .child(
                div()
                    .id("new-workspace") // Stateful: on_click 需要 element id
                    .cursor_pointer()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(0xcccccc))
                    .hover(|style| style.bg(rgb(0xe9e9e9)))
                    .on_click(on_new_workspace)
                    .child("+ new workspace"),
            )
            .children(save_error.map(|err| {
                div()
                    .text_xs()
                    .text_color(rgb(0xcd3131)) // 原版同款红 (205,49,49)
                    .child(format!("save error: {err}"))
            }))
            .children(rows)
            .into_any_element()
    }
}

/// 构建一行 workspace。listener 在这里用 cx 预构建 (值捕获 path),
/// 返回**具体类型** `Stateful<Div>` 而非 opaque: Rust 2024 的 RPIT
/// 会捕获 cx 的借用生命周期, 导致循环里无法多次调用。
fn build_workspace_row(
    ws: &WorkspaceRow,
    cx: &mut Context<SidebarPanel>,
) -> Stateful<Div> {
    // 载荷用 PathBuf 稳定身份 (D3): 闭包捕获不随删除错位
    let path = PathBuf::from(&ws.full_path);
    let group = format!("ws-row-{}", ws.full_path);
    let toggle_path = path.clone();
    let on_toggle: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, _window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_sidebar_event(SidebarEvent::ToggleCollapse(toggle_path.clone()), _window, cx)
            })
            .ok();
        },
    ));
    let disabled = ws.agent_count > 0;
    let on_close: Option<ClickListener> = if disabled {
        None // spec: 非空 workspace 的 × 禁用且不响应点击
    } else {
        Some(Box::new(cx.listener(
            move |this, _event: &ClickEvent, _window, cx| {
                let app = this.app.clone();
                app.update(cx, |app, cx| {
                    app.on_sidebar_event(SidebarEvent::CloseWorkspace(path.clone()), _window, cx)
                })
                .ok();
                cx.stop_propagation(); // 别连带触发所在行的 toggle
            },
        )))
    };
    let full_path = ws.full_path.clone();
    let display_name = ws.display_name.clone();
    let collapsed = ws.collapsed;
    // hover 才显示的 × (透明度 0 → group_hover 显形)。点按前必然先
    // hover, 所以透明态不会误触。
    let close = div()
        .id(format!("ws-close-{}", ws.full_path))
        .px_1()
        .opacity(0.)
        .group_hover(group.clone(), |style| style.opacity(1.))
        .child(if disabled {
            div()
                .id(format!("ws-close-grey-{}", ws.full_path))
                .text_color(rgb(0x666666))
                .child("×")
        } else {
            div()
                .id(format!("ws-close-btn-{}", ws.full_path))
                .cursor_pointer()
                .hover(|style| style.text_color(rgb(0xcd3131)))
                .on_click(on_close.expect("present when enabled"))
                .child("×")
        });

    div()
        .id(format!("ws-row-{}", ws.full_path))
        .group(group)
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0xe9e9e9)))
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
                // 观测/点击锚点 (no-op): test_support 不能跨函数边界
                // (返回类型是 Stateful<Div>), 挂在内层子元素上, 点击冒泡
                .id(format!("ws-name-{}", ws.full_path))
                .test_support()
                .flex_1()
                .text_sm()
                .child(display_name),
        )
        .child(close)
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
        move |this, _event: &ClickEvent, _window, cx| {
            let app = this.app.clone();
            app.update(cx, |app, cx| {
                app.on_sidebar_event(SidebarEvent::CloseAgent(id), _window, cx)
            })
            .ok();
            cx.stop_propagation(); // 别连带触发选中
        },
    ));

    // 状态点颜色与原版一致
    let dot_color = match agent.status {
        SessionStatus::Starting => rgb(0xe5c110), // (229,193,16) 黄
        SessionStatus::Running => rgb(0x4bd76c),  // (75,215,108) 绿
        SessionStatus::Exited => rgb(0xcd3131),   // (205,49,49) 红
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
                .hover(|style| style.text_color(rgb(0xcd3131)))
                .on_click(on_close)
                .child("×"),
        );

    let row = div()
        .id(format!("agent-{id}"))
        .group(group)
        .ml(px(16.)) // 缩进 (原版的 ui.indent)
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0xe9e9e9)))
        .on_click(on_select);
    // active 高亮: 底色 + 粗体 (原版用 strong)
    let row = if is_active {
        row.bg(rgb(0xd3e6f7)).font_weight(FontWeight::BOLD)
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

/// 展开的 workspace 底部的 "[+ agent]" 按钮。
fn build_new_agent_button(full_path: String, cx: &mut Context<SidebarPanel>) -> Stateful<Div> {
    let path = PathBuf::from(&full_path);
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
        .id(format!("new-agent-{full_path}"))
        .ml(px(16.))
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0xe9e9e9)))
        .text_xs()
        .text_color(rgb(0x888888))
        .on_click(on_new)
        .child(
            div()
                // 存在性检查锚点 (no-op): 内层子元素, 不跨函数边界
                .id(format!("new-agent-label-{full_path}"))
                .test_support()
                .child("+ agent"),
        )
}
