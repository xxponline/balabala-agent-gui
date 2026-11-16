//! UI 层: vt100 颜色映射 (theme) + 共享小件 (widgets)。
//! pane 已收为 crate::pane 面板, sidebar 收为 crate::sidebar 面板
//! (change: split-ui-modules-dockarea)。
//!
//! **双主题颜色体系 (add-light-dark-theme)**:
//! - **调色板/默认色 (本模块)**: `THEME_LIGHT` / `THEME_DARK` 两份
//!   `TerminalTheme` 快照 (ANSI 0-15 表 + 默认 fg/bg)。渲染路径经
//!   `terminal_theme(cx)` 按全局主题取快照, 再传给纯函数
//!   `fg_to_gpui`/`bg_to_gpui` — 保持映射纯净可单测 (design D2)。
//!   xterm 256 色立方/灰阶为标准公式, 两主题共用。
//! - **chrome 语义 token (design D3 映射表)**: 各面板的 hover/边框/
//!   背景/状态色一律读 `cx.theme()` (gpui-component 语义 token):
//!   hover → `list_hover`, active 行 → `list_active`, 边框/分隔线 →
//!   `border`, 面板底 → `background`, 灰字 → `muted_foreground`,
//!   红/黄/绿状态 → `danger`/`warning`/`success`, 蓝强调 → `accent`。
//!   完整表见 openspec/changes/add-light-dark-theme/design.md D3。
//! - **Hsla→Rgba 转换点**: token 是 `Hsla`; gpui 的 `.bg()`/`.text_color()`
//!   接受 `impl Into<Hsla>`/Fill, 直接传 token 即可, 无需转换。本模块的
//!   `fg_to_gpui`/`bg_to_gpui` 输出 `Rgba` (gpui `rgb()` 构造器), 两类型
//!   经 gpui-pre `From` impl 互转; 比较色值时统一到 u32 空间 (避免
//!   Hsla↔Rgba 浮点舍入误报, 见 pane 测试 `to_u32`)。
//!
//! 浅色调色板初值见 design D9 (历史); 深色表为 pre-port 原版恢复 (D2)。


pub mod widgets;

use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{Rgba, rgb};
use pi_shell_rs::app::theme::TuiTheme;
use vt100::Color;

/// ANSI 0-15 调色板 (浅色主题, design D9): 比暗色版整体
/// 加深以保白底可读; "white" (7/15) 取灰阶 — 纯白在白底上不可见。
const PALETTE_LIGHT: [u32; 16] = [
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

/// ANSI 0-15 调色板 (深色主题, add-light-dark-theme D2): 0-7 为原版
/// 暗色值 (pre-port git history, 即 VS Code dark+ ANSI 色板); 8-15 按
/// 同源亮色系补全 — 旧实现 8-15 全 fallback 灰、256+ 白的行为已随
/// 完整立方转换废弃, 此处不恢复。
const PALETTE_DARK: [u32; 16] = [
    0x000000, // 0  black
    0xcd3131, // 1  red        (原版)
    0x0dbc79, // 2  green      (原版)
    0xe5e510, // 3  yellow     (原版)
    0x2472c8, // 4  blue       (原版)
    0xbc4bad, // 5  magenta    (原版)
    0x11a8cd, // 6  cyan       (原版)
    0xe5e5e5, // 7  white      (原版)
    0x6c6c6c, // 8  bright black
    0xf14c4c, // 9  bright red
    0x23d18b, // 10 bright green
    0xf5f543, // 11 bright yellow
    0x3b8eea, // 12 bright blue
    0xd670d6, // 13 bright magenta
    0x29b8db, // 14 bright cyan
    0xffffff, // 15 bright white
];

/// 主题相关的终端颜色快照: 0-15 调色板 + 默认前景/背景。
/// 纯数据 — fg/bg 映射函数按它转换, 保持函数纯净可单测 (design D2)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalTheme {
    pub palette: [u32; 16],
    pub default_fg: u32,
    pub default_bg: u32,
}

/// 浅色主题终端配色 (白底深字; 现值不变)。
pub const THEME_LIGHT: TerminalTheme = TerminalTheme {
    palette: PALETTE_LIGHT,
    default_fg: 0x2b2b2b,
    default_bg: 0xffffff,
};

/// 深色主题终端配色 (原版恢复: 底 0x0a0a0a / 字 0xe5e5e5)。
/// 底色对齐 gpui-component 深色 `background` token (而非历史值 0x1e1e1e):
/// pane 面板底走该 token (design D3), 两者必须同值否则网格四周出现
/// 接缝 (spec: "no visible seam against the pane frame"); design 风险
/// 条款允许调常量。
pub const THEME_DARK: TerminalTheme = TerminalTheme {
    palette: PALETTE_DARK,
    default_fg: 0xe5e5e5,
    default_bg: 0x0a0a0a,
};

/// xterm 256 色索引 → RGB:
/// 0-15 查当前主题调色板, 16-231 为 6x6x6 立方, 232-255 为灰阶。
/// (原实现对 16+ 回退白/默认色, 这里是完整转换 — 更符合
///  agent-shell spec 的 "preserve colors" 要求)
fn idx_to_rgb(i: u8, theme: &TerminalTheme) -> u32 {
    match i {
        0..=15 => theme.palette[i as usize],
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

/// 当前主题的终端配色快照 (供渲染路径从 cx 取; 纯函数仍收显式
/// theme 参数 — design D2)。
pub fn terminal_theme(cx: &gpui_kit::App) -> &'static TerminalTheme {
    match gpui_kit::component::theme::Theme::global(cx).mode {
        gpui_kit::component::theme::ThemeMode::Light => &THEME_LIGHT,
        gpui_kit::component::theme::ThemeMode::Dark => &THEME_DARK,
    }
}

/// 构造注入 pi-shell-rs 的宿主配色 (add-light-dark-theme D4: 颜色真相
/// 在宿主)。深色 = `TuiTheme::default()` (pi dark.json 现值 — 其 R2
/// 保证默认路径字节级不变, 注入默认值 ≡ 不注入); 浅色为本仓自有值:
///   - 工具卡三态面板: 白底上的浅 tint (深字 0x2b2b2b 高对比), 对应
///     深色版的运行灰/成功墨绿/失败暗红语义
///   - 滚动条: track 贴应用边框灰、thumb 贴 muted 灰 (双主题同源审美)
///   - 语法高亮: syntect 内置 `base16-ocean.light` (其 is_syntax_theme_valid
///     可预检; 无效名渲染回退不 panic)
pub fn tui_theme(mode: ThemeMode) -> TuiTheme {
    match mode {
        ThemeMode::Dark => TuiTheme::default(),
        ThemeMode::Light => TuiTheme {
            tool_running_bg: (0xeb, 0xeb, 0xeb).into(),
            tool_error_bg: (0xfd, 0xe8, 0xe8).into(),
            tool_success_bg: (0xe6, 0xf4, 0xea).into(),
            scrollbar_track: (0xd9, 0xd9, 0xd9).into(),
            scrollbar_thumb: (0x9e, 0x9e, 0x9e).into(),
            syntax_theme: "base16-ocean.light".to_string(),
        },
    }
}

/// vt100 前景色 → gpui (默认前景随主题)。
pub fn fg_to_gpui(color: Color, theme: &TerminalTheme) -> Rgba {
    match color {
        Color::Default => rgb(theme.default_fg),
        Color::Idx(i) => rgb(idx_to_rgb(i, theme)),
        Color::Rgb(r, g, b) => rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)),
    }
}

/// vt100 背景色 → gpui (默认背景随主题)。
pub fn bg_to_gpui(color: Color, theme: &TerminalTheme) -> Rgba {
    match color {
        Color::Default => rgb(theme.default_bg),
        Color::Idx(i) => rgb(idx_to_rgb(i, theme)),
        Color::Rgb(r, g, b) => rgb((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)),
    }
}

#[cfg(test)]
mod tests {
    //! add-light-dark-theme task 2.1: 双调色板表 + 共享 256 色转换的锁定。

    use super::*;

    /// 两表各 16 项且不互为拷贝 (深色表不是浅色表的意外副本)。
    #[test]
    fn both_palettes_are_16_entries_and_distinct() {
        assert_eq!(PALETTE_LIGHT.len(), 16);
        assert_eq!(PALETTE_DARK.len(), 16);
        assert_ne!(
            PALETTE_LIGHT, PALETTE_DARK,
            "深色表必须与浅色表不同"
        );
        // 原版恢复锚点: 深色绿/黄/白为 pre-port 暗色值
        assert_eq!(PALETTE_DARK[2], 0x0dbc79);
        assert_eq!(PALETTE_DARK[3], 0xe5e510);
        assert_eq!(PALETTE_DARK[7], 0xe5e5e5);
        // 两主题默认底/字不同
        assert_ne!(THEME_LIGHT.default_bg, THEME_DARK.default_bg);
        assert_ne!(THEME_LIGHT.default_fg, THEME_DARK.default_fg);
    }

    /// 16-255 共享转换 (立方 + 灰阶) 锁定: 与 xterm 标准公式逐项比对 —
    /// 该段不查表, 两主题共用, 任何回归两主题同时受影响。
    #[test]
    fn shared_256_color_conversion_matches_xterm_formula() {
        let reference = |i: u8| -> u32 {
            match i {
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
        };
        for i in 16..=255u8 {
            assert_eq!(
                idx_to_rgb(i, &THEME_LIGHT),
                reference(i),
                "idx {i} 的转换偏离 xterm 标准公式"
            );
            // 两主题共享同一立方/灰阶转换
            assert_eq!(
                idx_to_rgb(i, &THEME_DARK),
                reference(i),
                "idx {i} 深色主题下偏离 xterm 标准公式"
            );
        }
        // 关锚点: 立方黑/立方白/灰阶首/灰阶尾
        assert_eq!(idx_to_rgb(16, &THEME_LIGHT), 0x000000);
        assert_eq!(idx_to_rgb(231, &THEME_LIGHT), 0xffffff);
        assert_eq!(idx_to_rgb(232, &THEME_LIGHT), 0x080808);
        assert_eq!(idx_to_rgb(255, &THEME_LIGHT), 0xeeeeee);
    }

    /// add-light-dark-theme task 4.2/4.3: 注入配色构造 —
    /// 深色 ≡ pi-shell-rs 默认 (其 R2: 默认路径字节级不变, 注入默认值
    /// ≡ 不注入); 浅色为自有值且语法主题名必须有效 (无效名会回退)。
    #[test]
    fn tui_theme_dark_is_pi_default_and_light_is_valid() {
        use pi_shell_rs::app::theme::TuiTheme;

        let dark = tui_theme(ThemeMode::Dark);
        assert_eq!(
            dark,
            TuiTheme::default(),
            "深色注入必须等于 pi dark.json 现值 (4.3: 注入 ≡ 不注入)"
        );

        let light = tui_theme(ThemeMode::Light);
        assert_ne!(light, dark, "浅色必须与深色不同");
        assert_eq!(light.syntax_theme, "base16-ocean.light");
        assert!(
            light.is_syntax_theme_valid(),
            "语法主题名必须在 syntect 内置集中 (无效会回退默认)"
        );
        // 浅色面板必须比深色面板亮 (白底浅 tint, spec: light color set) —
        // 不点名 ratatui::Color (无直接依赖), 经 Debug 输出取 RGB 分量。
        let rgb_sum = |c: &pi_shell_rs::app::theme::TuiTheme| -> u32 {
            let dbg = format!("{:?}", c.tool_running_bg);
            let nums: Vec<u32> = dbg
                .trim_start_matches("Rgb(")
                .trim_end_matches(')')
                .split(',')
                .filter_map(|p| p.trim().parse().ok())
                .collect();
            nums.iter().sum()
        };
        assert!(
            rgb_sum(&light) > rgb_sum(&dark),
            "浅色运行面板应更亮 (light={:?}, dark={:?})",
            light.tool_running_bg,
            dark.tool_running_bg
        );
    }

    /// add-light-dark-theme task 2.2: Default 随主题、显式 RGB 双主题透传。
    #[test]
    fn default_colors_follow_theme_and_rgb_passes_through() {
        // Default: 浅色 = 深字白底, 深色 = 亮字暗底
        assert_eq!(fg_to_gpui(Color::Default, &THEME_LIGHT), rgb(0x2b2b2b));
        assert_eq!(bg_to_gpui(Color::Default, &THEME_LIGHT), rgb(0xffffff));
        assert_eq!(fg_to_gpui(Color::Default, &THEME_DARK), rgb(0xe5e5e5));
        assert_eq!(bg_to_gpui(Color::Default, &THEME_DARK), rgb(0x0a0a0a));

        // 显式 RGB: 双主题逐字节相同 (spec: Explicit session colors pass through)
        let sample = Color::Rgb(0x12, 0x34, 0x56);
        assert_eq!(
            fg_to_gpui(sample, &THEME_LIGHT),
            fg_to_gpui(sample, &THEME_DARK)
        );
        assert_eq!(fg_to_gpui(sample, &THEME_LIGHT), rgb(0x123456));
        assert_eq!(
            bg_to_gpui(sample, &THEME_LIGHT),
            bg_to_gpui(sample, &THEME_DARK)
        );

        // 索引色: 双主题走各自调色板 (深浅不同)
        let idx = Color::Idx(2); // green
        assert_ne!(
            fg_to_gpui(idx, &THEME_LIGHT),
            fg_to_gpui(idx, &THEME_DARK)
        );
        assert_eq!(fg_to_gpui(idx, &THEME_LIGHT), rgb(0x1a7f2e));
        assert_eq!(fg_to_gpui(idx, &THEME_DARK), rgb(0x0dbc79));
    }
}

