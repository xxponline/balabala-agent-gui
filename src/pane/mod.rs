//! 终端面板 (TerminalPanel): main pane 渲染 + 键盘/IME 桥。
//!
//! change `split-ui-modules-dockarea` 4.1-4.3: 原 `ui/pane.rs` 自由函数 +
//! `app.rs` 的 `#terminal-pane` 整块收为 **TerminalPanel entity**
//! (`Panel` trait 面板; 当前先作为普通 child 挂回现有布局, 步骤 5 进
//! DockArea 中心)。焦点 (原 `TerminalApp::pane_focus`) 迁入本面板 —
//! 一步切焦点 (spec) 的目标; 键盘转发 (design D5) 的 listener 挂在此处,
//! 经 `WeakEntity<TerminalApp>` 调 model 方法 (4.3); `EntityInputHandler`
//! 实现留在 TerminalApp (trait 约束), canvas paint 内 weak 注册。
//!
//! (change: port-ui-to-gpui-kit 起源; 旧实现在 git history)

use gpui_kit::base::TestTextExt;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::*;
use vt100::Screen;

use crate::agent_entities::SessionStatus;
use crate::app::{TerminalApp, TerminalTab};
use crate::ui::{bg_to_gpui, fg_to_gpui};

/// 网格最小尺寸 (spec: Tiny pane clamps to minimum size; design D2)
pub const MIN_GRID_COLS: u16 = 20;
pub const MIN_GRID_ROWS: u16 = 5;

/// 把按键映射为终端字节 (与 spike 同一张表):
/// 可打印字符优先 key_char, 特殊键按名字。Ctrl/Cmd/Function 组合
/// 不在此列 (调用方负责过滤 — spec: Ctrl 组合不转发)。
/// GPUI keystroke → crossterm 键事件（通常 0/1 个；`key_char` 多字符时逐字）。
/// 映射语义与原 `keystroke_to_bytes` 一致：特殊键 + 可打印字符。
fn keystroke_to_keyevents(key: &str, key_char: Option<&str>) -> Vec<crossterm::event::KeyEvent> {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let special: Option<KeyCode> = match key {
        "enter" | "return" => Some(KeyCode::Enter),
        "backspace" => Some(KeyCode::Backspace),
        "tab" => Some(KeyCode::Tab),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "right" => Some(KeyCode::Right),
        "left" => Some(KeyCode::Left),
        _ => None,
    };
    if let Some(code) = special {
        return vec![KeyEvent::new(code, KeyModifiers::NONE)];
    }
    // 可打印字符: 优先平台给出的合成字符
    if let Some(c) = key_char {
        if !c.is_empty() {
            return c
                .chars()
                .map(|ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .collect();
        }
    }
    if key == "space" {
        return vec![KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)];
    }
    if key.chars().count() == 1 {
        if let Some(ch) = key.chars().next() {
            return vec![KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)];
        }
    }
    vec![]
}

/// 网格字体 (与 row/segment 的 text_size 同源)。
const GRID_FONT_FAMILY: &str = "Menlo";
const GRID_FONT_SIZE_PX: f32 = 13.;

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
        .child({
            let status_text = format!("[{:?}]", data.status);
            div()
                // F4 回归锚点: plain string 子节点不走 styled-text 记录路径,
                // texts() 只收录 test_text/ObservedText — 附件同文本文本
                .id("pane-status")
                .text_color(rgb(0x969696)) // 原版同款灰 (150,150,150)
                .child(status_text.clone())
                .test_text(status_text)
        })
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

/// 单元格宽度 (等宽字体 advance): 渲染分段定宽与后续自适应换算共用的
/// 唯一度量 (design D2)。对 `MMMMMMMMMM` 排版取均值, 避免单字形取整
/// 误差; 依赖排版缓存, 同一帧内重复调用返回相同值 (单测锁定)。
pub fn cell_width(window: &Window) -> Pixels {
    grid_metrics(window).0
}

/// 网格度量: (单元格宽, 行高) — 自适应换算与渲染共享的唯一真相 (design D2)。
///
/// 单元格宽 = Menlo 13px 的等宽 advance (排版取均值);
/// 行高 = 默认 line_height (phi×字号) 经 pixel_snap, 与 gpui 文本布局
/// `plain_line_height` 同式 — 必须等于 terminal_grid 行的实际绘制高度,
/// 否则 floor 换算会多算/少算一行 (单测断言二者相等)。
/// 依赖排版缓存, 同一帧内重复调用返回相同值 (单测锁定)。
pub fn grid_metrics(window: &Window) -> (Pixels, Pixels) {
    const SAMPLE: &str = "MMMMMMMMMM";
    let runs = [TextRun {
        len: SAMPLE.len(),
        font: font(GRID_FONT_FAMILY),
        ..Default::default()
    }];
    let layout = window
        .text_system()
        .layout_line(SAMPLE.into(), px(GRID_FONT_SIZE_PX), &runs, None);
    let cell_w = px(f32::from(layout.width) / SAMPLE.len() as f32);
    let mut style = TextStyle::default();
    style.font_size = AbsoluteLength::Pixels(px(GRID_FONT_SIZE_PX));
    let line_h = window.pixel_snap(
        style
            .line_height
            .to_pixels(style.font_size, window.rem_size()),
    );
    (cell_w, line_h)
}

/// 一段同色 (fg, bg) 的连续单元格。固定宽度契约: 段宽 =
/// `cols × cell_width`, 因此行总宽恒等于 `网格列数 × cell_width`。
struct Segment {
    text: String,
    fg: Rgba,
    bg: Rgba,
    /// 该段占的终端列数 (宽字符记 2)。
    cols: u16,
}

/// 把 vt100 网格按行折叠成同色段 (纯数据, 可单测; design D3/D4):
/// - `is_wide_continuation()` 续格跳过 — 宽字符的占位 cell 不贡献宽度,
///   否则每个汉字会把行尾多推一格 (CJK 漂移缺陷)
/// - `is_wide()` 首格记 2 列, 普通/空格记 1 列 (空 cell 用 nbsp 保证行高)
/// - 光标 cell 前景/背景互换 (静态反色块); 光标落在宽字符续格时
///   不单独绘制 — 首格的反色块已覆盖 2 格
/// - **宽字符立即独立成段** (F2 修复, 6.1 手动验收发现): gpui 对 CJK 字形
///   按半角推进 (实测 7.8px/字, 无全角归一), 若与后续同色 cell 合并,
///   段内文字排不满强制段宽 (2×cell/字), 空隙随字数累积堆在段尾 →
///   光标视觉后移 (每个中文字 +1 个半角 cell)。独立成段后: 每字独占
///   2-cell 盒, 字起点间距 = 2×cell (标准全角字距), 文字段总宽 = 段
///   强制宽, 光标紧贴文字尾 (仅剩字形侧边距)。
fn build_grid(screen: &Screen) -> Vec<Vec<Segment>> {
    let (rows_nz, cols_nz) = screen.size();
    let (rows, cols) = (rows_nz.get(), cols_nz.get());
    let cursor = screen.cursor_position();

    let default_fg = fg_to_gpui(vt100::Color::Default);
    let default_bg = bg_to_gpui(vt100::Color::Default);

    let mut out = Vec::with_capacity(usize::from(rows));
    for r in 0..rows {
        let mut segments: Vec<Segment> = Vec::new();
        let (mut cur_fg, mut cur_bg) = (default_fg, default_bg);
        let mut cur_text = String::new();
        let mut cur_cols = 0u16;

        let mut c = 0u16;
        while c < cols {
            let cell = screen.cell(r, c);
            if cell.is_some_and(vt100::Cell::is_wide_continuation) {
                c += 1;
                continue;
            }
            let cols_here = if cell.is_some_and(vt100::Cell::is_wide) { 2 } else { 1 };
            let (mut fg, mut bg) = match cell {
                Some(cell) => (fg_to_gpui(cell.fgcolor()), bg_to_gpui(cell.bgcolor())),
                None => (default_fg, default_bg),
            };
            // 未写入的 cell (ratatui 增量绘制下常见) 按真实终端的空格对待:
            // 用不换行空格 U+00A0 占位 (普通空格串可能被行盒裁剪, 空段会让
            // 行 div 无子元素而坍缩成 0 高 — 验收缺陷: 输入框贴顶显示)。
            let text: &str = match cell {
                Some(cell) if !cell.contents().is_empty() => cell.contents(),
                _ => "\u{00a0}",
            };

            // 光标: 前景/背景互换 (静态反色块)
            if cursor == (r, c) {
                std::mem::swap(&mut fg, &mut bg);
            }

            if (fg, bg) != (cur_fg, cur_bg) && !cur_text.is_empty() {
                segments.push(Segment {
                    text: std::mem::take(&mut cur_text),
                    fg: cur_fg,
                    bg: cur_bg,
                    cols: std::mem::take(&mut cur_cols),
                });
            }
            cur_fg = fg;
            cur_bg = bg;
            cur_text.push_str(text);
            cur_cols += cols_here;
            c += 1;
            // F2 修复: 宽字符立即独立成段 (见函数文档) — 否则同色合并段内
            // 文字按半角推进排不满全角段宽, 空隙随字数累积。
            if cols_here == 2 && !cur_text.is_empty() {
                segments.push(Segment {
                    text: std::mem::take(&mut cur_text),
                    fg: cur_fg,
                    bg: cur_bg,
                    cols: std::mem::take(&mut cur_cols),
                });
            }
        }
        if !cur_text.is_empty() {
            segments.push(Segment {
                text: cur_text,
                fg: cur_fg,
                bg: cur_bg,
                cols: cur_cols,
            });
        }
        out.push(segments);
    }
    out
}

/// 构建 vt100 网格 (design D6; cell-width 重构见 change
/// `add-cell-width-terminal-rendering`):
///
/// **固定宽度契约** — 每行渲染为同色段序列, 每段定宽
/// `cols × cell_width`, 行总宽恒等于 `网格列数 × cell_width`;
/// 度量来源是 [`cell_width`] (与后续 pane 自适应换算共享同一真相)。
/// 宽字符占 2 格、续格不单独绘制, 因此含 CJK/emoji 的行右边缘与
/// 纯 ASCII 行对齐 (spec: Terminal output rendering)。
///
/// 数据经 [`build_grid`] 从 screen 快照成拥有值 (元素树要求 'static);
/// 段盒 `overflow_hidden` + `whitespace_nowrap` — 字形墨迹溢出时裁剪
/// 而不换行/撑宽 (与真实终端一致)。
pub fn terminal_grid(screen: &Screen, window: &Window) -> Div {
    let cell_w = cell_width(window);
    let rows = build_grid(screen);

    div()
        .flex()
        .flex_col()
        // 行按内容收窄 (否则 align-items: stretch 会把行拉满容器,
        // 行宽就不再等于列数 × cell_width, 契约不可观测)
        .items_start()
        .font_family(GRID_FONT_FAMILY)
        .text_size(px(GRID_FONT_SIZE_PX))
        .children(rows.into_iter().enumerate().map(|(r, segments)| {
            div()
                .id(("grid-row", r))
                .test_support() // 无 test-support 特性时为 no-op
                .flex()
                .flex_row()
                .flex_shrink_0()
                .children(segments.into_iter().map(|seg| {
                    div()
                        .flex_shrink_0() // 宽度契约: 不允许被压缩
                        .w(px(seg.cols as f32 * f32::from(cell_w)))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_color(seg.fg)
                        .bg(seg.bg)
                        .child(seg.text)
                }))
        }))
}

#[cfg(test)]
mod tests {
    //! 行宽契约测试 (spec: Terminal output rendering —
    //! "row's rendered width equals logical column count × cell width")。
    //! 纯数据测试锁 build_grid 的列数账目; gpui 无头窗口测试锁真实
    //! div bounds (需 dev-dependency 的 test-support 特性)。
    //! 显式导入、不用 glob — 避免 `gpui_kit::*` 把 `#[test]` 宏遮蔽掉。

    use super::{
        GRID_FONT_FAMILY, GRID_FONT_SIZE_PX, build_grid, cell_width, grid_metrics, terminal_grid,
    };
    use crate::ui::{bg_to_gpui, fg_to_gpui};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Bounds, Context, InteractiveElement, IntoElement,
        ParentElement, Point, Render, TestAppContext, TestSupportExt, TextRun, Window, WindowBounds,
        WindowOptions, Styled, div, font, px, size,
    };
    use std::num::NonZeroU16;

    const ROWS: u16 = 30;
    const COLS: u16 = 100;
    /// 混合 ASCII/CJK + emoji 的验收 fixture (spec scenario 的输入)。
    const FIXTURE: &str = "> \u{5e2e} me\r\n\u{1f389} ok";

    fn parser_with(input: &str) -> vt100::Parser {
        let mut parser = vt100::Parser::new(
            NonZeroU16::new(ROWS).unwrap(),
            NonZeroU16::new(COLS).unwrap(),
            0,
        );
        parser.process(input.as_bytes());
        parser
    }

    fn row_sum(segs: &[super::Segment]) -> u16 {
        segs.iter().map(|s| s.cols).sum()
    }

    // ---------- 纯数据 (build_grid) ----------

    #[test]
    fn mixed_ascii_cjk_emoji_rows_sum_to_grid_columns() {
        let parser = parser_with(FIXTURE);
        // 宽字符必须走 is_wide 分支: 若续格被漏计/重计, 总和会偏离 COLS
        assert!(parser.screen().cell(0, 2).is_some_and(vt100::Cell::is_wide));
        let rows = build_grid(parser.screen());
        assert_eq!(rows.len(), usize::from(ROWS));
        for (r, segs) in rows.iter().enumerate() {
            assert_eq!(row_sum(segs), COLS, "row {r}: 段列数之和必须等于网格列数");
        }
        // 行 0 文本包含混合内容 (CJK 不被丢弃)
        let text: String = rows[0].iter().map(|s| s.text.as_str()).collect();
        assert!(text.contains('\u{5e2e}'), "row0 text: {text:?}");
    }

    #[test]
    fn cursor_on_wide_char_inverts_two_cells() {
        let mut parser = parser_with("\u{4f60}");
        parser.process(b"\x1b[1;1H"); // 光标移到 (0,0) = 宽字符首格
        assert_eq!(parser.screen().cursor_position(), (0, 0));
        let rows = build_grid(parser.screen());
        let segs = &rows[0];
        assert_eq!(row_sum(segs), COLS);
        // 反色段: bg 被换成默认前景色 (独立成段)
        let inv = segs
            .iter()
            .find(|s| s.bg == fg_to_gpui(vt100::Color::Default))
            .expect("cursor 应产生反色段");
        assert_eq!(inv.cols, 2, "宽字符上的光标反色块必须占 2 格");
        assert_eq!(inv.fg, bg_to_gpui(vt100::Color::Default));
        assert!(inv.text.contains('\u{4f60}'));
    }

    #[test]
    fn cursor_on_wide_continuation_not_painted_separately() {
        let mut parser = parser_with("\u{4f60}");
        parser.process(b"\x1b[1;2H"); // 光标移到 (0,1) = 续格
        assert_eq!(parser.screen().cursor_position(), (0, 1));
        let rows = build_grid(parser.screen());
        // 续格不贡献宽度、不产生反色段; F2 修复后宽字符独立成段 ->
        // [你][余下 nbsp] 两段, 且无任何反色段 (光标在续格上不可见, 原行为)
        assert_eq!(rows[0].len(), 2, "宽字符独立成段, 续格不产生额外段");
        assert_eq!(rows[0][0].text, "\u{4f60}");
        assert_eq!(rows[0][0].cols, 2);
        assert!(
            !rows[0].iter().any(|s| s.bg == fg_to_gpui(vt100::Color::Default)),
            "续格上的光标不应产生反色段"
        );
        assert_eq!(row_sum(&rows[0]), COLS);
    }

    /// F2 修复锁定: 每个宽字符独立成段 — 段起点按 cell 对齐, 文字段累计
    /// 列数 = 光标列 (光标紧贴文字尾, 空隙不再随中文字数累积)。
    #[test]
    fn wide_chars_are_isolated_segments_f2() {
        let mut parser = parser_with("\u{4f60}\u{597d}abc");
        parser.process(b"\x1b[1;8H"); // 光标移到文本后 (你0-1 好2-3 abc4-6 -> col7)
        assert_eq!(parser.screen().cursor_position(), (0, 7));
        let rows = build_grid(parser.screen());
        let segs = &rows[0];
        assert_eq!(segs[0].text, "\u{4f60}");
        assert_eq!(segs[0].cols, 2, "宽字符独立成段 (2 列)");
        assert_eq!(segs[1].text, "\u{597d}");
        assert_eq!(segs[1].cols, 2);
        assert_eq!(segs[2].text, "abc");
        assert_eq!(segs[2].cols, 3);
        let before_cursor: u16 = segs[..3].iter().map(|s| s.cols).sum();
        assert_eq!(
            before_cursor, 7,
            "文字段累计列必须等于光标列 (光标紧贴文字尾)"
        );
        assert_eq!(segs[3].cols, 1, "光标反色段紧随其后占 1 列");
        assert_eq!(row_sum(segs), COLS);
    }

    // ---------- 真实 div bounds (gpui 无头窗口) ----------

    /// 测试夹具: 持有 parser, render 时调用与生产完全相同的 terminal_grid。
    struct GridFixture {
        parser: vt100::Parser,
    }

    impl Render for GridFixture {
        fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(terminal_grid(self.parser.screen(), window))
        }
    }

    fn open_grid(cx: &mut TestAppContext, parser: vt100::Parser) -> AnyWindowHandle {
        cx.update(gpui_kit::init);
        cx.update(|cx| {
            let (handle, _) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(1200.), px(800.)),
                    })),
                    ..Default::default()
                },
                cx,
                |_window, cx| cx.new(|_| GridFixture { parser }),
            )
            .expect("open test window");
            handle
        })
    }

    #[gpui_kit::test]
    fn cell_width_is_sane_and_stable_within_a_frame(cx: &mut TestAppContext) {
        let handle = open_grid(cx, parser_with(FIXTURE));
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let a = f32::from(cell_width(window));
            let b = f32::from(cell_width(window));
            println!("cell_width(Menlo 13px) = {a}px (second call: {b}px)");
            assert_eq!(a, b, "同一帧内两次测量必须一致");
            assert!(
                (7.0..8.5).contains(&a),
                "Menlo 13px 等宽 advance 应约为 7-8px, 得到 {a}"
            );
        });
    }

    #[gpui_kit::test]
    fn row_div_width_equals_columns_times_cell_width(cx: &mut TestAppContext) {
        let handle = open_grid(cx, parser_with(FIXTURE));
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let cell_w = f32::from(cell_width(window));
            let expected = f32::from(COLS) * cell_w;
            for r in 0..usize::from(ROWS) {
                let row = window.find(("grid-row", r));
                let w = f32::from(row.bounds().size.width);
                let h = f32::from(row.bounds().size.height);
                assert!(
                    (w - expected).abs() < 0.01,
                    "row {r}: 宽度 {w} != 列数 × cell_width {expected}"
                );
                // 空行由 nbsp 撑住, 行盒不得坍缩成 0 高 (历史验收缺陷)
                assert!(h > 0., "row {r}: 高度坍缩为 {h}");
            }
        });
    }

    /// 1.2 spike 锁定: 度量在 paint 期可取、同帧稳定、量级合理,
    /// 且 line_h 必须等于实际绘制行高 (否则 floor 换算会多算/少算行)。
    #[gpui_kit::test]
    fn grid_metrics_sane_stable_and_match_painted_row_height(cx: &mut TestAppContext) {
        let handle = open_grid(cx, parser_with(FIXTURE));
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let (cw, lh) = grid_metrics(window);
            let (cw2, lh2) = grid_metrics(window);
            assert_eq!(
                (f32::from(cw), f32::from(lh)),
                (f32::from(cw2), f32::from(lh2)),
                "同帧内两次测量必须一致"
            );
            let (cw, lh) = (f32::from(cw), f32::from(lh));
            assert!((7.0..8.5).contains(&cw), "cell advance 应约 7-8px, 得 {cw}");
            assert!((10.0..30.0).contains(&lh), "行高应约 16-25px, 得 {lh}");
            // 换算基准必须等于实际绘制行高
            let row_h = f32::from(window.find(("grid-row", 0usize)).bounds().size.height);
            assert!(
                (row_h - lh).abs() <= 0.51,
                "line_h {lh} 应≈ 实际行高 {row_h}"
            );
        });
    }

    // ---------- 诊断 (手动): 真实 pi 会话 dump ----------

    /// 手动运行: `cargo test diagnostic_real_pi -- --ignored --nocapture`
    /// 注入已知文本, dump vt100 网格/光标/build_grid 段预测位置,
    /// 定位"光标落在文本中间"类缺陷在哪一层 (pi ANSI / vt100 / 渲染)。
    #[test]
    #[ignore]
    fn diagnostic_real_pi_cursor_layout() {
        use pi_shell_rs::app::embed::{EmbedConfig, EmbeddedApp};
        use std::time::{Duration, Instant};

        let embed = EmbeddedApp::spawn(EmbedConfig {
            pi_bin: None,
            pi_args: Vec::new(),
            cwd: Some(std::env::temp_dir()),
            cols: COLS,
            rows: ROWS,
            set_title: Some(Box::new(|_t: String| {})),
            copy_text: Some(Box::new(|_t: String| {})),
        })
        .expect("spawn real pi embed");

        let mut ansi = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            ansi.extend(embed.take_ansi());
            std::thread::sleep(Duration::from_millis(50));
        }
        embed.inject_text("\u{4f60}\u{597d}\u{1f60a}hahah");
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            ansi.extend(embed.take_ansi());
            std::thread::sleep(Duration::from_millis(50));
        }
        embed.shutdown();
        assert!(!ansi.is_empty(), "no ANSI from real pi");

        let mut parser = vt100::Parser::new(
            NonZeroU16::new(ROWS).unwrap(),
            NonZeroU16::new(COLS).unwrap(),
            1000,
        );
        parser.process(&ansi);
        let screen = parser.screen();

        let row = (0..ROWS).find(|&r| {
            (0..COLS).any(|c| screen.cell(r, c).is_some_and(|c| c.contents() == "\u{4f60}"))
        });
        let Some(row) = row else {
            eprintln!("no CJK row found; dumping all rows");
            return;
        };
        println!("== input row {row} cells ==");
        for c in 0..30 {
            let cell = screen.cell(row, c);
            let desc = match cell {
                Some(cell) => format!(
                    "{:?} wide={} cont={} inv={}",
                    cell.contents(),
                    cell.is_wide(),
                    cell.is_wide_continuation(),
                    cell.inverse()
                ),
                None => "<none>".into(),
            };
            println!("  col {c:2}: {desc}");
        }
        println!("cursor_position = {:?}", screen.cursor_position());

        let grid_rows = build_grid(screen);
        println!("== build_grid segments (predicted box x, cell_w=7.8) ==");
        let mut x = 0f32;
        for seg in &grid_rows[usize::from(row)] {
            let w = f32::from(seg.cols) * 7.8;
            println!(
                "  x {x:6.1}..{:<6.1} cols={} text={:?} bg={:?}",
                x + w,
                seg.cols,
                seg.text,
                seg.bg
            );
            x += w;
        }
    }

    /// 手动: `cargo test diagnostic_per_commit_cursor -- --ignored --nocapture`
    ///
    /// F2 定位 (6.1 手动发现): "每个中文字光标后移半角"。逐字符注入 —
    /// 与 IME commit 完全同路径 (send_text -> inject_text -> Char 键事件),
    /// 每次注入后 dump vt100 光标列增量:
    ///   delta = 2 (全角) / 1 (半角) -> 光标正确 (问题在渲染侧或用户场景)
    ///   delta = 1 (全角)            -> pi/inject 侧按字符数推进光标
    ///   delta = 0                   -> 注入没推动光标
    /// 同时打印光标 cell 是否落在宽字符续格 (我们的渲染跳过续格 -> 光标不可见)。
    #[test]
    #[ignore]
    fn diagnostic_per_commit_cursor() {
        use pi_shell_rs::app::embed::{EmbedConfig, EmbeddedApp};
        use std::time::{Duration, Instant};

        // 64 列 = 用户实际布局 (502px / 7.83 ≈ 64), 100 列是默认值
        const DIAG_COLS: u16 = 64;
        let embed = EmbeddedApp::spawn(EmbedConfig {
            pi_bin: None,
            pi_args: Vec::new(),
            cwd: Some(std::env::temp_dir()),
            cols: DIAG_COLS,
            rows: 30,
            set_title: Some(Box::new(|_t: String| {})),
            copy_text: Some(Box::new(|_t: String| {})),
        })
        .expect("spawn real pi embed");

        // 等启动 + 清空启动输出
        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            let _ = embed.take_ansi();
            std::thread::sleep(Duration::from_millis(50));
        }

        let mut parser = vt100::Parser::new(
            NonZeroU16::new(ROWS).unwrap(),
            NonZeroU16::new(DIAG_COLS).unwrap(),
            1000,
        );

        let dump = |parser: &vt100::Parser, label: &str| -> u16 {
            let screen = parser.screen();
            let cur = screen.cursor_position();
            let on_cont = screen
                .cell(cur.0, cur.1)
                .is_some_and(vt100::Cell::is_wide_continuation);
            let on_wide = screen.cell(cur.0, cur.1).is_some_and(vt100::Cell::is_wide);
            // 逐 cell dump 光标行 (对齐 batch 诊断的粒度)
            let mut cells = String::new();
            for c in 0..16 {
                let desc = match screen.cell(cur.0, c) {
                    Some(cell) if !cell.contents().is_empty() => format!("{c}:{:?} ", cell.contents()),
                    Some(_) => format!("{c}:. ",),
                    None => format!("{c}:x ",),
                };
                cells.push_str(&desc);
            }
            // 光标行最后一个非空 cell
            let mut last_text_col: i32 = -1;
            for c in 0..DIAG_COLS {
                if screen
                    .cell(cur.0, c)
                    .is_some_and(|c| !c.contents().is_empty() || c.is_wide_continuation())
                {
                    last_text_col = i32::from(c);
                }
            }
            println!(
                "[{label}] cursor={cur:?} 续格:{on_cont} 宽首格:{on_wide} text_end_col={last_text_col} (cursor-text_end={})",
                i32::from(cur.1) - last_text_col
            );
            println!("  row{}: {cells}", cur.0);
            cur.1
        };

        let mut prev_col = dump(&parser, "baseline");
        for ch in ["你", "好", "你", "好", "好", "吗", "h", "i"] {
            embed.inject_text(ch);
            std::thread::sleep(Duration::from_millis(700));
            let ansi = embed.take_ansi();
            parser.process(&ansi);
            let col = dump(&parser, &format!("after {ch:?}"));
            let delta = i32::from(col) - i32::from(prev_col);
            let expect = if ch.is_ascii() { 1 } else { 2 };
            println!(
                "  col delta {delta} (期望 {expect}) {}",
                if delta == expect { "OK" } else { "!! MISMATCH" }
            );
            prev_col = col;
        }
        // --- 阶段2: 方向键/退格与宽字符交互 (光标进入宽字符区) ---
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let screen_snapshot = |parser: &vt100::Parser| -> String {
            let screen = parser.screen();
            let (rows, cols) = screen.size();
            let mut all = String::new();
            for r in 0..rows.get() {
                for c in 0..cols.get() {
                    all.push_str(screen.cell(r, c).map(|c| c.contents()).unwrap_or(""));
                    all.push('\u{1}');
                }
                all.push('\u{2}');
            }
            all
        };
        for (label, code) in [
            ("Left", KeyCode::Left),
            ("Right", KeyCode::Right),
            ("Tab", KeyCode::Tab),
            ("Backspace", KeyCode::Backspace),
        ] {
            let before = screen_snapshot(&parser);
            embed.inject_key(KeyEvent::new(code, KeyModifiers::NONE));
            std::thread::sleep(Duration::from_millis(700));
            let ansi = embed.take_ansi();
            parser.process(&ansi);
            let after = screen_snapshot(&parser);
            let col = dump(&parser, &format!("key {label}"));
            println!(
                "  {label}: 整屏变化={} (光标行之外是否有任何反应)",
                if before != after { "YES" } else { "NO — pi 完全忽略该键" }
            );
            let _ = col;
        }

        embed.shutdown();
    }

    /// F2 机制探针: Menlo 13px 的 CJK 字形实际 advance vs 强制 cell 宽
    /// (2×cell_w)。若字形窄于 cell 宽 -> 同色文本段尾部留空 -> 光标
    /// (画在 cell 边界) 与可见文字尾之间出现空隙 = 用户看到的"光标落后"。
    #[gpui_kit::test]
    fn cjk_glyph_advance_vs_cell_width(cx: &mut TestAppContext) {
        let handle = open_grid(cx, parser_with(FIXTURE));
        let _ = cx.update_window(handle.into(), |_, window, _cx| {
            let (cell_w, _) = grid_metrics(window);
            let double = f32::from(cell_w) * 2.0;
            for sample in ["你", "AB"] {
                for (len_label, run_len) in [("bytes", sample.len()), ("chars", sample.chars().count())] {
                    let runs = [TextRun {
                        len: run_len,
                        font: font(GRID_FONT_FAMILY),
                        ..Default::default()
                    }];
                    let layout = window
                        .text_system()
                        .layout_line(sample.into(), px(GRID_FONT_SIZE_PX), &runs, None);
                    let width = f32::from(layout.width);
                    let adv = width / sample.chars().count() as f32;
                    let flag = if (adv - double).abs() < 0.3 {
                        "OK (=2×cell)"
                    } else if adv < double {
                        "!! 窄于 cell -> 段尾空隙 -> 光标视觉落后"
                    } else {
                        "!! 宽于 cell -> 溢出裁剪"
                    };
                    eprintln!(
                        "[F2-diag] {sample:?} len={len_label}({run_len}): total={width:.2}px per-char={adv:.2}px 2×cell_w={double:.2}px {flag}"
                    );
                }
            }
        });
    }

    /// F2 决定性探针: 走**真实渲染路径** (hug 宽度 div, 含字体 fallback)
    /// 量 "你好你好你好" 的整形宽度:
    ///   = 93.6px (6×15.6, fallback 归一到 2×cell) -> 段内无空隙, 理论无 gap
    ///   = 78px  (6×13, PingFang 自然宽)            -> 段尾空隙 15.6px = gap ✓假设
    ///   = 46.8px (6×7.8, 无 fallback/.notdef)      -> 字形重叠, 与截图不符
    #[gpui_kit::test]
    fn cjk_rendered_width_hug_div(cx: &mut TestAppContext) {
        struct Probe;
        impl Render for Probe {
            fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_start() // 关键: 强制子项内容拥抱 (否则 stretch 撑满)
                    .child(
                        div()
                            .id("probe-cjk")
                            .test_support()
                            .font_family(GRID_FONT_FAMILY)
                            .text_size(px(GRID_FONT_SIZE_PX))
                            .child("你好你好你好"),
                    )
            }
        }
        cx.update(gpui_kit::init);
        let (handle, _) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(600.), px(400.)),
                    })),
                    ..Default::default()
                },
                cx,
                |_window, cx| cx.new(|_| Probe),
            )
            .expect("open probe window")
        });
        let w = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let b = window.find("probe-cjk").bounds().size.width;
            f32::from(b)
        })
        .expect("update window");
        let cell = 7.8f32; // cell_width 在同帧稳定 (单测锁定)
        eprintln!(
            "[F2-render] 你好你好你好 实际渲染宽={w:.2}px | 6×15.6={} 6×13={} 6×7.8={}",
            6.0 * 15.6,
            6.0 * 13.0,
            6.0 * 7.8
        );
        let _ = cell;
    }
}

/// 中心终端面板: `#terminal-pane` 整块 + 焦点 + 键盘/IME 桥。
pub struct TerminalPanel {
    app: WeakEntity<TerminalApp>,
    /// 面板焦点 (原 TerminalApp::pane_focus 迁入; 4.1):
    /// `track_focus` 挂在 `#terminal-pane`, 一步切焦点/IME handler 均用它。
    focus: FocusHandle,
}

impl TerminalPanel {
    pub fn new(app: WeakEntity<TerminalApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app,
            focus: cx.focus_handle(),
        }
    }

    /// 键盘转发 (design D5, spike 已验证的机制):
    /// 可打印字符/控制键 → crossterm 键事件; 每次转发都 stop_propagation,
    /// 阻止 macOS 后端把同一个键再交给 input context (insertText
    /// 双发)。Ctrl/Cmd/Function 组合不转发也不阻止传播。
    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus.is_focused(window) {
            return;
        }
        let ks = &event.keystroke;
        if ks.modifiers.control || ks.modifiers.platform || ks.modifiers.function {
            return; // 平台快捷键 / IME 菜单照常工作 (spec: Ctrl 不转发)
        }
        let keys = keystroke_to_keyevents(&ks.key, ks.key_char.as_deref());
        if !keys.is_empty() {
            let app = self.app.clone();
            app.update(cx, |app, cx| {
                for key in keys {
                    app.send_key(key, cx);
                }
            })
            .ok();
            cx.stop_propagation(); // 防双发 (见上)
        }
    }
}

impl BasePanel for TerminalPanel {
    fn panel_name(&self) -> &'static str {
        "terminal"
    }
    fn closable(&self, _: &App) -> bool {
        false // 中心不可关 (spec: 主面板常驻)
    }
    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for TerminalPanel {
    fn title_bar(&self, _: &App) -> bool {
        false // D8: spec 状态条在面板内, 不加第二层标题
    }
    fn inner_padding(&self, _: &App) -> bool {
        false // D8: 面板自带 chrome
    }
}

impl EventEmitter<PanelEvent> for TerminalPanel {}

impl Focusable for TerminalPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        // 快照先行 (每帧现读 active session — design §7.2; 拥有值进元素树)
        let pane_data = app.read(cx).active_session().map(|session| {
            let s = session.read(cx);
            PaneData {
                // 用 session 自己的 ws_path (AgentEntity 契约)
                path: s.0.ws_path().to_string_lossy().to_string(),
                name: s.0.name().to_string(),
                status: s.0.status(),
                grid: terminal_grid(s.0.screen(), window),
            }
        });
        let focus = self.focus.clone();
        let app_weak = self.app.clone();
        div()
            .id("terminal-pane")
            .test_support() // 观测锚点 (无 test-support 特性时为 no-op)
            // 尺寸策略 (DockArea 装配后二分实测):
            // - **不能加 .flex().flex_col()** — gpui 对 flex 容器的子项
            //   按内容拥抱宽度 (780px grid 撑爆 520px 中心槽); 非 flex
            //   子项默认纵向堆叠 + 宽度填充槽位 (实测 520 ✓), 无需 flex_col。
            // - h_full: 高度填充 (否则高度拥抱内容, 窗口 resize 不再增行);
            // - min_w/min_h(0): 断开 min-content 反馈链 (t​​affy auto-min
            //   不看 overflow — 与 grid 的 min_w(0) 同款问题);
            // - overflow_hidden: 内容溢出裁剪 (spec: 溢出行按真实终端裁剪)。
            .overflow_hidden()
            .h_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .key_context("Terminal")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.handle_key_down(event, window, cx);
            }))
            .on_action(cx.listener(|this, _: &TerminalTab, _window, cx| {
                // Tab 被 Terminal context 的绑定截获 (原始 key
                // event 不再派发), 在动作里转发 — 单路径 (D5)
                let app = this.app.clone();
                app.update(cx, |app, cx| {
                    app.send_key(
                        crossterm::event::KeyEvent::new(
                            crossterm::event::KeyCode::Tab,
                            crossterm::event::KeyModifiers::NONE,
                        ),
                        cx,
                    );
                })
                .ok();
                cx.stop_propagation();
            }))
            .on_click(cx.listener(|this, _event: &ClickEvent, window, cx| {
                this.focus.focus(window, cx);
                cx.notify();
            }))
            .child(match pane_data {
                None => render_empty(),
                Some(data) => {
                    let starting = data.status == SessionStatus::Starting;
                    div()
                        .size_full()
                        .p_2()
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(0x4ba3d7)) // 原版同款蓝 (75,161,215)
                        .flex()
                        .flex_col()
                        .child(status_bar(&data))
                        .child(separator())
                        .children(starting.then(starting_placeholder))
                        .child(
                            div()
                                // 网格区域锚点: 自适应测量的 bounds
                                // 即此 div (canvas size_full 同界),
                                // 测试经此断言换算 (test_support 在
                                // 非测试构建下为 no-op)
                                .id("terminal-grid")
                                .test_support()
                                // 断开反馈环: 否则 flex 自动最小尺寸让
                                // 此 div 被网格内容撑大 (回退 100x30 =
                                // 780x630px), canvas 量到的是内容而非
                                // 可用面积, floor 捰好落回回退值后去重
                                // 永久锁死溢出。overflow != visible 时
                                // 自动最小尺寸为 0 -> bounds = 可用面积,
                                // 溢出行按真实终端裁剪 (spec: area
                                // available to the grid)
                                .overflow_hidden()
                                // 显式清零双轴自动最小尺寸: taffy 对
                                // cross 轴的 auto-min 不看 overflow,
                                // 内容仍会把宽度撑到 780px (反馈环残留)
                                .min_w(px(0.))
                                .min_h(px(0.))
                                .flex_1()
                                .w_full()
                                .child(data.grid)
                                .child(
                                    // IME 挂载点: paint 阶段注册
                                    // InputHandler (spike 同款);
                                    // EntityInputHandler 留 TerminalApp (4.3)
                                    canvas(
                                        move |bounds, _window, _cx| bounds,
                                        move |_bounds, pane_bounds, window, cx| {
                                            if let Some(app) = app_weak.upgrade() {
                                                window.handle_input(
                                                    &focus,
                                                    ElementInputHandler::new(pane_bounds, app),
                                                    cx,
                                                );
                                            }
                                            // 自适应 (spec: Adaptive terminal
                                            // grid sizing): 量网格区域 -> 换算
                                            // 列/行 (floor + 下限夹取) ->
                                            // defer 分发 (paint 阶段不能直接
                                            // 改实体)
                                            let (cell_w, line_h) = grid_metrics(window);
                                            let (w, h) = (
                                                f32::from(pane_bounds.size.width),
                                                f32::from(pane_bounds.size.height),
                                            );
                                            let clamp = |v: f32, min: u16| {
                                                (v.floor() as i64)
                                                    .clamp(min as i64, u16::MAX as i64)
                                                    as u16
                                            };
                                            let cols =
                                                clamp(w / f32::from(cell_w), MIN_GRID_COLS);
                                            let rows =
                                                clamp(h / f32::from(line_h), MIN_GRID_ROWS);
                                            let weak = app_weak.clone();
                                            cx.defer(move |cx| {
                                                if let Some(app) = weak.upgrade() {
                                                    app.update(cx, |app, cx| {
                                                        app.propagate_grid_size(
                                                            cols, rows, w, h, cell_w, line_h,
                                                            cx,
                                                        );
                                                    });
                                                }
                                            });
                                        },
                                    )
                                    .absolute()
                                    .size_full(),
                                ),
                        )
                }
            })
            .into_any_element()
    }
}

