//! main pane 渲染 (gpui 版): 状态条 + 占位 + 分隔线。
//!
//! (change: port-ui-to-gpui-kit; 旧实现在 git history)
//! 4.2 在此叠加 vt100 网格; 焦点/点击在外层 `terminal-pane` div 上
//! (app.rs 持有 FocusHandle)。

use gpui_kit::*;
use vt100::Screen;

use crate::agent_entities::SessionStatus;
use crate::ui::{bg_to_gpui, fg_to_gpui};

/// active 面板的状态数据 (拥有值; gpui 元素树要求 'static)。
pub struct PaneData {
    /// 工作区绝对路径
    pub path: String,
    /// 会话名 (如 "foo/pi-1")
    pub name: String,
    pub status: SessionStatus,
    /// 已构建好的 vt100 网格 (渲染时从 screen 快照成拥有值)
    pub grid: Div,
}

/// 没有 active session 时的居中占位。
pub fn render_empty() -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .font_family("Menlo")
        .text_sm()
        .text_color(rgb(0x969696)) // 原版同款灰 (150,150,150)
        .child("no agent selected")
        .child("press + to create one")
}

/// 顶部状态条: `● <abs path> — <name>   [Status]`
/// (agent-shell spec: 状态条显示完整路径 + 前缀名 + 状态)
pub fn status_bar(data: &PaneData) -> Div {
    div()
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .child(
            div()
                .text_color(rgb(0x4ba3d7)) // 原版同款蓝 (75,161,215)
                .child(format!("● {} — {}", data.path, data.name)),
        )
        .child(
            div()
                .text_color(rgb(0x969696)) // 原版同款灰 (150,150,150)
                .child(format!("[{:?}]", data.status)),
        )
}

/// `Starting` 状态的占位文本 (首个字节到达后消失)。
pub fn starting_placeholder() -> Div {
    div()
        .px_3()
        .py_2()
        .font_family("Menlo")
        // 浅色主题: 比原版的 (229,193,16) 更深的琥珀色才可读 (D9)
        .text_color(rgb(0x8f6f00))
        .child("waiting for pi to start...")
}

/// 1px 水平分隔线 (浅色主题)。
pub fn separator() -> Div {
    div().w_full().h(px(1.)).bg(rgb(0xd9d9d9))
}

/// 构建 vt100 网格 (design D6): 逐行把 (fg, bg) 相同的相邻 cell 合并
/// 成分段, 每段渲染为带背景色的文本块; 光标位置的 cell 反色成块。
/// 数据在构建时从 screen 快照成拥有值 (元素树要求 'static)。
pub fn terminal_grid(screen: &Screen) -> Div {
    let (rows_nz, cols_nz) = screen.size();
    let (rows, cols) = (rows_nz.get(), cols_nz.get());
    let cursor = screen.cursor_position();

    let default_fg = fg_to_gpui(vt100::Color::Default);
    let default_bg = bg_to_gpui(vt100::Color::Default);

    let mut row_divs = Vec::with_capacity(usize::from(rows));
    for r in 0..rows {
        let mut segments: Vec<(String, Rgba, Rgba)> = Vec::new();
        let (mut cur_fg, mut cur_bg) = (default_fg, default_bg);
        let mut cur_text = String::new();

        for c in 0..cols {
            let Some(cell) = screen.cell(r, c) else {
                continue;
            };
            let mut fg = fg_to_gpui(cell.fgcolor());
            let mut bg = bg_to_gpui(cell.bgcolor());

            // 光标: 前景/背景互换 (静态反色块)
            if cursor == (r, c) {
                std::mem::swap(&mut fg, &mut bg);
            }

            if (fg, bg) != (cur_fg, cur_bg) && !cur_text.is_empty() {
                segments.push((std::mem::take(&mut cur_text), cur_fg, cur_bg));
            }
            cur_fg = fg;
            cur_bg = bg;
            cur_text.push_str(cell.contents());
        }
        if !cur_text.is_empty() {
            segments.push((cur_text, cur_fg, cur_bg));
        }

        row_divs.push(
            div()
                .flex()
                .flex_row()
                .children(
                    segments
                        .into_iter()
                        .map(|(text, fg, bg)| div().text_color(fg).bg(bg).child(text)),
                ),
        );
    }

    div()
        .flex()
        .flex_col()
        .font_family("Menlo")
        .text_size(px(13.))
        .children(row_divs)
}
