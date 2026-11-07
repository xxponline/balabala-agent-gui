//! main pane 渲染 (gpui 版): 状态条 + 占位 + 分隔线。
//!
//! (change: port-ui-to-gpui-kit; 旧实现在 git history)
//! 4.2 在此叠加 vt100 网格; 焦点/点击在外层 `terminal-pane` div 上
//! (app.rs 持有 FocusHandle)。

use gpui_kit::*;
use vt100::Screen;

use crate::agent_entities::SessionStatus;
use crate::ui::{bg_to_gpui, fg_to_gpui};

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

    use super::{build_grid, cell_width, grid_metrics, terminal_grid};
    use crate::ui::{bg_to_gpui, fg_to_gpui};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, AppContext, Bounds, Context, IntoElement, ParentElement, Point, Render,
        TestAppContext, Window, WindowBounds, WindowOptions, Styled, div, px, size,
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
        // 续格既不贡献宽度也不产生反色段: 整行默认色 -> 单段
        assert_eq!(rows[0].len(), 1, "续格上的光标不应产生额外段");
        assert_eq!(row_sum(&rows[0]), COLS);
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
}
