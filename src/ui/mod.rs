//! UI 层: sidebar + pane 渲染。

pub mod pane;
pub mod sidebar;

use egui::Color32;
use vt100::Color;

/// 把 vt100 的前景色翻译成 egui 的 Color32。
///
/// 只覆盖常用的 16 色调色板 + Rgb; 256 色以上的 Idx 暂时 fallback 到白色。
pub fn fg_to_egui(color: Color) -> Color32 {
    match color {
        Color::Default => Color32::from_rgb(229, 229, 229),
        Color::Idx(0) => Color32::from_rgb(0, 0, 0),
        Color::Idx(1) => Color32::from_rgb(205, 49, 49), // red
        Color::Idx(2) => Color32::from_rgb(13, 188, 121), // green
        Color::Idx(3) => Color32::from_rgb(229, 229, 16), // yellow
        Color::Idx(4) => Color32::from_rgb(36, 114, 200), // blue
        Color::Idx(5) => Color32::from_rgb(188, 75, 173), // magenta
        Color::Idx(6) => Color32::from_rgb(17, 168, 205), // cyan
        Color::Idx(7) => Color32::from_rgb(229, 229, 229), // white (bright)
        Color::Idx(i) if i < 16 => Color32::from_rgb(128, 128, 128),
        Color::Idx(_) => Color32::WHITE,
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

/// 把 vt100 的背景色翻译成 egui 的 Color32 (终端默认深灰底)。
pub fn bg_to_egui(color: Color) -> Color32 {
    match color {
        Color::Default => Color32::from_rgb(30, 30, 30),
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        _ => Color32::from_rgb(30, 30, 30),
    }
}
