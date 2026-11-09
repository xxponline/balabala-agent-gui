//! UI 层: vt100 颜色映射 (theme) + 共享小件 (widgets)。
//! pane 已收为 crate::pane 面板, sidebar 收为 crate::sidebar 面板
//! (change: split-ui-modules-dockarea)。
//!
//!浅色主题默认值见 design D9; 颜色映射是迁移前实现的 gpui 等价物
//! (旧实现在 git history)。


pub mod widgets;

use gpui_kit::{Rgba, rgb};
use vt100::Color;

/// ANSI 0-15 调色板 (浅色主题适配, design D9): 比暗色版整体
/// 加深以保白底可读; "white" (7/15) 取灰阶 — 纯白在白底上不可见。
const PALETTE: [u32; 16] = [
    0x000000, // 0  black
    0xcd3131, // 1  red        (与原版一致)
    0x1a7f2e, // 2  green      (比原版的 0dbc79 深, 白底可读)
    0x8a7f00, // 3  yellow     (橄榄色; 原版 e5e510 白底不可读)
    0x2472c8, // 4  blue       (与原版一致)
    0xa94db0, // 5  magenta
    0x0f7fa8, // 6  cyan
    0x9e9e9e, // 7  white → 中灰 (可读性适配)
    0x5c5c5c, // 8  bright black
    0xe05252, // 9  bright red
    0x2fa043, // 10 bright green
    0x9a8f00, // 11 bright yellow
    0x4a8fe0, // 12 bright blue
    0xc066cc, // 13 bright magenta
    0x2f9fc0, // 14 bright cyan
    0xbfbfbf, // 15 bright white → 浅灰
];

/// xterm 256 色索引 → RGB:
/// 0-15 查表, 16-231 为 6x6x6 立方, 232-255 为灰阶。
/// (原实现对 16+ 回退白/默认色, 这里是完整转换 — 更符合
///  agent-shell spec 的 "preserve colors" 要求)
fn idx_to_rgb(i: u8) -> u32 {
    match i {
        0..=15 => PALETTE[i as usize],
        16..=231 => {
            let n = u32::from(i - 16);
            let (r, g, b) = (n / 36, (n / 6) % 6, n % 6);
            let c = |v: u32| if v == 0 { 0 } else { 55 + 40 * v };
            (c(r) << 16) | (c(g) << 8) | c(b)
        }
        _ => {
            let v = 8 + 10 * u32::from(i - 232);
            (v << 16) | (v << 8) | v
        }
    }
}

/// vt100 前景色 → gpui (浅色主题默认深色前景)。
pub fn fg_to_gpui(color: Color) -> Rgba {
    match color {
        Color::Default => rgb(0x2b2b2b),
        Color::Idx(i) => rgb(idx_to_rgb(i)),
        Color::Rgb(r, g, b) => rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)),
    }
}

/// vt100 背景色 → gpui (浅色主题默认白底)。
pub fn bg_to_gpui(color: Color) -> Rgba {
    match color {
        Color::Default => rgb(0xffffff),
        Color::Idx(i) => rgb(idx_to_rgb(i)),
        Color::Rgb(r, g, b) => rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)),
    }
}
