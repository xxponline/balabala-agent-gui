//! main pane 渲染: vt100 网格 + 光标 + 状态条。

use eframe::egui;

use crate::agent_entities::{AgentEntity, SessionStatus};
use crate::ui::{bg_to_egui, fg_to_egui};

/// 渲染 active agent 的主面板。
///
/// - 没有 active agent: 显示空 placeholder
/// - active agent 处于 Starting: 显示 "waiting for pi to start..." 占位
/// - 其他: 画 vt100 网格 + 静态光标
pub fn render(ui: &mut egui::Ui, active: Option<&mut Box<dyn AgentEntity>>) {
    let Some(active) = active else {
        // 6.5: 空 main 区
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new("no agent selected\npress + to create one")
                    .color(egui::Color32::from_rgb(150, 150, 150))
                    .monospace(),
            );
        });
        return;
    };

    let name = active.name().to_string();
    let status = active.status();
    let screen = active.screen();
    let (rows, cols) = screen.size();
    let (rows, cols) = (rows.get(), cols.get());
    let (cur_row, cur_col) = screen.cursor_position();

    // ---- 6.4: 高亮边框 + 顶部状态条 ----
    egui::Frame::new()
        .fill(egui::Color32::from_rgb(30, 30, 30))
        .stroke(egui::Stroke::new(1.5, egui::Color32::from_rgb(75, 161, 215)))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            // 顶部状态条
            ui.horizontal(|ui| {
                ui.colored_label(
                    egui::Color32::from_rgb(75, 161, 215),
                    format!("● {name}"),
                );
                ui.label(
                    egui::RichText::new(format!("[{status:?}]"))
                        .color(egui::Color32::from_rgb(150, 150, 150)),
                );
            });
            ui.separator();

            // ---- 6.3: Starting 占位 ----
            if status == SessionStatus::Starting {
                ui.label(
                    egui::RichText::new("waiting for pi to start...")
                        .color(egui::Color32::from_rgb(229, 193, 16))
                        .monospace(),
                );
            }

            // ---- 6.1 / 6.2: vt100 网格 + 静态光标 ----
            ui.vertical(|ui| {
                ui.style_mut().override_font_id =
                    Some(egui::FontId::monospace(13.0));

                for r in 0..rows {
                    // 收集这一行的"颜色段"
                    let mut segments: Vec<(String, egui::Color32, egui::Color32)> =
                        Vec::new();
                    let mut cur_text = String::new();
                    let mut cur_fg = egui::Color32::WHITE;
                    let mut cur_bg = egui::Color32::from_rgb(30, 30, 30);

                    for c in 0..cols {
                        let cell = match screen.cell(r, c) {
                            Some(cell) => cell,
                            None => continue,
                        };
                        let mut fg = fg_to_egui(cell.fgcolor());
                        let mut bg = bg_to_egui(cell.bgcolor());

                        // 6.2: 在光标位置反色
                        if r == cur_row && c == cur_col {
                            std::mem::swap(&mut fg, &mut bg);
                        }

                        if (fg, bg) != (cur_fg, cur_bg) && !cur_text.is_empty() {
                            segments.push((
                                std::mem::take(&mut cur_text),
                                cur_fg,
                                cur_bg,
                            ));
                            cur_fg = fg;
                            cur_bg = bg;
                        }
                        cur_text.push_str(cell.contents());
                    }
                    if !cur_text.is_empty() {
                        segments.push((cur_text, cur_fg, cur_bg));
                    }

                    // 关掉 egui 默认的 widget 间距, 让字符格紧密排列, 否则
                    // 相邻的颜色段 / 行会出现一条缝。
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    ui.horizontal(|ui| {
                        for (text, fg, bg) in segments {
                            // 在 bg 是终端默认深灰时, 不画背景 (省 fill 开销)
                            if bg == egui::Color32::from_rgb(30, 30, 30) {
                                ui.colored_label(fg, text);
                            } else {
                                // 自定义底色: 用 Frame 包一下 (不加 margin, 否则会比
                                // 无底色段宽出几像素, 行列不对齐)
                                egui::Frame::new()
                                    .fill(bg)
                                    .show(ui, |ui| {
                                        ui.colored_label(fg, text);
                                    });
                            }
                        }
                    });
                }
            });
        });
}
