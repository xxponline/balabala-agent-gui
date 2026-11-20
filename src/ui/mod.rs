//! UI 层: vt100 颜色映射 (theme) + 共享小件 (widgets)。
//! 终端面板已迁 agent-service::terminal (task 4.1), sidebar 收为 crate::sidebar 面板
//! (change: split-ui-modules-dockarea)。
//!
//! **双主题颜色体系 (add-light-dark-theme)**:
//! - **调色板/默认色 (已迁 `agent-service::terminal`)**: `THEME_LIGHT` / `THEME_DARK` 两份
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
use pi_shell_rs::app::theme::TuiTheme;

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


#[cfg(test)]
mod tests {
    //! tui_theme 注入配色 (留在 GUI 侧 — 颜色真相服务宿主主题)。

    use super::*;


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

}
