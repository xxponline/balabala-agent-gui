//! sidebar 渲染: 顶部 (heading + new workspace + 错误行) + workspace 树行。
//!
//! gpui 版 (change: port-ui-to-gpui-kit)。gpui 元素树要求数据被拥有
//! (`AnyElement: 'static`), 所以 shell 状态以值传入; 回调经
//! `cx.listener` 构建, 点击时经 entity update 进入 `TerminalApp` 的
//! 方法 (design D7, 取代原 `SidebarAction` 结构)。
//! agent 行在 3.3。
//!
//! 注意: 辅助构建函数不接收 `cx` — Rust 2024 的 RPIT 会捕获输入
//! 生命周期, 返回的元素会一直借住 cx, 循环里无法复用。listener 在
//! 调用点预构建后以值传入。

use gpui_kit::{InteractiveElement, *};

use crate::agent_entities::SessionStatus;
use crate::app::TerminalApp;

/// 点击回调的统一类型 (拥有; 可存进元素树)。
type ClickListener = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

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
    pub ws_idx: usize,
    pub agent_idx: usize,
    /// 显示名 (如 "foo/pi-1")
    pub name: String,
    pub status: SessionStatus,
    /// 是否是当前 active (高亮 + 粗体)
    pub is_active: bool,
}

/// tooltip 里的纯文本 view (tooltip builder 需要返回 AnyView)。
struct TextTooltip {
    text: String,
}

impl Render for TextTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(0xffffff))
            .border_1()
            .border_color(rgb(0xcccccc))
            .text_xs()
            .child(self.text.clone())
    }
}

/// 渲染 sidebar: 顶部 + workspace 树行。
pub fn render(
    save_error: Option<String>,
    workspaces: Vec<WorkspaceRow>,
    cx: &mut Context<TerminalApp>,
) -> impl IntoElement {
    let on_new_workspace = cx.listener(|this, _event: &ClickEvent, _window, cx| {
        // 异步对话框: 结果回来时再 notify (同步版会阻塞 dispatch 而崩)
        this.create_workspace(cx);
    });

    let mut rows = Vec::with_capacity(workspaces.len() * 3);
    for (idx, ws) in workspaces.iter().enumerate() {
        rows.push(build_workspace_row(idx, ws, cx));
        if !ws.collapsed {
            for agent in &ws.agents {
                rows.push(build_agent_row(agent, cx));
            }
            rows.push(build_new_agent_button(idx, cx));
        }
    }

    div()
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
}

/// 构建一行 workspace。listener 在这里用 cx 预构建 (值捕获 idx),
/// 返回**具体类型** `Stateful<Div>` 而非 opaque: Rust 2024 的 RPIT
/// 会捕获 cx 的借用生命周期, 导致循环里无法多次调用。
fn build_workspace_row(
    idx: usize,
    ws: &WorkspaceRow,
    cx: &mut Context<TerminalApp>,
) -> Stateful<Div> {
    let group = format!("ws-row-{idx}");
    let on_toggle: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, _window, cx| {
            this.toggle_collapse(idx);
            cx.notify();
        },
    ));
    let disabled = ws.agent_count > 0;
    let on_close: Option<ClickListener> = if disabled {
        None // spec: 非空 workspace 的 × 禁用且不响应点击
    } else {
        Some(Box::new(cx.listener(
            move |this, _event: &ClickEvent, _window, cx| {
                this.close_workspace(idx);
                cx.notify();
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
        .id(format!("ws-close-{idx}"))
        .px_1()
        .opacity(0.)
        .group_hover(group.clone(), |style| style.opacity(1.))
        .child(if disabled {
            div()
                .id(format!("ws-close-grey-{idx}"))
                .text_color(rgb(0x666666))
                .child("×")
        } else {
            div()
                .id(format!("ws-close-btn-{idx}"))
                .cursor_pointer()
                .hover(|style| style.text_color(rgb(0xcd3131)))
                .on_click(on_close.expect("present when enabled"))
                .child("×")
        });

    div()
        .id(format!("ws-row-{idx}"))
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
        .child(div().flex_1().text_sm().child(display_name))
        .child(close)
}

/// 构建一个 agent 行: 状态点 + 名字 (active 高亮) + hover 显隐 ×。
fn build_agent_row(agent: &AgentRow, cx: &mut Context<TerminalApp>) -> Stateful<Div> {
    let ws_idx = agent.ws_idx;
    let agent_idx = agent.agent_idx;
    let group = format!("agent-{ws_idx}-{agent_idx}");

    let on_select: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, window, cx| {
            this.active = Some((ws_idx, agent_idx));
            // 一步切焦点 (specs delta): 选中即把键盘/IME 焦点交给 pane
            this.pane_focus.focus(window, cx);
            cx.notify();
        },
    ));
    let on_close: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, _window, cx| {
            this.close_agent(ws_idx, agent_idx, cx);
            cx.notify();
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
        .id(format!("agent-close-{ws_idx}-{agent_idx}"))
        .px_1()
        .opacity(0.)
        .group_hover(group.clone(), |style| style.opacity(1.))
        .child(
            div()
                .id(format!("agent-close-btn-{ws_idx}-{agent_idx}"))
                .cursor_pointer()
                .hover(|style| style.text_color(rgb(0xcd3131)))
                .on_click(on_close)
                .child("×"),
        );

    let row = div()
        .id(format!("agent-{ws_idx}-{agent_idx}"))
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
        .child(div().text_sm().child(name))
        .child(close)
}

/// 展开的 workspace 底部的 "[+ agent]" 按钮。
fn build_new_agent_button(ws_idx: usize, cx: &mut Context<TerminalApp>) -> Stateful<Div> {
    let on_new: ClickListener = Box::new(cx.listener(
        move |this, _event: &ClickEvent, _window, cx| {
            this.create_agent(ws_idx, cx);
            cx.notify();
        },
    ));

    div()
        .id(format!("new-agent-{ws_idx}"))
        .ml(px(16.))
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0xe9e9e9)))
        .text_xs()
        .text_color(rgb(0x888888))
        .on_click(on_new)
        .child("+ agent")
}
