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
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::*;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Mutex;
use vt100::Screen;

use crate::agent_entities::SessionStatus;
use crate::app::{TerminalApp, TerminalCopy, TerminalTab};
use crate::ui::{TerminalTheme, bg_to_gpui, fg_to_gpui, terminal_theme};

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
        // 历史回看翻页 (terminal-pointer-and-scroll-input, spec: Page keys
        // forwarded for history scrollback — pi 侧 KeyIntent::ScrollPage 消费)
        "pageup" => Some(KeyCode::PageUp),
        "pagedown" => Some(KeyCode::PageDown),
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

/// 网格等宽字体候选表 (change fix-windows-font-drift-and-input-lag, D1):
/// 按平台优先级排序, 取宿主实际安装的第一个。Menlo 仅 macOS 自带; Windows
/// 上缺 Menlo 时 gpui 按字形回退到非等宽字体 → cell_w 与实际 advance 不
/// 一致 → 行内漂移 (本次修复的根因)。
const GRID_FONT_FALLBACKS: &[&str] = &[
    "Menlo",            // macOS 自带等宽
    "Cascadia Mono",    // Windows 10+ 自带
    "Consolas",         // Windows 长期自带 (Office/.NET 捆绑)
    "Ubuntu Sans Mono", // Linux 常见
    "DejaVu Sans Mono", // Linux 通用兜底
];
const GRID_FONT_SIZE_PX: f32 = 13.;
/// 等宽校验容差 (D2): 四类代表字形的 advance 极差超过此值 → 启动诊断。
const MONOSPACE_TOLERANCE_PX: f32 = 0.5;

/// 已解析字体缓存 (D1 "resolved once"): 运行期首个字体表非空的解析结果
/// 进程级复用, 避免每帧枚举系统字体。测试构建 (cfg!(test)) 不读写 —
/// mock 文本系统 (NoopTextSystem) 的字体表为空, 若落缓存会毒化同一测试
/// 进程里后续的真实字体回归测试 (task 3.1)。
static GRID_FONT_CACHE: Mutex<Option<SharedString>> = Mutex::new(None);
/// 等宽校验去重 (D2): 每个字体族只在首次度量时校验一次。
static GRID_FONT_VERIFIED: Mutex<Option<SharedString>> = Mutex::new(None);

/// 网格字体唯一解析入口 (D1): 按 [`GRID_FONT_FALLBACKS`] 顺序取宿主已
/// 安装的第一个候选; 字体表不可用/全部缺失时退回首候选 — 该退化情形由
/// [`verify_grid_font_monospace`] 的启动诊断负责暴露 (D2, 永不硬失败)。
/// `grid_metrics` 与 terminal_grid 行渲染共用此函数, 度量与绘制不可能分歧。
fn grid_font_family(window: &Window) -> SharedString {
    if !cfg!(test) {
        if let Some(cached) = GRID_FONT_CACHE.lock().unwrap().clone() {
            return cached;
        }
    }
    let available = window.text_system().all_font_names();
    let resolved: SharedString = pick_grid_font_family(&available).into();
    if !cfg!(test) && !available.is_empty() {
        *GRID_FONT_CACHE.lock().unwrap() = Some(resolved.clone());
    }
    resolved
}

/// 候选表选择 (纯函数, 单测入口): 字体表按序命中首个已安装候选;
/// 空表 (mock/字体枚举不可用) 或全部缺失 → 首候选。
fn pick_grid_font_family(available: &[String]) -> &'static str {
    GRID_FONT_FALLBACKS
        .iter()
        .copied()
        .find(|candidate| available.iter().any(|name| name.as_str() == *candidate))
        .unwrap_or(GRID_FONT_FALLBACKS[0])
}

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

/// 没有 active session 时的居中占位 (`font` = 解析后的网格字体; 颜色随主题)。
pub fn render_empty(cx: &App, font: &str) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .font_family(font)
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child("no agent selected")
        .child("press + to create one")
}

/// 顶部状态条: `● <abs path> — <name>   [Status]`
/// (agent-shell spec: 状态条显示完整路径 + 前缀名 + 状态)
pub fn status_bar(data: &PaneData, cx: &App) -> Div {
    div()
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_1()
        .child(
            div()
                .text_color(cx.theme().accent)
                .child(format!("● {} — {}", data.path, data.name)),
        )
        .child({
            let status_text = format!("[{:?}]", data.status);
            div()
                // F4 回归锚点: plain string 子节点不走 styled-text 记录路径,
                // texts() 只收录 test_text/ObservedText — 附件同文本文本
                .id("pane-status")
                .text_color(cx.theme().muted_foreground)
                .child(status_text.clone())
                .test_text(status_text)
        })
}

/// `Starting` 状态的占位文本 (首个字节到达后消失)。
pub fn starting_placeholder(cx: &App, font: &str) -> Div {
    div()
        .px_3()
        .py_2()
        .font_family(font)
        // D9 的深琥珀字面量 → warning token (双主题可读, design D3)
        .text_color(cx.theme().warning)
        .child("waiting for pi to start...")
}

/// 1px 水平分隔线 (随主题)。
pub fn separator(cx: &App) -> Div {
    div().w_full().h(px(1.)).bg(cx.theme().border)
}

/// 单元格宽度 (等宽字体 advance): 渲染分段定宽与后续自适应换算共用的
/// 唯一度量 (design D2)。对 `MMMMMMMMMM` 排版取均值, 避免单字形取整
/// 误差; 依赖排版缓存, 同一帧内重复调用返回相同值 (单测锁定)。
pub fn cell_width(window: &Window) -> Pixels {
    grid_metrics(window).0
}

/// 网格度量: (单元格宽, 行高) — 自适应换算与渲染共享的唯一真相 (design D2)。
///
/// 单元格宽 = 解析后网格字体 ([`grid_font_family`]) 13px 的等宽 advance
/// (排版取均值); 首次度量时执行等宽校验 (design D2)。
/// 行高 = 默认 line_height (phi×字号) 经 pixel_snap, 与 gpui 文本布局
/// `plain_line_height` 同式 — 必须等于 terminal_grid 行的实际绘制高度,
/// 否则 floor 换算会多算/少算一行 (单测断言二者相等)。
/// 依赖排版缓存, 同一帧内重复调用返回相同值 (单测锁定)。
pub fn grid_metrics(window: &Window) -> (Pixels, Pixels) {
    const SAMPLE: &str = "MMMMMMMMMM";
    let family = grid_font_family(window);
    verify_grid_font_monospace(window, &family);
    let runs = [TextRun {
        len: SAMPLE.len(),
        font: font(family),
        ..Default::default()
    }];
    let layout = window
        .text_system()
        .layout_line(SAMPLE.into(), px(GRID_FONT_SIZE_PX), &runs, None);
    let cell_w = px(f32::from(layout.width) / SAMPLE.len() as f32);
    // D3 (task 2.1): 吸附到布局量化网格 — gpui 把盒宽/原点吸附到设备
    // 像素 (window.pixel_snap 同式), 格宽若不落网上, 每个盒被独立取整:
    // 宽字符段 (2×cell) 与单格段取整不一致 → 格边界错位 (漂移回来)。
    // 吸附后 cols × cell_w 恒为设备像素整数倍 → 绘制位置恰为
    // N × cell_w (±0.5px), 选区 overlay/自适应换算也同源对齐。
    let cell_w = window.pixel_snap(cell_w);
    let mut style = TextStyle::default();
    style.font_size = AbsoluteLength::Pixels(px(GRID_FONT_SIZE_PX));
    let line_h = window.pixel_snap(
        style
            .line_height
            .to_pixels(style.font_size, window.rem_size()),
    );
    (cell_w, line_h)
}

/// 等宽校验判定 (D2, 纯函数): 四类代表字形 (M/a/空格/—) 的 advance
/// 极差 > 容差 → 返回点名字体与各字形 advance 的诊断文本; 否则 None
/// (静默通过)。
fn monospace_diagnostic(family: &str, samples: &[(char, f32)], tolerance: f32) -> Option<String> {
    let mut iter = samples.iter().map(|&(_, advance)| advance);
    let first = iter.next()?;
    let (min, max) = iter.fold((first, first), |(min, max), a| (min.min(a), max.max(a)));
    if max - min <= tolerance {
        return None;
    }
    let list = samples
        .iter()
        .map(|(ch, a)| format!("{ch:?}={a:.2}px"))
        .collect::<Vec<_>>()
        .join(" ");
    Some(format!(
        "non-monospace grid font: {family:?} advance spread {:.2}px exceeds {tolerance}px tolerance at {}px: {list}",
        max - min,
        GRID_FONT_SIZE_PX
    ))
}

/// 等宽校验: 量解析字体 13px 下 M/a/空格/— 的 advance 并判定 (返回
/// 诊断文本; 等宽 → None)。与 Window 解耦 — 单测直接喂 stub 文本系统
/// 走完整测量路径 (task 1.2)。
fn grid_monospace_diagnostic(text_system: &TextSystem, family: &str) -> Option<String> {
    let font_id = text_system.resolve_font(&font(family));
    let samples: Vec<(char, f32)> = ['M', 'a', ' ', '—']
        .into_iter()
        .map(|ch| {
            (
                ch,
                f32::from(text_system.layout_width(font_id, px(GRID_FONT_SIZE_PX), ch)),
            )
        })
        .collect();
    monospace_diagnostic(family, &samples, MONOSPACE_TOLERANCE_PX)
}

/// 启动等宽校验 (D2, spec: Resolved font verified as monospace): 首次
/// 度量时执行 [`grid_monospace_diagnostic`], 有偏差 → eprintln! 诊断
/// (点名字体与发散值); 每字体族只查一次, 只告警不硬失败。
fn verify_grid_font_monospace(window: &Window, family: &SharedString) {
    {
        let mut done = GRID_FONT_VERIFIED.lock().unwrap();
        if done.as_deref() == Some(family.as_str()) {
            return;
        }
        *done = Some(family.clone());
    }
    if let Some(msg) = grid_monospace_diagnostic(window.text_system(), family) {
        eprintln!("[terminal-grid] {msg}");
    }
}

/// 一段同色 (fg, bg) 的连续单元格。固定宽度契约: 段宽 =
/// `cols × cell_width`, 因此行总宽恒等于 `网格列数 × cell_width`。
/// design D3: `cells` 逐格 (文本, 列宽) — 渲染成定宽内联格块, 字形
/// 位置 = 格索引 × cell_w, 不随前导字形 advance 漂移 (task 2.1)。
struct Segment {
    cells: Vec<(String, u16)>,
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
///   (D3 逐格定宽块落地后, 空隙累积已在块边界阻断 — 本捷径保留,由
///    现有 CJK 场景测试锁定, task 2.2: keep/simplify。)
fn build_grid(screen: &Screen, theme: &TerminalTheme) -> Vec<Vec<Segment>> {
    let (rows_nz, cols_nz) = screen.size();
    let (rows, cols) = (rows_nz.get(), cols_nz.get());
    let cursor = screen.cursor_position();

    let default_fg = fg_to_gpui(vt100::Color::Default, theme);
    let default_bg = bg_to_gpui(vt100::Color::Default, theme);

    let mut out = Vec::with_capacity(usize::from(rows));
    for r in 0..rows {
        let mut segments: Vec<Segment> = Vec::new();
        let (mut cur_fg, mut cur_bg) = (default_fg, default_bg);
        let mut cur_cells: Vec<(String, u16)> = Vec::new();
        let mut cur_cols = 0u16;
        // 待并空白列数 (字形不可见的 nbsp/空格) — 收段/遇可见格时一次性
        // 入账为单条目 (卡顿修复: 逐格建元素是每帧构建成本主因)
        let mut blank_run = 0u16;

        let mut c = 0u16;
        while c < cols {
            let cell = screen.cell(r, c);
            if cell.is_some_and(vt100::Cell::is_wide_continuation) {
                c += 1;
                continue;
            }
            let cols_here = if cell.is_some_and(vt100::Cell::is_wide) { 2 } else { 1 };
            let (mut fg, mut bg) = match cell {
                Some(cell) => (fg_to_gpui(cell.fgcolor(), theme), bg_to_gpui(cell.bgcolor(), theme)),
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

            // 空白格 (字形不可见) 并入待并空白段 — 典型终端屏多数格为
            // 空白, 合并后渲染为单盒 (宽 = k × cell_w, 设备网格上精确);
            // 记录文本 = k×nbsp, 测试按字符数推列 (task: 卡顿修复)。
            let is_blank = text == "\u{00a0}" || text == " ";
            if (fg, bg) != (cur_fg, cur_bg) && (!cur_cells.is_empty() || blank_run > 0) {
                if blank_run > 0 {
                    cur_cells.push(("\u{00a0}".repeat(blank_run as usize), blank_run));
                    blank_run = 0;
                }
                segments.push(Segment {
                    cells: std::mem::take(&mut cur_cells),
                    fg: cur_fg,
                    bg: cur_bg,
                    cols: std::mem::take(&mut cur_cols),
                });
            }
            cur_fg = fg;
            cur_bg = bg;
            if is_blank {
                blank_run += cols_here;
            } else {
                if blank_run > 0 {
                    cur_cells.push(("\u{00a0}".repeat(blank_run as usize), blank_run));
                    blank_run = 0;
                }
                cur_cells.push((text.to_string(), cols_here));
            }
            cur_cols += cols_here;
            c += 1;
            // F2 修复: 宽字符立即独立成段 (见函数文档) — 否则同色合并段内
            // 文字按半角推进排不满全角段宽, 空隙随字数累积。
            if cols_here == 2 && !cur_cells.is_empty() {
                segments.push(Segment {
                    cells: std::mem::take(&mut cur_cells),
                    fg: cur_fg,
                    bg: cur_bg,
                    cols: std::mem::take(&mut cur_cols),
                });
            }
        }
        if blank_run > 0 {
            cur_cells.push(("\u{00a0}".repeat(blank_run as usize), blank_run));
        }
        if !cur_cells.is_empty() {
            segments.push(Segment {
                cells: cur_cells,
                fg: cur_fg,
                bg: cur_bg,
                cols: cur_cols,
            });
        }
        out.push(segments);
    }
    out
}

/// D3 定宽格块 (task 2.1): 盒宽 = cols × cell_w, 字形只落自己盒内 →
/// 绘制位置 = 格索引 × cell_w, 前导字形 advance 无法累积漂移。
/// 测试构建额外用 `test_text` 记录逐格绘制原点 (2.1/2.3 断言);
/// 生产构建原样返回 — 不为测试锚点付出每格开销 (卡顿修复)。
#[cfg(not(test))]
fn grid_cell(text: String, cols: u16, cell_w: Pixels) -> Div {
    div()
        .flex_shrink_0()
        .w(px(cols as f32 * f32::from(cell_w)))
        .overflow_hidden()
        .whitespace_nowrap()
        .child(text)
}

#[cfg(test)]
fn grid_cell(text: String, cols: u16, cell_w: Pixels) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .w(px(cols as f32 * f32::from(cell_w)))
        .overflow_hidden()
        .whitespace_nowrap()
        .child(text.clone())
        .test_text(text)
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
pub fn terminal_grid(
    screen: &Screen,
    window: &Window,
    selection: Option<CellRange>,
    theme: &TerminalTheme,
) -> Div {
    let cell_w = cell_width(window);
    let rows = build_grid(screen, theme);

    div()
        .flex()
        .flex_col()
        // 行按内容收窄 (否则 align-items: stretch 会把行拉满容器,
        // 行宽就不再等于列数 × cell_width, 契约不可观测)
        .items_start()
        .font_family(grid_font_family(window))
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
                        .flex()
                        .flex_row()
                        .text_color(seg.fg)
                        .bg(seg.bg)
                        .children(
                            seg.cells
                                .into_iter()
                                .map(|(text, cols)| grid_cell(text, cols, cell_w)),
                        )
                }))
        }))
        // 选区 overlay (task 2.1): 行之后追加 = 画在文本上层;
        // 绝对定位相对 grid, rect = (col×cell_w, row×line_h) 纯数学
        .children(selection.map(|range| selection_overlay(range, screen, window)))
}

/// 选区高亮层 (task 2.1, spec: rendered as an overlay highlight covering
/// the cell rectangle): 按行分段, 首行从 anchor 列、末行到 focus 列、
/// 中间行整行; 半透明系统蓝。
fn selection_overlay(range: CellRange, screen: &Screen, window: &Window) -> Div {
    let cell_w = f32::from(cell_width(window));
    let line_h = f32::from(grid_metrics(window).1);
    let (_, cols_nz) = screen.size();
    let (start, end) = range.normalized();
    let full_w = f32::from(cols_nz.get()) * cell_w;
    div()
        .absolute()
        .size_full()
        .children((start.row..=end.row).map(|row| {
            let from = if row == start.row { start.col } else { 0 };
            let to = if row == end.row {
                end.col.saturating_add(1)
            } else {
                cols_nz.get()
            };
            let x = f32::from(from) * cell_w;
            let w = (f32::from(to) * cell_w).min(full_w) - x;
            div()
                .id(("sel-rect", usize::from(row)))
                .test_support() // 观测锚点 (无 test-support 时 no-op)
                .absolute()
                .left(px(x))
                .top(px(f32::from(row) * line_h))
                .w(px(w.max(0.)))
                .h(px(line_h))
                .bg(rgba(0x0078D959)) // 系统蓝 #0078D9 @ 35% (0xRRGGBBAA)
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
        GRID_FONT_SIZE_PX, build_grid, cell_width, grid_font_family, grid_metrics, terminal_grid,
    };
    use crate::ui::{THEME_DARK, THEME_LIGHT, bg_to_gpui, fg_to_gpui, terminal_theme};
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

    /// 段的拼接全文 (测试断言用; 生产渲染直接走 `cells` 逐格定宽块)。
    fn seg_text(seg: &super::Segment) -> String {
        seg.cells.iter().map(|(t, _)| t.as_str()).collect()
    }

    /// row r 的逐格绘制快照 (paint 顺序 = 列序)。D3 格块的 `test_text`
    /// 记录 bounds — 仅 test-support 构建可取 (生产无此开销)。
    fn row_cell_texts(
        window: &Window,
        r: usize,
    ) -> Vec<gpui_kit::base::test_support::TextSnapshot> {
        let row_id: gpui_kit::ElementId = ("grid-row", r).into();
        gpui_kit::base::test_support::texts(window)
            .into_iter()
            .filter(|t| t.path().last() == Some(&row_id))
            .collect()
    }

    // ---------- 纯数据 (build_grid) ----------

    /// 选区文本组装纯测 (terminal-text-selection task 1.2, spec:
    /// Copy selection — 尾空格去除/宽字符单次/闭区间单格非空)。
    #[test]
    fn selection_text_assembles_cells() {
        use super::{CellPos, CellRange, selection_text};

        // row0 = "ab   " (真实尾随空格), row1 = "> 中 ok" (CJK 宽字符)
        let p = parser_with("ab   \r\n\u{1b}[2;1H> \u{4e2d} ok\r\n");
        let screen = p.screen();

        // 单行含尾空格: 闭区间 (0,0)-(0,4) → trim_end → "ab"
        let r = CellRange {
            anchor: CellPos { row: 0, col: 0 },
            focus: CellPos { row: 0, col: 4 },
        };
        assert_eq!(selection_text(screen, r), "ab", "行尾随空格去除");

        // 单格: (1,2)-(1,2) 落在宽字符上 → 非空 (库 Equal 分支半开坑被 +1 绕开)
        let single = CellRange {
            anchor: CellPos { row: 1, col: 2 },
            focus: CellPos { row: 1, col: 2 },
        };
        assert_eq!(selection_text(screen, single), "\u{4e2d}", "单格选区非空且宽字符完整");

        // 多行跨 CJK: (0,1)-(1,6) → "b" + 换行 + "> 中 ok"
        let multi = CellRange {
            anchor: CellPos { row: 0, col: 1 },
            focus: CellPos { row: 1, col: 6 },
        };
        let text = selection_text(screen, multi);
        assert_eq!(text, "b\n> \u{4e2d} ok", "多行: 首行到尾去空、末行到 end 含入");
        assert_eq!(
            text.matches('\u{4e2d}').count(),
            1,
            "宽字符续格不产生重复 (spec: CJK appears once)"
        );

        // 反向 range 规范化后同结果
        let reversed = CellRange {
            anchor: CellPos { row: 1, col: 6 },
            focus: CellPos { row: 0, col: 1 },
        };
        assert_eq!(selection_text(screen, reversed), text, "反向选区结果一致");
    }

    /// 选区状态与换算纯测 (terminal-text-selection task 1.1, spec:
    /// Terminal area text selection — 规范化 + 出界 clamp)。
    #[test]
    fn selection_range_normalizes_and_clamps() {
        use super::{CellPos, CellRange, cell_at};

        // 反向拖选 (往上/左拖) → start ≤ end 规范化
        let reversed = CellRange {
            anchor: CellPos { row: 5, col: 8 },
            focus: CellPos { row: 2, col: 3 },
        };
        let (s, e) = reversed.normalized();
        assert_eq!((s, e), (CellPos { row: 2, col: 3 }, CellPos { row: 5, col: 8 }));

        // 正向不变; 单格 (anchor == focus)
        let single = CellRange {
            anchor: CellPos { row: 1, col: 1 },
            focus: CellPos { row: 1, col: 1 },
        };
        let (s, e) = single.normalized();
        assert_eq!((s, e), (CellPos { row: 1, col: 1 }, CellPos { row: 1, col: 1 }));

        // cell_at: 网格原点 (100,50), 格 8×20, 10 列 × 4 行
        let grid = Bounds {
            origin: Point::new(px(100.), px(50.)),
            size: size(px(80.), px(80.)),
        };
        // 格内: 相对 (24,25) → col=3 row=1
        assert_eq!(
            cell_at(Point::new(px(124.), px(75.)), grid, 8., 20., 10, 4),
            CellPos { row: 1, col: 3 }
        );
        // 负值 (点在网格左上外) → clamp 到 (0,0)
        assert_eq!(
            cell_at(Point::new(px(50.), px(10.)), grid, 8., 20., 10, 4),
            CellPos { row: 0, col: 0 },
            "spec: clamps to the grid edge"
        );
        // 超界 (右下外) → 最后一行/列
        assert_eq!(
            cell_at(Point::new(px(1000.), px(999.)), grid, 8., 20., 10, 4),
            CellPos { row: 3, col: 9 }
        );
        // 尾带: 网格宽 84px (10.5 格, floor 账目), rel=83 → 10 → min(9)=9
        let wide = Bounds {
            origin: Point::new(px(100.), px(50.)),
            size: size(px(84.), px(80.)),
        };
        assert_eq!(
            cell_at(Point::new(px(183.), px(69.)), wide, 8., 20., 10, 4),
            CellPos { row: 0, col: 9 },
            "尾带不越出最后一列"
        );

        // 度量未就绪防御
        assert_eq!(
            cell_at(Point::new(px(124.), px(75.)), grid, 0., 20., 10, 4),
            CellPos { row: 0, col: 0 }
        );
    }

    /// 哨兵判定纯测 (terminal-text-selection task 3.1, spec: Selection
    /// cleared when its content changes / survives unrelated updates)。
    #[test]
    fn selection_snapshot_detects_drift() {
        use super::selection_stale;

        // 快照未建立 (拖动中) → 不清
        assert!(!selection_stale("abc", &None), "未建立守卫不清 (拖动中不定界)");
        // 同文本 → 保留 (无关输出不误伤)
        assert!(
            !selection_stale("abc", &Some("abc".into())),
            "文本未变 → 保留"
        );
        // 漂移 → 清
        assert!(
            selection_stale("xyz", &Some("abc".into())),
            "文本被顶走 → 清 (spec: streaming push)"
        );
        // 空白选区守卫: 同空保留; 内容顶入则清
        assert!(!selection_stale("", &Some("".into())), "同空保留");
        assert!(selection_stale("x", &Some("".into())), "空白选区被内容顶入 → 清");
    }

    /// 滚轮换算纯测 (task 3.2, spec: Wheel deltas accumulate to whole
    /// lines / Remainder resets / Alt multiplies)。
    /// 符号约定: gpui 正 y (触摸板上滑) → 发负行数 = pi 的"上滚看更早"
    /// (standalone crossterm ScrollUp→-1 金标准; 2026-11-10 验收修正)。
    #[test]
    fn wheel_lines_for_accumulates_alt_and_resets() {
        use super::{WheelAccumulator, wheel_lines_for};
        use gpui_kit::{ScrollDelta, point, px};

        let pix = |y: f32| ScrollDelta::Pixels(point(px(0.), px(y)));
        let lines = |y: f32| ScrollDelta::Lines(gpui_kit::Point { x: 0.0, y });

        let mut acc = WheelAccumulator::default();
        // line_h 未测量 (0) → 拒绝一切输入 (防御)
        assert_eq!(wheel_lines_for(&mut acc, &pix(42.), 0.0, false), None);

        // 上滑 0.4 行/次 (gpui +y): 前两次 None (余数携带), 第三次 -1.2
        // → Some(-1) — 负号 = 看更早
        assert_eq!(wheel_lines_for(&mut acc, &pix(8.4), 21.0, false), None);
        assert_eq!(wheel_lines_for(&mut acc, &pix(8.4), 21.0, false), None);
        assert_eq!(
            wheel_lines_for(&mut acc, &pix(8.4), 21.0, false),
            Some(-1),
            "累积 -1.2 行 → 发 -1 行 (余 -0.2 携带, 负=上滚看更早)"
        );

        // Alt 倍率: +1 行 gpui → -1 → ×5 保号
        assert_eq!(
            wheel_lines_for(&mut acc, &pix(21.0), 21.0, true),
            Some(-(pi_shell_rs::app::scroll::ALT_WHEEL_MULTIPLIER as i16)),
            "Alt 持有一行 = ALT_WHEEL_MULTIPLIER 行 (保符号)"
        );

        // Lines 直通: gpui +0.3 → -0.3; 余 -0.2-0.3=-0.5 → None
        assert_eq!(wheel_lines_for(&mut acc, &lines(0.3), 21.0, false), None);

        // line_h 变化 → 余数清零 (spec: Remainder resets on line height
        // change): 清后 -0.7 单独 → None; 若未清则 -0.5-0.7=-1.2 → Some(-1)
        assert_eq!(wheel_lines_for(&mut acc, &lines(0.2), 24.0, false), None);
        assert_eq!(
            wheel_lines_for(&mut acc, &lines(0.7), 24.0, false),
            None,
            "换行高后余数从零起算 (-0.7 → None)"
        );
        assert_eq!(
            wheel_lines_for(&mut acc, &lines(0.4), 24.0, false),
            Some(-1),
            "-0.7-0.4=-1.1 → 发 -1"
        );
    }

    /// 翻页键映射 (terminal-pointer-and-scroll-input task 2.3, spec: Page
    /// keys forwarded for history scrollback)。
    #[test]
    fn page_keys_map_to_crossterm_paging() {
        use super::keystroke_to_keyevents;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        assert_eq!(
            keystroke_to_keyevents("pageup", None),
            vec![KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)],
            "pageup → crossterm PageUp"
        );
        assert_eq!(
            keystroke_to_keyevents("pagedown", None),
            vec![KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)],
            "pagedown → crossterm PageDown"
        );
    }

    #[test]
    fn mixed_ascii_cjk_emoji_rows_sum_to_grid_columns() {
        let parser = parser_with(FIXTURE);
        // 宽字符必须走 is_wide 分支: 若续格被漏计/重计, 总和会偏离 COLS
        assert!(parser.screen().cell(0, 2).is_some_and(vt100::Cell::is_wide));
        let rows = build_grid(parser.screen(), &THEME_LIGHT);
        assert_eq!(rows.len(), usize::from(ROWS));
        for (r, segs) in rows.iter().enumerate() {
            assert_eq!(row_sum(segs), COLS, "row {r}: 段列数之和必须等于网格列数");
        }
        // 行 0 文本包含混合内容 (CJK 不被丢弃)
        let text: String = rows[0].iter().map(seg_text).collect();
        assert!(text.contains('\u{5e2e}'), "row0 text: {text:?}");
    }

    #[test]
    fn cursor_on_wide_char_inverts_two_cells() {
        let mut parser = parser_with("\u{4f60}");
        parser.process(b"\x1b[1;1H"); // 光标移到 (0,0) = 宽字符首格
        assert_eq!(parser.screen().cursor_position(), (0, 0));
        let rows = build_grid(parser.screen(), &THEME_LIGHT);
        let segs = &rows[0];
        assert_eq!(row_sum(segs), COLS);
        // 反色段: bg 被换成默认前景色 (独立成段)
        let inv = segs
            .iter()
            .find(|s| s.bg == fg_to_gpui(vt100::Color::Default, &THEME_LIGHT))
            .expect("cursor 应产生反色段");
        assert_eq!(inv.cols, 2, "宽字符上的光标反色块必须占 2 格");
        assert_eq!(inv.fg, bg_to_gpui(vt100::Color::Default, &THEME_LIGHT));
        assert!(seg_text(inv).contains('\u{4f60}'));
    }

    #[test]
    fn cursor_on_wide_continuation_not_painted_separately() {
        let mut parser = parser_with("\u{4f60}");
        parser.process(b"\x1b[1;2H"); // 光标移到 (0,1) = 续格
        assert_eq!(parser.screen().cursor_position(), (0, 1));
        let rows = build_grid(parser.screen(), &THEME_LIGHT);
        // 续格不贡献宽度、不产生反色段; F2 修复后宽字符独立成段 ->
        // [你][余下 nbsp] 两段, 且无任何反色段 (光标在续格上不可见, 原行为)
        assert_eq!(rows[0].len(), 2, "宽字符独立成段, 续格不产生额外段");
        assert_eq!(seg_text(&rows[0][0]), "\u{4f60}");
        assert_eq!(rows[0][0].cols, 2);
        assert!(
            !rows[0].iter().any(|s| s.bg == fg_to_gpui(vt100::Color::Default, &THEME_LIGHT)),
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
        let rows = build_grid(parser.screen(), &THEME_LIGHT);
        let segs = &rows[0];
        assert_eq!(seg_text(&segs[0]), "\u{4f60}");
        assert_eq!(segs[0].cols, 2, "宽字符独立成段 (2 列)");
        assert_eq!(seg_text(&segs[1]), "\u{597d}");
        assert_eq!(segs[1].cols, 2);
        assert_eq!(seg_text(&segs[2]), "abc");
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
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(terminal_grid(
                    self.parser.screen(),
                    window,
                    None,
                    terminal_theme(cx),
                ))
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

    // ---------- mock 文本系统下的 cell-width 契约 (task 3.2) ----------
    // 以下测试跑在 NoopTextSystem (无头测试默认: 全 BMP 字形均匀
    // 0.6em = 7.8px) 上 — 锁定的是 div 宽度契约/同帧稳定性/行高等
    // 行为, 不对真实字体正确性提供任何判定 (spec: Mock-based tests are
    // not treated as font evidence)。真实字体判定归
    // `real_text_system_resolved_font_is_monospace` (task 3.1)。

    #[gpui_kit::test]
    fn cell_width_is_sane_and_stable_within_a_frame(cx: &mut TestAppContext) {
        let handle = open_grid(cx, parser_with(FIXTURE));
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let a = f32::from(cell_width(window));
            let b = f32::from(cell_width(window));
            let family = grid_font_family(window);
            println!("cell_width({family} 13px) = {a}px (second call: {b}px)");
            assert_eq!(a, b, "同一帧内两次测量必须一致");
            assert!(
                (7.0..8.5).contains(&a),
                "{family} 13px 等宽 advance 应约为 7-8px (mock 度量), 得到 {a}"
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

    /// add-light-dark-theme task 2.3: 双主题无头渲染验证 —
    /// (1) 默认格底 == 主题 `background` token (spec: no visible seam);
    /// (2) 索引色格随主题换调色板 (spec: Theme switch re-maps existing screen
    ///     的映射层);
    /// (3) render 路径两主题均出帧且 terminal_theme 随全局主题翻转。
    #[gpui_kit::test]
    fn grid_renders_under_both_themes_without_seam(cx: &mut TestAppContext) {
        use gpui_kit::component::theme::{Theme, ThemeMode};
        use gpui_kit::Rgba;

        // Rgba → u32 (round 到 8bit) — token 是 Hsla float 转换而来,
        // 直接 float 相等会因舍入误报, 统一到色值空间比较。
        let to_u32 = |c: Rgba| -> u32 {
            (((c.r * 255.).round() as u32) << 16)
                | (((c.g * 255.).round() as u32) << 8)
                | (c.b * 255.).round() as u32
        };

        // init 必须先于 Theme::change (ThemeRegistry 全局由它注册)
        cx.update(gpui_kit::init);

        // (1) 默认格底 == background token, 两主题各验一次 (无接缝)
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            cx.update(|cx| {
                Theme::change(mode, None, cx);
                let theme = terminal_theme(cx);
                let token_bg = to_u32(Theme::global(cx).background.into());
                let cell_bg = to_u32(bg_to_gpui(vt100::Color::Default, theme));
                assert_eq!(
                    cell_bg, token_bg,
                    "{mode:?}: 终端默认格底 {cell_bg:#08x} 必须等于 background token {token_bg:#08x}"
                );
            });
        }

        // (2) 索引色格两主题不同 (同一 screen, 仅换调色板)
        let mut parser = parser_with("");
        parser.process(b"\x1b[32mgreen\x1b[0m"); // 绿 = ANSI idx 2
        let seg_under = |theme: &crate::ui::TerminalTheme| -> u32 {
            let rows = build_grid(parser.screen(), theme);
            let seg = rows[0]
                .iter()
                .find(|s| seg_text(s).contains("green"))
                .expect("绿色文本段应存在");
            to_u32(seg.fg)
        };
        let light_green = seg_under(&THEME_LIGHT);
        let dark_green = seg_under(&THEME_DARK);
        assert_ne!(light_green, dark_green, "索引色必须随主题换调色板");
        assert_eq!(light_green, 0x1a7f2e);
        assert_eq!(dark_green, 0x0dbc79);

        // (3) render 路径两主题均出帧, terminal_theme 随全局主题翻转
        let handle = open_grid(cx, parser_with(FIXTURE));
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(terminal_theme(cx), &THEME_LIGHT);
            let h = f32::from(window.find(("grid-row", 0usize)).bounds().size.height);
            assert!(h > 0., "浅色主题下网格行应有高度");
        });
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(terminal_theme(cx), &THEME_DARK);
            let h = f32::from(window.find(("grid-row", 0usize)).bounds().size.height);
            assert!(h > 0., "深色主题下网格行应有高度");
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

    // ---------- D3 逐格定宽块位置 (task 2.1/2.3) ----------

    /// task 2.1 (spec: No horizontal drift — position(N) == N × cell_w):
    /// 混排行 (box-drawing 边框 + ASCII + 空格) 的每个格块绘制原点 =
    /// 列索引 × cell_w (±0.5px); 框线两端落在同一列 → 矩形。
    #[gpui_kit::test]
    fn cell_positions_equal_column_index_times_cell_width(cx: &mut TestAppContext) {
        // 两行混排: ┌ab ──┐ / └  ──┘ xy (box-drawing + ASCII + 空格)
        let input = "\u{250c}ab \u{2500}\u{2500}\u{2510}\r\n\u{2514}  \u{2500}\u{2518} xy";
        let handle = open_grid(cx, parser_with(input));
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let cell_w = f32::from(cell_width(window));
            for r in 0..2 {
                let cells = row_cell_texts(window, r);
                let row_x =
                    f32::from(window.find(("grid-row", r)).bounds().origin.x);
                // 逐记录推列: 可见格文本 1 字符 (fixture 无宽字符);
                // 空白盒文本 = k×nbsp → 覆盖 k 列 (空白已合并为单盒)
                let mut col = 0usize;
                for t in cells.iter() {
                    let x = f32::from(t.bounds().origin.x) - row_x;
                    assert!(
                        (x - col as f32 * cell_w).abs() <= 0.5,
                        "row {r} col {col}: 绘制原点 {x:.2}px != {col} × cell_w ({:.2}px)",
                        col as f32 * cell_w
                    );
                    col += t.text().chars().count();
                }
                assert_eq!(col, usize::from(COLS), "row {r}: 记录覆盖全部列");
            }
            // 框线矩形: ┌/┐ 同行两端 — ┐ 在 col6, 位置由上循环锁定
            let row0 = row_cell_texts(window, 0);
            assert_eq!(row0[0].text(), "\u{250c}", "左框线在 col0");
            assert_eq!(row0[6].text(), "\u{2510}", "右框线在 col6");
        });
    }

    /// task 2.3 (spec scenario: Cursor stays glued to typed text): 输入
    /// "abc" 后光标停在 col3 — 反色块 (光标格盒) 必须从 3 × cell_w
    /// 开始, 恰在最后一个字符后一格, 无空隙。
    #[gpui_kit::test]
    fn cursor_block_starts_one_cell_after_typed_text(cx: &mut TestAppContext) {
        let parser = parser_with("abc");
        assert_eq!(
            parser.screen().cursor_position(),
            (0, 3),
            "光标紧跟文字尾 (abc 后一格)"
        );
        let handle = open_grid(cx, parser);
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let cells = row_cell_texts(window, 0);
            let cell_w = f32::from(cell_width(window));
            let row_x = f32::from(window.find(("grid-row", 0usize)).bounds().origin.x);
            // 逐记录推列 (可见格 1 字符; 空白盒 = k×nbsp); 光标盒在 col3
            let mut col = 0usize;
            let mut cursor_x = None;
            for t in cells.iter() {
                let x = f32::from(t.bounds().origin.x) - row_x;
                assert!(
                    (x - col as f32 * cell_w).abs() <= 0.5,
                    "col {col}: {x:.2}px != {col} × cell_w"
                );
                if col == 3 {
                    // 光标格 (空 cell → nbsp, bg/fg 互换); 其盒即反色块本体
                    assert_eq!(t.text(), "\u{00a0}", "col3 = 光标空格盒");
                    cursor_x = Some(x);
                }
                col += t.text().chars().count();
            }
            assert_eq!(col, usize::from(COLS), "记录覆盖全部列");
            let cursor_x = cursor_x.expect("光标盒记录在场");
            assert!(
                (cursor_x - 3.0 * cell_w).abs() <= 0.5,
                "反色块必须从 3 × cell_w = {:.2}px 开始 (紧贴文字尾), 得 {cursor_x:.2}px",
                3.0 * cell_w
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
            // theme: None = 未注入默认路径 (add-light-dark-theme 4.3 验证)
            theme: None,
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

        let grid_rows = build_grid(screen, &THEME_LIGHT);
        println!("== build_grid segments (predicted box x, cell_w=7.8) ==");
        let mut x = 0f32;
        for seg in &grid_rows[usize::from(row)] {
            let w = f32::from(seg.cols) * 7.8;
            println!(
                "  x {x:6.1}..{:<6.1} cols={} text={:?} bg={:?}",
                x + w,
                seg.cols,
                seg_text(seg),
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
            // theme: None = 未注入默认路径 (add-light-dark-theme 4.3 验证)
            theme: None,
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
                        font: font(grid_font_family(window)),
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
            fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_start() // 关键: 强制子项内容拥抱 (否则 stretch 撑满)
                    .child(
                        div()
                            .id("probe-cjk")
                            .test_support()
                            .font_family(grid_font_family(window))
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

    /// task 1.1 (D1): 字体解析唯一入口 — 选择逻辑 + 源码形状锁定:
    /// pane 内 font()/.font_family() 不得再接字体族字符串字面量,
    /// 字体族只能来自 grid_font_family → pick_grid_font_family → 候选表。
    #[test]
    fn grid_font_resolution_is_the_only_font_source() {
        // 选择逻辑: 空表/全缺 → 首候选; 表内按序取首个已安装者
        assert_eq!(super::pick_grid_font_family(&[]), "Menlo");
        assert_eq!(super::pick_grid_font_family(&["Arial".to_string()]), "Menlo");
        assert_eq!(
            super::pick_grid_font_family(&["Consolas".to_string(), "Arial".to_string()]),
            "Consolas",
            "排在 Menlo 之后的候选在 Menlo 缺席时应命中"
        );
        assert_eq!(
            super::pick_grid_font_family(&["Menlo".to_string(), "Consolas".to_string()]),
            "Menlo",
            "按候选表顺序取首个已安装者"
        );

        // 源码形状: 调用点写死字面量 (历史 "Menlo" 回归) 时此测红
        let c = '"';
        let font_lit = format!("font({c}");
        let family_lit = format!(".font_family({c}");
        let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/pane/mod.rs"))
            .expect("读取 pane 源码");
        assert!(
            !src.contains(&font_lit),
            "font() 不得接字体族字面量 (唯一来源应为 grid_font_family): {font_lit}"
        );
        assert!(
            !src.contains(&family_lit),
            ".font_family() 不得接字体族字面量: {family_lit}"
        );
    }

    /// task 1.2 (D2): 等宽校验 — 四类字形 advance 极差超容差触发诊断
    /// (点名字体与发散值); 等宽时静默通过 (spec 两个 scenario)。
    #[test]
    fn non_monospace_spread_triggers_startup_diagnostic() {
        // 等宽 (mock Noop 文本系统实际行为: 全部 0.6em = 7.8px) → 静默
        let mono = [('M', 7.8f32), ('a', 7.8), (' ', 7.8), ('—', 7.8)];
        assert!(
            super::monospace_diagnostic("StubMono", &mono, super::MONOSPACE_TOLERANCE_PX)
                .is_none(),
            "等宽时不得产出诊断"
        );

        // 极差 4.2px > 0.5px 容差 → 诊断, 且点名字体/发散值/极差
        let bad = [('M', 7.8f32), ('a', 7.8), (' ', 7.8), ('—', 12.0)];
        let msg = super::monospace_diagnostic("Menlo", &bad, super::MONOSPACE_TOLERANCE_PX)
            .expect("非等宽必须触发诊断");
        assert!(msg.contains("Menlo"), "诊断必须点名字体: {msg}");
        assert!(msg.contains("12.00"), "诊断必须含发散字形 advance: {msg}");
        assert!(msg.contains("4.20"), "诊断必须含 advance 极差: {msg}");
    }

    /// task 1.2: 非等宽 stub 文本系统 — 除 layout_line 外全部委托
    /// NoopTextSystem; layout_line 模拟非等宽字体 (普通字形 0.6em、
    /// — 0.923em → 13px 下 7.8px vs 12.0px, 极差 4.2px > 0.5px 容差)。
    struct DriftingTextSystem(gpui_kit::NoopTextSystem);

    impl gpui_kit::PlatformTextSystem for DriftingTextSystem {
        fn add_fonts(&self, fonts: Vec<std::borrow::Cow<'static, [u8]>>) -> gpui_kit::Result<()> {
            self.0.add_fonts(fonts)
        }
        fn all_font_names(&self) -> Vec<String> {
            self.0.all_font_names()
        }
        fn font_id(&self, descriptor: &gpui_kit::Font) -> gpui_kit::Result<gpui_kit::FontId> {
            self.0.font_id(descriptor)
        }
        fn font_metrics(&self, font_id: gpui_kit::FontId) -> gpui_kit::FontMetrics {
            self.0.font_metrics(font_id)
        }
        fn typographic_bounds(
            &self,
            font_id: gpui_kit::FontId,
            glyph_id: gpui_kit::GlyphId,
        ) -> gpui_kit::Result<gpui_kit::Bounds<f32>> {
            self.0.typographic_bounds(font_id, glyph_id)
        }
        fn advance(
            &self,
            font_id: gpui_kit::FontId,
            glyph_id: gpui_kit::GlyphId,
        ) -> gpui_kit::Result<gpui_kit::Size<f32>> {
            self.0.advance(font_id, glyph_id)
        }
        fn glyph_for_char(&self, font_id: gpui_kit::FontId, ch: char) -> Option<gpui_kit::GlyphId> {
            self.0.glyph_for_char(font_id, ch)
        }
        fn glyph_raster_bounds(
            &self,
            params: &gpui_kit::RenderGlyphParams,
        ) -> gpui_kit::Result<gpui_kit::Bounds<gpui_kit::DevicePixels>> {
            self.0.glyph_raster_bounds(params)
        }
        fn rasterize_glyph(
            &self,
            params: &gpui_kit::RenderGlyphParams,
            raster_bounds: gpui_kit::Bounds<gpui_kit::DevicePixels>,
        ) -> gpui_kit::Result<(gpui_kit::Size<gpui_kit::DevicePixels>, Vec<u8>)> {
            self.0.rasterize_glyph(params, raster_bounds)
        }
        fn layout_line(
            &self,
            text: &str,
            font_size: gpui_kit::Pixels,
            _runs: &[gpui_kit::FontRun],
        ) -> gpui_kit::LineLayout {
            // 非等宽: 普通字形 0.6em, — 0.923em (13px → 7.8px vs 12.0px)
            let mut width = px(0.);
            for ch in text.chars() {
                width += if ch == '—' {
                    font_size * 0.923
                } else {
                    font_size * 0.6
                };
            }
            let metrics = self.0.font_metrics(gpui_kit::FontId(0));
            gpui_kit::LineLayout {
                font_size,
                width,
                ascent: font_size * (metrics.ascent / metrics.units_per_em as f32),
                descent: font_size * (metrics.descent / metrics.units_per_em as f32),
                runs: Vec::new(),
                len: text.len(),
            }
        }
        fn recommended_rendering_mode(
            &self,
            font_id: gpui_kit::FontId,
            font_size: gpui_kit::Pixels,
        ) -> gpui_kit::TextRenderingMode {
            self.0.recommended_rendering_mode(font_id, font_size)
        }
    }

    /// task 1.2 (spec 两个 scenario): 完整测量路径 — 非等宽 stub →
    /// 诊断触发且点名字体/发散值/极差; Noop (全 0.6em 等宽) → 静默。
    #[test]
    fn non_monospace_text_system_triggers_diagnostic_output() {
        let drifting = gpui_kit::TextSystem::new(std::sync::Arc::new(DriftingTextSystem(
            gpui_kit::NoopTextSystem::new(),
        )));
        let msg = super::grid_monospace_diagnostic(&drifting, "Menlo")
            .expect("非等宽 stub 必须触发诊断");
        assert!(msg.contains("Menlo"), "诊断必须点名字体: {msg}");
        assert!(msg.contains("12.00"), "诊断必须含 — 的发散 advance: {msg}");
        assert!(msg.contains("4.20"), "诊断必须含 advance 极差: {msg}");

        let mono = gpui_kit::TextSystem::new(std::sync::Arc::new(gpui_kit::NoopTextSystem::new()));
        assert!(
            super::grid_monospace_diagnostic(&mono, "Menlo").is_none(),
            "等宽 stub (Noop 全 0.6em) 必须静默通过"
        );
    }

    /// task 3.1 (spec: Real-font metrics regression coverage): 宿主真实
    /// 平台文本系统 (Windows = DirectWrite, 非 NoopTextSystem) 上的等宽
    /// 回归 — 解析字体的 M/a/空格/— 四类 advance 极差 ≤ 容差, 超差则
    /// 点名字体 panic, 使字体回退损坏无法通过 CI。macOS 平台创建需
    /// 主线程、Linux headless 语义不同 (gpui-base 同款注释) — 仅
    /// Windows 守卫; 打印 family/cell_w 亦为 task 1.3 提供宿主证据。
    #[cfg(target_os = "windows")]
    #[test]
    fn real_text_system_resolved_font_is_monospace() {
        // 非 headless: WindowsPlatform::new(false) 内建真实 DirectWrite
        // 文本系统 (headless 在 Windows 上是 Noop — gpui-base 注释)
        let platform = gpui_kit::platform::current_platform(false);
        let mut cx = TestAppContext::build_with_text_system(
            gpui_kit::TestDispatcher::new(0),
            None,
            platform.text_system(),
        );
        let handle = open_grid(&mut cx, parser_with(FIXTURE));
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let family = grid_font_family(window);
            let (cell_w, _) = grid_metrics(window); // 内含启动等宽校验
            let cell_w = f32::from(cell_w);
            let em = cell_w / GRID_FONT_SIZE_PX;
            println!("real-font: family={family} cell_w={cell_w:.2}px ({em:.3}em)");
            // 真实等宽字体 13px advance sanity (0.5-0.65em-class;
            // Consolas 0.55em / Cascadia Mono 0.6em / Menlo 0.602em)
            assert!(
                (6.0..9.0).contains(&cell_w),
                "真实等宽字体 13px advance 应约 6-9px (0.5-0.7em), 得 {cell_w:.2}px"
            );
            if let Some(msg) = super::grid_monospace_diagnostic(window.text_system(), &family) {
                panic!("宿主真实文本系统非等宽 (task 3.1): {msg}");
            }
        });
    }
}

/// 中心终端面板: `#terminal-pane` 整块 + 焦点 + 键盘/IME 桥。
pub struct TerminalPanel {
    app: WeakEntity<TerminalApp>,
    /// 面板焦点 (原 TerminalApp::pane_focus 迁入; 4.1):
    /// `track_focus` 挂在 `#terminal-pane`, 一步切焦点/IME handler 均用它。
    focus: FocusHandle,
    /// `#terminal-grid` 子元素的窗口坐标 bounds (task 3.1/4.1, design D4):
    /// origin 作点击换算减项, size 作越界判定 (spec: Clicks outside the
    /// grid area)。prepaint 期直写 (render 中不得 update 实体 → Rc<Cell>
    /// 裸闭包, gpui-component 库测试同款)。pub(crate): 测试/点击读。
    pub(crate) grid_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// 滚轮像素/行混合累积器 (task 3.2, spec: Wheel deltas accumulate to
    /// whole lines — 慢滚不丢、余数跨事件携带、line_h 变化清零)。
    wheel: WheelAccumulator,
    /// 文本选区 (terminal-text-selection task 2.1): 锚点/焦点 cell,
    /// overlay 与哨兵/复制共用。
    pub(crate) selection: Option<CellRange>,
    /// 当前左键按选手势 (task 2.1; on_click 消费 moved 后复位)。
    press: Option<Press>,
    /// 选区文本快照 (task 3.1, spec 哨兵): 定界时建立 (Some), 清选区时
    /// 一并清 (None = 未建立守卫, 拖动中不定界不比对)。
    selection_snapshot: Option<String>,
}

impl TerminalPanel {
    pub fn new(app: WeakEntity<TerminalApp>, cx: &mut Context<Self>) -> Self {
        Self {
            app,
            focus: cx.focus_handle(),
            grid_bounds: Rc::new(Cell::new(Bounds::default())),
            wheel: WheelAccumulator::default(),
            selection: None,
            press: None,
            selection_snapshot: None,
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

    /// 滚轮转发 (task 3.2, design D2/D3/D6): hover 归属 (不查焦点),
    /// 换算/累积/Alt 倍率经 `wheel_lines_for` 纯函数, 发整行给会话。
    /// 无论是否凑满行都 stop_propagation — 外层滚动容器永不接管
    /// (spec: Wheel input follows hover and stops propagation)。
    fn handle_scroll(
        &mut self,
        event: &ScrollEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let line_h = f32::from(grid_metrics(window).1);
        let Some(lines) = wheel_lines_for(&mut self.wheel, &event.delta, line_h, event.modifiers.alt)
        else {
            return; // 未凑满一行 (余数已携带)
        };
        let app = self.app.clone();
        app.update(cx, |app, cx| app.send_wheel(lines, cx)).ok();
    }

    /// Cmd+C 复制 (task 4.1, spec: Copy selection with Cmd+C): 先内联
    /// 哨兵判定 (design D3 — 帧到 notify 与 render 之间存在 defer 窗口,
    /// 复制前必须保证内容未漂), 漂了则清选区本次不复制; 同则写剪贴板。
    /// 无选区 → 不动剪贴板 (spec)。
    fn handle_copy(&mut self, cx: &mut Context<Self>) {
        let Some(live) = self.selection_text_now(cx) else {
            return; // 无选区
        };
        if selection_stale(&live, &self.selection_snapshot) {
            self.selection = None;
            self.selection_snapshot = None;
            cx.notify();
            return;
        }
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(live));
    }

    /// 当前选区的实时文本 (active session screen + selection_text;
    /// 无会话/无选区 → None)。哨兵比对与 Cmd+C 复制共用同一读取路径。
    pub(crate) fn selection_text_now(&self, cx: &App) -> Option<String> {
        let range = self.selection?;
        let app = self.app.upgrade()?;
        let app = app.read(cx);
        let session = app.active_session()?;
        Some(selection_text(session.read(cx).0.screen(), range))
    }

    /// 窗口坐标 → 选区 cell (task 2.1): grid_bounds 换算 + screen 行列
    /// 上限 clamp (spec: clamps to the grid edge)。无 active session 时
    /// 退化到 80×24 上限 (仍可选, 只是 clamp 边界保守)。
    fn cell_of(&self, pos: Point<Pixels>, window: &Window, cx: &App) -> CellPos {
        let grid = self.grid_bounds.get();
        let (cell_w, line_h) = grid_metrics(window);
        let (cols, rows) = self
            .app
            .upgrade()
            .and_then(|app| {
                let app = app.read(cx);
                app.active_session().map(|s| {
                    let (r, c) = s.read(cx).0.screen().size();
                    (c.get(), r.get())
                })
            })
            .unwrap_or((80, 24));
        cell_at(pos, grid, f32::from(cell_w), f32::from(line_h), cols, rows)
    }

    /// 左键单元格坐标转发 (task 4.1, design D4, spec: Terminal area click
    /// coordinates forwarded): 窗口坐标 − grid bounds.origin ÷ cell 尺寸;
    /// grid 外 (状态条/边框) 或非左键 → 不发。坐标系 = 会话帧 0-based,
    /// 命中在会话内部。
    fn forward_click_coordinate(
        &self,
        event: &ClickEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let ClickEvent::Mouse(mouse) = event else {
            return; // 键盘/触控点击不发坐标
        };
        if mouse.down.button != gpui_kit::MouseButton::Left {
            return;
        }
        let grid = self.grid_bounds.get();
        let rel_x = mouse.down.position.x - grid.origin.x;
        let rel_y = mouse.down.position.y - grid.origin.y;
        if rel_x < px(0.) || rel_y < px(0.) || rel_x >= grid.size.width || rel_y >= grid.size.height {
            return; // spec: outside the grid area → nothing
        }
        let (cell_w, line_h) = grid_metrics(window);
        let (cell_w, line_h) = (f32::from(cell_w), f32::from(line_h));
        if cell_w <= 0.0 || line_h <= 0.0 {
            return;
        }
        let col = (f32::from(rel_x) / cell_w) as u16;
        let row = (f32::from(rel_y) / line_h) as u16;
        // 尾带防护: bounds 可能比 cols×cell_w 宽出不到一格 (floor 账目,
        // 与 propagate_grid_size 同源), 超出整格数即越界
        if col >= (f32::from(grid.size.width) / cell_w) as u16
            || row >= (f32::from(grid.size.height) / line_h) as u16
        {
            return;
        }
        let app = self.app.clone();
        app.update(cx, |app, cx| app.send_click(col, row, cx)).ok();
    }
}

/// 选区单元格位置 (row, col — 行优先, 与 overlay 分行一致;
/// terminal-text-selection task 1.1)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct CellPos {
    pub row: u16,
    pub col: u16,
}

/// 选区范围: anchor = 按下锚点, focus = 拖动当前点。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct CellRange {
    pub anchor: CellPos,
    pub focus: CellPos,
}

impl CellRange {
    /// 规范化: 行列字典序 start ≤ end (spec: normalized so start
    /// precedes end, across rows)。反向拖选 (往上/左) 也得到合法区间。
    fn normalized(&self) -> (CellPos, CellPos) {
        let (a, f) = (self.anchor, self.focus);
        if (a.row, a.col) <= (f.row, f.col) {
            (a, f)
        } else {
            (f, a)
        }
    }
}

/// 一次左键按选手势 (task 2.1): `moved` 越过阈值即判为拖选 (design D2,
/// 与 TUI 点击坐标分离的依据)。
#[derive(Clone, Copy)]
struct Press {
    down_pos: Point<Pixels>,
    moved: bool,
}

/// 拖选阈值 (px): 小于此距离的位移仍是"点击"。
const DRAG_THRESHOLD_PX: f32 = 3.0;

/// 像素 → cell (窗口坐标; `grid` = 网格窗口 bounds; cols/rows = 网格
/// 行列上限)。出界 clamp (spec: clamps to the grid edge): 负值 → 0,
/// 超界/尾带 → 最后一行/列。`cell_at` 与点击坐标换算同源
/// (grid_bounds.origin + grid_metrics, design D4)。
fn cell_at(
    pos: Point<Pixels>,
    grid: Bounds<Pixels>,
    cell_w: f32,
    line_h: f32,
    cols: u16,
    rows: u16,
) -> CellPos {
    if cell_w <= 0.0 || line_h <= 0.0 || cols == 0 || rows == 0 {
        return CellPos { row: 0, col: 0 }; // 度量未就绪的防御退化
    }
    // 负值 → 0 行/列; 超界或尾带 (floor 账目) → min 到最后一行/列
    let clamp_axis = |rel: f32, cell: f32, cells: u16| -> u16 {
        if rel <= 0.0 {
            0
        } else {
            ((rel / cell) as u16).min(cells - 1)
        }
    };
    let col = clamp_axis(f32::from(pos.x - grid.origin.x), cell_w, cols);
    let row = clamp_axis(f32::from(pos.y - grid.origin.y), line_h, rows);
    CellPos { row, col }
}

/// 选区文本组装 (task 1.2; 哨兵与 Cmd+C 复制共用)。
///
/// 复用 vt100 官方 [`Screen::contents_between`] — 其文档明言为 clipboard
/// selection 设计: 软换行行不插 \n、宽字符续格跳过、行尾空 cell 不输出。
/// 闭区间语义靠尾列 +1 闭合 (库是半开边界; 单格 sc==ec 原生返回空, +1 绕开);
/// 再按行 trim_end (spec: trailing whitespace per row removed)。
fn selection_text(screen: &Screen, range: CellRange) -> String {
    let (start, end) = range.normalized();
    let raw = screen.contents_between(
        start.row,
        start.col,
        end.row,
        end.col.saturating_add(1),
    );
    raw.split('\n')
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 哨兵判定 (task 3.1, spec: Selection cleared when its content changes):
/// 快照未建立 (None, 拖动中) → 不清; 建立后实时文本与快照漂移 → 清。
fn selection_stale(live: &str, snapshot: &Option<String>) -> bool {
    snapshot.as_ref().is_some_and(|s| s != live)
}

/// 滚轮累积器 (task 3.2): 像素差按行高折行、余数跨事件携带、
/// line_h 变化时清零 (spec: Remainder resets on line height change)。
/// 纯数据 — 无窗口依赖, 单测直驱。
#[derive(Default)]
struct WheelAccumulator {
    /// 未凑成整行的余数 (行单位, 与整数部分同号)。
    remainder: f32,
    /// 当前行高 (px); 0 = 尚未测量, 拒绝一切输入。
    line_h: f32,
}

impl WheelAccumulator {
    fn set_line_h(&mut self, line_h: f32) {
        if self.line_h != line_h {
            self.line_h = line_h;
            self.remainder = 0.0; // 字号/度量变更 → 余数作废
        }
    }

    /// 累积一次输入 (行单位), 返回本次可发送的整数行 (向零取整)。
    fn add(&mut self, delta_lines: f32) -> i16 {
        if !delta_lines.is_finite() || !self.line_h.is_finite() || self.line_h <= 0.0 {
            return 0;
        }
        self.remainder += delta_lines;
        let whole = self.remainder.trunc();
        self.remainder -= whole;
        whole.clamp(i16::MIN as f32, i16::MAX as f32) as i16
    }
}

/// 滚轮事件 → 本次应发送的行数 (None = 未凑满整行)。spec: 像素累积 /
/// Line 直通 / Alt 倍率 (design D2/D3 — 换算与倍率全在宿主侧)。
fn wheel_lines_for(
    acc: &mut WheelAccumulator,
    delta: &ScrollDelta,
    line_h: f32,
    alt: bool,
) -> Option<i16> {
    acc.set_line_h(line_h);
    // 符号适配 (2026-11-10 headed 验收: 触摸板方向反了): gpui ScrollDelta
    // 的 y 符号与 pi 的行数约定相反 — 金标准是 pi standalone 的 crossterm
    // 路径 (ScrollUp → -1 = 上滚看更早), 此处把 gpui delta 翻转成同一约定。
    let delta_lines = match delta {
        ScrollDelta::Pixels(p) => -f32::from(p.y) / line_h,
        ScrollDelta::Lines(l) => -l.y,
    };
    let n = acc.add(delta_lines);
    if n == 0 {
        return None;
    }
    Some(if alt {
        n.saturating_mul(pi_shell_rs::app::scroll::ALT_WHEEL_MULTIPLIER as i16)
    } else {
        n
    })
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
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // 顶栏/多签 tab 的 console 标题 (tasks 2.1)。固定字面量而非
        // 随 session 名抖动; 未来要展示活动 session 名在此扩展。
        // test_support + id: 观测锚点 (try_find_by_label("console") 断言口;
        // test_support 要求既有 ElementId)。
        div().id("console-title").test_support().text_sm().child("console")
    }
    fn title_bar(&self, _: &App) -> bool {
        // true (2026-11-10 headed 验收追加): 顶栏常驻 — 单 console 签时
        // Auto 样式画 simple title 栏 (显示 console), 多签时为 tab 条;
        // 左右槽 title_bar=false 不受影响 (方案 B, 用户选定)。
        // 注意: 这不与 D8 冲突 — 状态条仍在面板内, 顶栏只是中槽的签/标题。
        true
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
        // 哨兵 (task 3.1, spec: Selection cleared when its content changes):
        // render = 帧到达路径 (帧到必 render); 实时选区文本 ≠ 快照 →
        // 内容被顶走/重排 → 本帧即清。无会话 (切走/空态) 同样失效。
        if self.selection.is_some() {
            let live = self.selection_text_now(cx);
            let stale = match &live {
                Some(live) => selection_stale(live, &self.selection_snapshot),
                None => true, // 无会话: 选区失效
            };
            if stale {
                self.selection = None;
                self.selection_snapshot = None;
            }
        }
        // 快照先行 (每帧现读 active session — design §7.2; 拥有值进元素树)
        let selection = self.selection;
        let theme = terminal_theme(cx);
        let pane_data = app.read(cx).active_session().map(|session| {
            let s = session.read(cx);
            PaneData {
                // 用 session 自己的 ws_path (AgentEntity 契约)
                path: s.0.ws_path().to_string_lossy().to_string(),
                name: s.0.name().to_string(),
                status: s.0.status(),
                grid: terminal_grid(s.0.screen(), window, selection, theme),
            }
        });
        let grid_font = grid_font_family(window);
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
            .on_action(cx.listener(|this, _: &TerminalCopy, _window, cx| {
                this.handle_copy(cx);
            }))
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
            // ---- 选区手势 (terminal-text-selection task 2.1, design D2) ----
            // 挂 pane (与 on_click 同层): hover 门更宽, 拖出网格由 cell_at
            // clamp 到边缘 (spec: clamps to the grid edge)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                let grid = this.grid_bounds.get();
                let p = event.position;
                let inside = p.x >= grid.origin.x
                    && p.y >= grid.origin.y
                    && p.x < grid.origin.x + grid.size.width
                    && p.y < grid.origin.y + grid.size.height;
                if !inside {
                    return; // 网格外按下: 不开手势 (旧选区不动)
                }
                this.press = Some(Press { down_pos: p, moved: false });
                this.selection = None; // 新按下清旧选区 (点击即取消选择)
                this.selection_snapshot = None; // 守卫随选区撤销
                cx.notify();
            }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                let Some(press) = this.press.as_mut() else {
                    return;
                };
                let dx = f32::from(event.position.x - press.down_pos.x);
                let dy = f32::from(event.position.y - press.down_pos.y);
                if !press.moved && (dx * dx + dy * dy).sqrt() > DRAG_THRESHOLD_PX {
                    press.moved = true;
                }
                if !press.moved {
                    return;
                }
                let down = press.down_pos;
                let focus_pos = event.position;
                let cell = this.cell_of(down, window, cx);
                this.selection = Some(CellRange {
                    anchor: cell,
                    focus: this.cell_of(focus_pos, window, cx),
                });
                cx.notify();
            }))
            // 出界收尾: up 不在 pane 内时 click 不会 fire, press 必须复位
            // (否则下次点击误判 dragged); 已扩展的选区保留
            .on_mouse_exit(cx.listener(|this, _event: &MouseExitEvent, _window, cx| {
                if this.press.take().is_some() {
                    cx.notify();
                }
            }))
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                // 一步聚焦保持 (spec: claims keyboard focus for the pane)
                this.focus.focus(window, cx);
                cx.notify();
                // 手势分离 (task 2.2, spec: Drag selection delivers no
                // coordinate): 本次手势拖过 (moved) 或选区仍在 (出界收尾后
                // press 已清但选区保留) → 拖选, 不发坐标; 纯点击维持转发。
                // down 时 selection 已被清 → is_some 即"本手势拖过"。
                let dragged =
                    this.press.map(|p| p.moved).unwrap_or(false) || this.selection.is_some();
                this.press = None; // 一次手势一个判定, 消费即复位
                if dragged {
                    // 定界: 建立快照守卫 (spec: snapshot while a selection
                    // is established) — 此后每帧比对
                    this.selection_snapshot = this.selection_text_now(cx);
                    return;
                }
                // 网格内左键 → 会话帧单元坐标 (task 4.1)
                this.forward_click_coordinate(event, window, cx);
            }))
            .child(match pane_data {
                None => render_empty(cx, &grid_font),
                Some(data) => {
                    let starting = data.status == SessionStatus::Starting;
                    div()
                        .size_full()
                        .p_2()
                        .bg(cx.theme().background)
                        .border_1()
                        .border_color(cx.theme().accent)
                        .flex()
                        .flex_col()
                        .child(status_bar(&data, cx))
                        .child(separator(cx))
                        .children(starting.then(|| starting_placeholder(cx, &grid_font)))
                        .child(
                            div()
                                // 子元素 prepaint bounds → 窗口原点直存
                                // (task 3.1): 点击坐标换算的减项; 裸闭包
                                // 不走 entity update, render 期安全。
                                // 必须在 .id() 之前挂 (inherent Div 方法,
                                // Stateful 包装后不可见)
                                .on_children_prepainted({
                                    let bounds = self.grid_bounds.clone();
                                    move |children, _window, _cx| {
                                        if let Some(b) = children.first() {
                                            bounds.set(*b);
                                        }
                                    }
                                })
                                // 滚轮 (task 3.2): 挂网格区域 = spec 的
                                // “terminal area” 界 (状态条上不滚); hover
                                // 归属由库的 should_handle_scroll 判定
                                .on_scroll(cx.listener(|this, event, window, cx| {
                                    this.handle_scroll(event, window, cx);
                                }))
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

