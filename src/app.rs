//! 主 App 状态: 持有 workspaces 列表, 以 gpui entity 形式渲染 shell。
//!
//! 行为规格见 openspec/specs/{agent-shell,workspace-management}。
//! sidebar 3.x 接入, 终端面板 4.x 接入。

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;

use crate::agent_entities::{AgentSession, PiAgentShell, SessionStatus};
use crate::ui;
use crate::workspace::{Workspace, store};

// Tab 在 Terminal context 下的转发动作: gpui-base 的 Root 全局绑定
// `tab` → 焦点遍历会吞掉 Tab (原始 key event 不再派发), 更深的
// Terminal context 绑定胜出 (design D5, spike 已验证)。
actions!(terminal, [TerminalTab]);

/// 网格最小尺寸 (spec: Tiny pane clamps to minimum size; design D2)
const MIN_GRID_COLS: u16 = 20;
const MIN_GRID_ROWS: u16 = 5;

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

pub struct TerminalApp {
    /// 所有 workspace (每个持有一个路径 + 该路径下的 agents)
    pub workspaces: Vec<Workspace>,
    /// 当前在 main 区显示的 (workspace_idx, agent_idx)
    pub active: Option<(usize, usize)>,
    /// 最近一次持久化失败的错误信息 (在 sidebar 顶部显示)
    pub save_error: Option<String>,
    /// 主面板的键盘/IME 焦点 (design D4, 一步切焦点: 选中即聚焦)
    pub pane_focus: FocusHandle,
    /// 节流 drain 任务; drop 会取消循环 (design D3)
    _update_task: Option<Task<()>>,
    /// 进行中的异步文件夹选择任务 (design D8; 换新对话框时旧任务被取消)
    _dialog_task: Option<Task<()>>,
    /// 累计实际传播的网格 resize 次数 (诊断/测试观测: 输入型重绘不增)
    pub grid_resizes: usize,
}

impl TerminalApp {
    /// 启动时从 `.workspaces.json` 读取, 失败时返回 Err 让 main 退出。
    pub fn new(cx: &mut Context<Self>) -> Result<Self, String> {
        // Terminal context 下的 tab 终端转发绑定 (Root 的焦点遍历
        // 绑定在更浅的 context, 被这条压过 — design D5)
        cx.bind_keys([KeyBinding::new("tab", TerminalTab, Some("Terminal"))]);

        let paths = store::load()?;
        let workspaces = paths.into_iter().map(Workspace::new).collect();
        Ok(Self {
            workspaces,
            active: None,
            save_error: None,
            _update_task: None,
            _dialog_task: None,
            grid_resizes: 0,
            pane_focus: cx.focus_handle(),
        })
    }

    /// 点 workspace 行: 只切折叠状态。
    /// (spec: 不切 active, 不抢键盘焦点)
    pub fn toggle_collapse(&mut self, ws_idx: usize) {
        if let Some(ws) = self.workspaces.get_mut(ws_idx) {
            ws.collapsed = !ws.collapsed;
        }
    }

    /// 点 workspace 的 "×": 只在 workspace 为空时删除 + 立即持久化。
    pub fn close_workspace(&mut self, ws_idx: usize) {
        if ws_idx >= self.workspaces.len() {
            return;
        }
        if !self.workspaces[ws_idx].agents.is_empty() {
            return; // 还有 agent, 禁止删除
        }
        self.workspaces.remove(ws_idx);

        // 调整 active
        self.active = match self.active {
            None => None,
            Some((active_ws, _active_agent)) if active_ws == ws_idx => None,
            Some((active_ws, active_agent)) if active_ws > ws_idx => {
                Some((active_ws - 1, active_agent))
            }
            Some(x) => Some(x),
        };

        self.persist();
    }

    /// 用户点了某个 workspace 下的 "[+ agent]": 创建嵌入式 agent 会话
    /// (进程内 pi-shell-rs + pi --mode rpc),
    /// 命名 <dirname>/pi-N (每 workspace 独立计数), 并设为 active。
    pub fn create_agent(&mut self, ws_idx: usize, cx: &mut Context<Self>) {
        if ws_idx >= self.workspaces.len() {
            return;
        }
        let (name, cwd) = {
            let ws = &mut self.workspaces[ws_idx];
            (ws.new_agent_name(), ws.path.clone())
        };

        match PiAgentShell::new(name, cwd) {
            Ok(shell) => {
                let entity = cx.new(|_| AgentSession(Box::new(shell)));
                let ws = &mut self.workspaces[ws_idx];
                ws.agents.push(entity);
                let agent_idx = ws.agents.len() - 1;
                self.active = Some((ws_idx, agent_idx));
            }
            Err(e) => {
                eprintln!("failed to spawn agent: {e}");
            }
        }
    }

    /// 用户点了某个 agent 的 "×": hard kill 子进程 + 移除 + 调整 active。
    pub fn close_agent(&mut self, ws_idx: usize, agent_idx: usize, cx: &mut Context<Self>) {
        if ws_idx >= self.workspaces.len() {
            return;
        }
        let ws = &mut self.workspaces[ws_idx];
        if agent_idx >= ws.agents.len() {
            return;
        }

        // hard kill (spec: 立即终止子进程)
        let session = ws.agents.remove(agent_idx);
        session.update(cx, |s, _| s.0.shutdown());

        // 调整 active (与原版同款规则)
        let ws = &mut self.workspaces[ws_idx];
        self.active = match self.active {
            None => None,
            Some((a_ws, a_agent)) if a_ws == ws_idx && a_agent == agent_idx => {
                // 关掉的是 active: 切到同 workspace 内不超过新长度的最后一个
                if ws.agents.is_empty() {
                    None
                } else {
                    Some((ws_idx, ws.agents.len() - 1))
                }
            }
            Some((a_ws, a_agent)) if a_ws == ws_idx && a_agent > agent_idx => {
                Some((a_ws, a_agent - 1))
            }
            Some(x) => Some(x),
        };
    }

    /// 把一个键送给 active session。
    fn send_key(&mut self, key: crossterm::event::KeyEvent, cx: &mut Context<Self>) {
        let Some((ws_idx, agent_idx)) = self.active else {
            return;
        };
        let Some(session) = self
            .workspaces
            .get(ws_idx)
            .and_then(|w| w.agents.get(agent_idx))
        else {
            return;
        };
        session.update(cx, |s, _| s.0.send_key(key));
    }

    /// 把一段已提交文本（IME commit / 粘贴）送给 active session。
    fn send_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some((ws_idx, agent_idx)) = self.active else {
            return;
        };
        let Some(session) = self
            .workspaces
            .get(ws_idx)
            .and_then(|w| w.agents.get(agent_idx))
        else {
            return;
        };
        session.update(cx, |s, _| s.0.send_text(text));
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
        if !self.pane_focus.is_focused(window) {
            return;
        }
        let ks = &event.keystroke;
        if ks.modifiers.control || ks.modifiers.platform || ks.modifiers.function {
            return; // 平台快捷键 / IME 菜单照常工作 (spec: Ctrl 不转发)
        }
        let keys = keystroke_to_keyevents(&ks.key, ks.key_char.as_deref());
        if !keys.is_empty() {
            for key in keys {
                self.send_key(key, cx);
            }
            cx.stop_propagation(); // 防双发 (见上)
        }
    }

    /// 把 (cols, rows) 分发给所有会话 (Exited 会话自行跳过, spec:
    /// Grid resize propagation); 仅当至少一个会话实际改变尺寸时才打
    /// 日志并累加 `grid_resizes` — 未变尺寸的重绘零传播零日志 (去重
    /// 在 `AgentEntity::resize` 内按会话进行)。
    ///
    /// 输入: cols/rows — 目标网格尺寸 (已夹取下限); w/h — 面板像素面积;
    ///       cell_w/line_h — 度量值 (日志用); cx — 实体上下文
    /// 输出: 副作用为传播 resize + notify + 诊断日志
    fn propagate_grid_size(
        &mut self,
        cols: u16,
        rows: u16,
        w: f32,
        h: f32,
        cell_w: Pixels,
        line_h: Pixels,
        cx: &mut Context<Self>,
    ) {
        let mut resized = 0usize;
        for ws in &self.workspaces {
            for session in &ws.agents {
                if session.update(cx, |s, cx| {
                    if s.0.resize(cols, rows) {
                        cx.notify();
                        true
                    } else {
                        false
                    }
                }) {
                    resized += 1;
                }
            }
        }
        if resized > 0 {
            self.grid_resizes += resized;
            eprintln!(
                "[grid] pane {w:.0}x{h:.0}px -> {cols}x{rows} cells \
                 (cell_w {:.2}, line_h {:.2}); resized {resized} session(s)",
                f32::from(cell_w),
                f32::from(line_h)
            );
        }
    }

    /// 把当前 workspaces 的路径列表刷到 `.workspaces.json`,
    /// 失败时把错误存到 self.save_error (而不是 panic)。
    fn persist(&mut self) {
        let paths: Vec<PathBuf> = self.workspaces.iter().map(|w| w.path.clone()).collect();
        match store::save(&paths) {
            Ok(_) => self.save_error = None,
            Err(e) => self.save_error = Some(e),
        }
    }

    /// 用户点了 "[+ new workspace]": 弹 rfd **异步**文件夹选择器。
    /// (workspace-management spec: 取消不变 / canonicalize 去重 /
    ///  选中即追加并立即持久化)
    ///
    /// 不能用同步 `rfd::FileDialog`: 监听器执行期间 App 的可变借用
    /// 被 dispatch 持有, 模态循环阻塞期间 50ms drain 循环的
    /// `weak.update` 会重入借用 → RefCell panic (实测崩溃)。
    pub fn create_workspace(&mut self, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let picked = rfd::AsyncFileDialog::new().pick_folder().await;
            let Some(handle) = picked else {
                return; // 用户取消: sidebar 不变, 不重写持久化文件
            };

            let path = handle.path().to_path_buf();
            // canonicalize 方便去重
            let path = std::fs::canonicalize(&path).unwrap_or(path);

            this.update(cx, |app, cx| {
                if app.workspaces.iter().any(|w| w.path == path) {
                    eprintln!("workspace already exists: {}", path.display());
                    return; // 重复: log + 状态不变
                }
                app.workspaces.push(Workspace::new(path));
                app.persist();
                cx.notify();
            })
            .ok();
        });
        // 存住任务 (drop 会取消 future); 再次点击时旧任务被替换
        self._dialog_task = Some(task);
    }

    /// 启动 50ms 节流的 poll 循环 (design D3): 每 tick drain 所有
    /// session, 只在消费了事件时 notify (替代旧的每帧全量重绘)。
    pub fn start_update_loop(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        self._update_task = Some(cx.spawn(async move |_this, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let alive = weak
                    .update(cx, |app, cx| {
                        app.poll_sessions(cx);
                        true
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        }));
    }

    /// 遍历所有 session entity, poll 并按需 notify。
    fn poll_sessions(&mut self, cx: &mut Context<Self>) {
        for ws in &self.workspaces {
            for session in &ws.agents {
                session.update(cx, |s, cx| {
                    if s.0.poll() {
                        cx.notify();
                    }
                });
            }
        }
    }
}

/// IME 桥 (design D5): 平台 NSTextInputClient → 这些方法。
/// 只有 `replace_text_in_range` (最终提交/粘贴) 产生字节;
/// preedit (`replace_and_mark_text_in_range`) 按 spec 不转发。
impl EntityInputHandler for TerminalApp {
    fn replace_text_in_range(
        &mut self,
        _replacement_range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !text.is_empty() {
            self.send_text(text, cx);
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<std::ops::Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // preedit: spec 明确不转发
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// 不报告 marked text (spec: 无预编辑显示) — 后端据此判断
    /// 非组合态, 普通键走 key path (spike 验证过的路由)。
    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn text_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds) // IME 候选框定位在面板区域
    }

    fn selection_bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Vec<Bounds<Pixels>> {
        Vec::new()
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }

    fn set_selected_text_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    fn text_length_utf16(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

impl Render for TerminalApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // gpui 元素树要求拥有数据: 行数据先快照成值
        let mut ws_rows = Vec::with_capacity(self.workspaces.len());
        for (ws_idx, w) in self.workspaces.iter().enumerate() {
            let mut agents = Vec::new();
            if !w.collapsed {
                for (agent_idx, session) in w.agents.iter().enumerate() {
                    let s = session.read(cx);
                    agents.push(ui::sidebar::AgentRow {
                        ws_idx,
                        agent_idx,
                        name: s.0.name().to_string(),
                        status: s.0.status(),
                        is_active: self.active == Some((ws_idx, agent_idx)),
                    });
                }
            }
            ws_rows.push(ui::sidebar::WorkspaceRow {
                full_path: w.full_path_str(),
                display_name: w.display_name().to_string(),
                collapsed: w.collapsed,
                agent_count: w.agents.len(),
                agents,
            });
        }

        // active 面板的状态条数据 (4.1)
        let pane_data = match self.active {
            Some((ws_idx, agent_idx)) => self
                .workspaces
                .get(ws_idx)
                .and_then(|w| w.agents.get(agent_idx))
                .map(|session| {
                    let s = session.read(cx);
                    ui::pane::PaneData {
                        // 用 session 自己的 ws_path (AgentEntity 契约)
                        path: s.0.ws_path().to_string_lossy().to_string(),
                        name: s.0.name().to_string(),
                        status: s.0.status(),
                        grid: ui::pane::terminal_grid(s.0.screen(), window),
                    }
                }),
            None => None,
        };

        div()
            .size_full()
            .flex()
            .flex_row()
            // ---- 左边栏 (agent 行 3.3) ----
            .child(
                div()
                    .w(px(240.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(ui::sidebar::render(
                        self.save_error.clone(),
                        ws_rows,
                        cx,
                    )),
            )
            .child(div().w(px(1.)).h_full().bg(rgb(0x2a2a2a)))
            // ---- 右边主区: 终端面板 (4.1 chrome) ----
            // 一步切焦点: pane 持有 FocusHandle, 选中/点击即聚焦,
            // 4.3 的键盘/IME 转发挂在它上面 (key_context 已就位)。
            .child(
                div()
                    .id("terminal-pane")
                    // 与网格容器同理: root 是横排 flex, 此项主轴为水平,
                    // overflow visible 时自动最小尺寸=内容最小宽 (网格 780px)
                    // 会把整个 pane 撑宽并挤出窗口 — 裁剪切断该反馈链
                    .overflow_hidden()
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .key_context("Terminal")
                    .track_focus(&self.pane_focus)
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.handle_key_down(event, window, cx);
                    }))
                    .on_action(cx.listener(|this, _: &TerminalTab, _window, cx| {
                        // Tab 被 Terminal context 的绑定截获 (原始 key
                        // event 不再派发), 在动作里转发 — 单路径 (D5)
                        this.send_key(
                            crossterm::event::KeyEvent::new(
                                crossterm::event::KeyCode::Tab,
                                crossterm::event::KeyModifiers::NONE,
                            ),
                            cx,
                        );
                        cx.stop_propagation();
                    }))
                    .on_click(cx.listener(|this, _event: &ClickEvent, window, cx| {
                        this.pane_focus.focus(window, cx);
                        cx.notify();
                    }))
                    .child(match pane_data {
                        None => ui::pane::render_empty(),
                        Some(data) => {
                            let starting = data.status == SessionStatus::Starting;
                            // 面板框: 选中即蓝边 (spec "Active vs inactive
                            // pane stroke"; 未选中走 render_empty, 无框)
                            let focus = self.pane_focus.clone();
                            let view_entity = cx.entity();
                            div()
                                .size_full()
                                .p_2()
                                .bg(rgb(0xffffff))
                                .border_1()
                                .border_color(rgb(0x4ba3d7)) // 原版同款蓝 (75,161,215)
                                .flex()
                                .flex_col()
                                .child(ui::pane::status_bar(&data))
                                .child(ui::pane::separator())
                                .children(starting.then(ui::pane::starting_placeholder))
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
                                            // InputHandler (spike 同款)
                                            canvas(
                                                move |bounds, _window, _cx| bounds,
                                                move |_bounds, pane_bounds, window, cx| {
                                                    window.handle_input(
                                                        &focus,
                                                        ElementInputHandler::new(
                                                            pane_bounds,
                                                            view_entity.clone(),
                                                        ),
                                                        cx,
                                                    );
                                                    // 自适应 (spec: Adaptive terminal
                                                    // grid sizing): 量网格区域 -> 换算
                                                    // 列/行 (floor + 下限夹取) ->
                                                    // defer 分发 (paint 阶段不能直接
                                                    // 改实体)
                                                    let (cell_w, line_h) =
                                                        ui::pane::grid_metrics(window);
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
                                                    let entity = view_entity.clone();
                                                    cx.defer(move |cx| {
                                                        entity.update(cx, |app, cx| {
                                                            app.propagate_grid_size(
                                                                cols, rows, w, h, cell_w, line_h,
                                                                cx,
                                                            );
                                                        });
                                                    });
                                                },
                                            )
                                            .absolute()
                                            .size_full(),
                                        ),
                                )
                        }
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    //! 自适应网格分发的无头验证 (spec: Adaptive terminal grid sizing /
    //! Grid resize propagation): canvas paint 量出的 bounds 经 defer 分发,
    //! 换算与去重在真实渲染路径上成立。显式导入、不用 glob (避免
    //! `gpui_kit::*` 把 `#[test]` 宏遮蔽掉)。

    use super::{MIN_GRID_COLS, MIN_GRID_ROWS, TerminalApp};
    use crate::agent_entities::{AgentSession, PiAgentShell};
    use crate::ui::pane::grid_metrics;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
    };
    use std::fs;
    use std::io::Write as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 假 pi (与 tests/embed_layout.rs 同款), 供 stub 会话嵌入。
    fn write_stub(dir: &std::path::Path) -> std::path::PathBuf {
        let pwd_file = dir.join("pwd.txt");
        let script = format!(
            "#!/bin/sh\npwd > '{}'\nwhile IFS= read -r line; do\n  id=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\n  printf '{{\"type\":\"response\",\"id\":\"%s\",\"success\":true,\"data\":{{}}}}\\n' \"$id\"\ndone\n",
            pwd_file.display()
        );
        let path = dir.join("fake-pi.sh");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(script.as_bytes()).unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[gpui_kit::test]
    fn adaptive_grid_dispatch(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (handle, app_entity) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Point::default(),
                        size: size(px(1000.), px(600.)),
                    })),
                    ..Default::default()
                },
                cx,
                |_window, cx| {
                    cx.new(|cx| TerminalApp::new(cx).expect(".workspaces.json 加载"))
                },
            )
            .expect("open test window")
        });

        // 注入 stub 会话并激活 (绕过 create_agent: 它走真实 pi)
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("grid-dispatch-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let stub = write_stub(&dir);
        app_entity.update(cx, |app, cx| {
            assert!(
                !app.workspaces.is_empty(),
                "测试需要 .workspaces.json 中至少一个 workspace"
            );
            let shell = PiAgentShell::new_with_pi_bin(
                "pi-t".into(),
                std::env::temp_dir(),
                Some(stub.to_str().unwrap().to_string()),
            )
            .expect("stub embed spawn");
            let e = cx.new(|_| AgentSession(Box::new(shell)));
            app.workspaces[0].agents.push(e);
            app.active = Some((0, app.workspaces[0].agents.len() - 1));
        });

        // frame 1: paint 量 bounds -> defer 分发
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
        });
        cx.run_until_parked();

        // 读取分发结果 + 网格区域 bounds + 度量
        let (resizes1, screen) = app_entity.read_with(cx, |app, cx| {
            let s = app.workspaces[0].agents[0].read(cx);
            (app.grid_resizes, s.0.screen().size())
        });
        let (win_bounds, bounds, (cell_w, line_h)) = cx
            .update_window(handle.into(), |_, window, _cx| {
                let b = window.find("terminal-grid").bounds();
                (window.bounds(), b, grid_metrics(window))
            })
            .expect("update window");
        eprintln!(
            "[dbg] window {:?}x{:?} grid {}x{} cell_w {:.2} line_h {:.2}",
            f32::from(win_bounds.size.width),
            f32::from(win_bounds.size.height),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            f32::from(cell_w),
            f32::from(line_h),
        );

        // 换算必须与 paint 侧同一公式一致 (1.1 bounds 流通 + 3.1 分发)
        let clamp = |v: f32, min: u16| {
            (v.floor() as i64).clamp(min as i64, u16::MAX as i64) as u16
        };
        let exp_cols = clamp(f32::from(bounds.size.width) / f32::from(cell_w), MIN_GRID_COLS);
        let exp_rows = clamp(f32::from(bounds.size.height) / f32::from(line_h), MIN_GRID_ROWS);
        assert!(
            resizes1 >= 1,
            "首帧测量后应至少传播一次 resize, 得 {resizes1}"
        );
        assert_eq!(screen.1.get(), exp_cols, "会话列数 = floor(宽/cell_w)");
        assert_eq!(screen.0.get(), exp_rows, "会话行数 = floor(高/line_h)");
        // bounds 应排除 sidebar (240px) 与 chrome: 1000px 窗口下明显小于全宽
        assert!(
            f32::from(bounds.size.width) < 800.,
            "网格 bounds 应排除 sidebar 等 chrome: {}",
            f32::from(bounds.size.width)
        );

        // 3.2: 未变布局的重绘零传播 (计数不变 = 无 resize 调用)
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
        });
        cx.run_until_parked();
        let resizes2 = app_entity.read_with(cx, |app, _| app.grid_resizes);
        assert_eq!(
            resizes2, resizes1,
            "输入型重绘不得再传播 resize (去重)"
        );

        // 1.1: 窗口 resize -> canvas 量到的 bounds 跟踪面板 -> 再传播一次
        cx.simulate_window_resize(handle.into(), size(px(1200.), px(700.)));
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
        });
        cx.run_until_parked();
        let (resizes3, screen3) = app_entity.read_with(cx, |app, cx| {
            let s = app.workspaces[0].agents[0].read(cx);
            (app.grid_resizes, s.0.screen().size())
        });
        let bounds3 = cx
            .update_window(handle.into(), |_, window, _cx| {
                window.find("terminal-grid").bounds()
            })
            .expect("update window");
        eprintln!("[dbg] after resize: resizes {resizes1}->{resizes2}->{resizes3} bounds {}x{} -> {}x{}", f32::from(bounds.size.width), f32::from(bounds.size.height), f32::from(bounds3.size.width), f32::from(bounds3.size.height));
        assert!(resizes3 > resizes2, "窗口 resize 后必须再传播一次");
        assert!(
            f32::from(bounds3.size.width) > f32::from(bounds.size.width),
            "bounds 应随窗口变宽: {} -> {}",
            f32::from(bounds.size.width),
            f32::from(bounds3.size.width)
        );
        let exp_cols3 = clamp(f32::from(bounds3.size.width) / f32::from(cell_w), MIN_GRID_COLS);
        let exp_rows3 = clamp(f32::from(bounds3.size.height) / f32::from(line_h), MIN_GRID_ROWS);
        assert_eq!(screen3.1.get(), exp_cols3, "resize 后列数跟随新 bounds");
        assert_eq!(screen3.0.get(), exp_rows3, "resize 后行数跟随新 bounds");

        // 收尾: 关会话、清 stub
        app_entity.update(cx, |app, cx| {
            if let Some(e) = app.workspaces.get_mut(0).and_then(|w| w.agents.pop()) {
                e.update(cx, |s, _| s.0.shutdown());
            }
        });
        let _ = fs::remove_dir_all(&dir);
    }
}
