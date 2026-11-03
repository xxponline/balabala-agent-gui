//! sidebar 渲染: "+" 按钮 + agent 列表。

use eframe::egui;

use crate::agent_entities::AgentEntity;
use crate::agent_entities::SessionStatus;

/// 一次 sidebar 渲染的结果: 用户在这一帧点了什么。
#[derive(Default)]
pub struct SidebarAction {
    /// 用户点了 "+" 按钮
    pub clicked_new: bool,
    /// 用户点了某个 agent 项的 "select" 区域 (切换 active)
    pub selected: Option<usize>,
    /// 用户点了某个 agent 项的 "x" 关闭按钮
    pub closed: Option<usize>,
}

pub fn render(
    ui: &mut egui::Ui,
    agents: &[Box<dyn AgentEntity>],
    active_id: Option<usize>,
) -> SidebarAction {
    let mut action = SidebarAction::default();

    ui.vertical(|ui| {
        ui.heading("agents");

        // ---- "+" 新建按钮 ----
        if ui.button("+  new").clicked() {
            action.clicked_new = true;
        }

        ui.separator();

        // ---- agent 列表 ----
        for (idx, agent) in agents.iter().enumerate() {
            let is_active = active_id == Some(idx);
            let name = agent.name().to_string();
            let status = agent.status();

            ui.horizontal(|ui| {
                // 状态徽章: 用一个小圆点 + 颜色区分
                let dot_color = match status {
                    SessionStatus::Starting => egui::Color32::from_rgb(229, 193, 16), // 黄
                    SessionStatus::Running => egui::Color32::from_rgb(75, 215, 108), // 绿
                    SessionStatus::Exited => egui::Color32::from_rgb(205, 49, 49), // 红
                };
                ui.colored_label(dot_color, "●");

                // 名字按钮 (点了切换 active)
                let label = if is_active {
                    egui::RichText::new(format!("{name}  (active)")).strong()
                } else {
                    egui::RichText::new(name)
                };
                if ui.button(label).clicked() {
                    action.selected = Some(idx);
                }

                // hover 时才显示的关闭按钮
                // egui 没有原生 "hover only", 用 small x 按钮 + 透明度模拟
                let close_btn = ui.add(
                    egui::Button::new("×")
                        .small()
                        .fill(egui::Color32::from_rgba_unmultiplied(0, 0, 0, 0)),
                );
                if close_btn.hovered() {
                    close_btn.clone().highlight();
                }
                if close_btn.clicked() {
                    action.closed = Some(idx);
                }
            });
        }
    });

    action
}
