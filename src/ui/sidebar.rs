//! sidebar 渲染: workspace 折叠树 + 每个 workspace 下的 agent 列表。

use eframe::egui;

use crate::agent_entities::SessionStatus;
use crate::workspace::Workspace;

/// 一次 sidebar 渲染的结果: 用户在这一帧点了什么。
#[derive(Default)]
pub struct SidebarAction {
    /// 用户点了顶部的 "[+ new workspace]"
    pub new_workspace: bool,
    /// 用户点了某个 workspace 行 (toggle 折叠)
    pub toggle_collapse: Option<usize>,
    /// 用户点了某个 workspace 的 "×" 关闭按钮
    pub close_workspace: Option<usize>,
    /// 用户点了某个 workspace 底部的 "[+ agent]"
    pub new_agent: Option<usize>,
    /// 用户点了某个 agent 行 (切换 active)
    pub select_agent: Option<(usize, usize)>,
    /// 用户点了某个 agent 的 "×" 关闭按钮
    pub close_agent: Option<(usize, usize)>,
}

pub fn render(
    ui: &mut egui::Ui,
    workspaces: &[Workspace],
    active: Option<(usize, usize)>,
    save_error: Option<&str>,
) -> SidebarAction {
    let mut action = SidebarAction::default();

    ui.vertical(|ui| {
        ui.heading("workspaces");

        // ---- 顶部 [+ new workspace] 按钮 ----
        if ui.button("+ new workspace").clicked() {
            action.new_workspace = true;
        }

        // ---- 持久化失败的红色提示 ----
        if let Some(err) = save_error {
            ui.colored_label(
                egui::Color32::from_rgb(205, 49, 49),
                egui::RichText::new(format!("save error: {err}")).small(),
            );
        }

        ui.separator();

        // ---- workspace 折叠树 ----
        for (ws_idx, ws) in workspaces.iter().enumerate() {
            render_workspace_row(ui, ws_idx, ws, active, &mut action);

            if !ws.collapsed {
                // 展开状态: 渲染 agents + [+ agent] 按钮
                for (agent_idx, agent) in ws.agents.iter().enumerate() {
                    render_agent_row(ui, ws_idx, agent_idx, agent.name(), agent.status(), active, &mut action);
                }
                // workspace 底部的 [+ agent] 按钮
                ui.indent("agent_indent", |ui| {
                    if ui
                        .small_button(egui::RichText::new("+ agent").weak())
                        .clicked()
                    {
                        action.new_agent = Some(ws_idx);
                    }
                });
            }
        }
    });

    action
}

fn render_workspace_row(
    ui: &mut egui::Ui,
    ws_idx: usize,
    ws: &Workspace,
    _active: Option<(usize, usize)>,
    action: &mut SidebarAction,
) {
    let has_agents = !ws.agents.is_empty();
    let toggle_char = if ws.collapsed { "▶" } else { "▼" };
    let short_name = ws.display_name().to_string();
    let full_path = ws.full_path_str();

    ui.horizontal(|ui| {
        // 折叠 toggle + 名字 (合并成一个按钮, 点了 toggle)
        let row = ui.button(format!("{toggle_char} {short_name}"));
        row.clone().on_hover_text(full_path);
        if row.clicked() {
            action.toggle_collapse = Some(ws_idx);
        }

        // hover 才显示的关闭按钮
        let close_btn = ui.add_enabled(
            !has_agents,
            egui::Button::new("×")
                .small()
                .fill(egui::Color32::from_rgba_unmultiplied(0, 0, 0, 0)),
        );
        if close_btn.clicked() {
            action.close_workspace = Some(ws_idx);
        }
    });
}

fn render_agent_row(
    ui: &mut egui::Ui,
    ws_idx: usize,
    agent_idx: usize,
    name: &str,
    status: SessionStatus,
    active: Option<(usize, usize)>,
    action: &mut SidebarAction,
) {
    let is_active = active == Some((ws_idx, agent_idx));

    ui.indent("agent_indent", |ui| {
        ui.horizontal(|ui| {
            // 状态徽章小圆点
            let dot_color = match status {
                SessionStatus::Starting => egui::Color32::from_rgb(229, 193, 16),
                SessionStatus::Running => egui::Color32::from_rgb(75, 215, 108),
                SessionStatus::Exited => egui::Color32::from_rgb(205, 49, 49),
            };
            ui.colored_label(dot_color, "●");

            // 名字按钮 (点了切换 active)
            let label = if is_active {
                egui::RichText::new(name.to_string()).strong()
            } else {
                egui::RichText::new(name.to_string())
            };
            if ui.button(label).clicked() {
                action.select_agent = Some((ws_idx, agent_idx));
            }

            // hover 关闭按钮
            let close_btn = ui.add(
                egui::Button::new("×")
                    .small()
                    .fill(egui::Color32::from_rgba_unmultiplied(0, 0, 0, 0)),
            );
            if close_btn.clicked() {
                action.close_agent = Some((ws_idx, agent_idx));
            }
        });
    });
}
