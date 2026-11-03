//! main pane 渲染: 用 InteractiveFrame 包住用户提供的 contents 闭包。
//!
//! pane 本身不再持有 view / agent 引用 — 它只负责:
//!   1. 空占位 (render_empty)
//!   2. 用 InteractiveFrame 包装 contents (render_active)

use eframe::egui;

use crate::ui::common::interactive_frame::InteractiveFrame;

/// 渲染空占位 (没有 active agent 时)。
pub fn render_empty(ui: &mut egui::Ui) {
    ui.centered_and_justified(|ui| {
        ui.label(
            egui::RichText::new("no agent selected\npress + to create one")
                .color(egui::Color32::from_rgb(150, 150, 150))
                .monospace(),
        );
    });
}

/// 渲染 active agent 的主面板。
///
/// - `on_commit`: IME 上屏时调用的闭包
/// - `contents`: 真正的面板内容 (status bar + vt100 网格等), 由调用方提供
pub fn render_active<R>(
    ui: &mut egui::Ui,
    on_commit: &mut dyn FnMut(&str),
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    InteractiveFrame::new()
        .fill(egui::Color32::from_rgb(30, 30, 30))
        .inner_margin(egui::Margin::same(8))
        .show(ui, /* active = */ true, on_commit, contents)
}
