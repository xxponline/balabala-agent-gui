//! 主 App 状态: 持有 workspaces 列表, 以 gpui entity 形式渲染 shell。
//!
//! 行为规格见 openspec/specs/{agent-shell,workspace-management}。
//! sidebar 3.x 接入, 终端面板 4.x 接入。

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::*;

use crate::agent_entities::{AgentSession, PiAgentPty, SessionStatus};
use crate::ui;
use crate::workspace::{Workspace, store};

// Tab 在 Terminal context 下的转发动作: gpui-base 的 Root 全局绑定
// `tab` → 焦点遍历会吞掉 Tab (原始 key event 不再派发), 更深的
// Terminal context 绑定胜出 (design D5, spike 已验证)。
actions!(terminal, [TerminalTab]);

/// 把按键映射为终端字节 (与 spike 同一张表):
/// 可打印字符优先 key_char, 特殊键按名字。Ctrl/Cmd/Function 组合
/// 不在此列 (调用方负责过滤 — spec: Ctrl 组合不转发)。
fn keystroke_to_bytes(key: &str, key_char: Option<&str>) -> Option<Vec<u8>> {
    let special: Option<&[u8]> = match key {
        "enter" | "return" => Some(b"\r"),
        "backspace" => Some(b"\x7f"),
        "tab" => Some(b"\t"),
        "up" => Some(b"\x1b[A"),
        "down" => Some(b"\x1b[B"),
        "right" => Some(b"\x1b[C"),
        "left" => Some(b"\x1b[D"),
        _ => None,
    };
    if let Some(b) = special {
        return Some(b.to_vec());
    }
    // 可打印字符: 优先平台给出的合成字符
    if let Some(c) = key_char {
        if !c.is_empty() {
            return Some(c.as_bytes().to_vec());
        }
    }
    if key == "space" {
        return Some(b" ".to_vec());
    }
    if key.chars().count() == 1 {
        return Some(key.as_bytes().to_vec());
    }
    None
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

    /// 用户点了某个 workspace 下的 "[+ agent]": spawn pi 子进程,
    /// 命名 <dirname>/pi-N (每 workspace 独立计数), 并设为 active。
    pub fn create_agent(&mut self, ws_idx: usize, cx: &mut Context<Self>) {
        if ws_idx >= self.workspaces.len() {
            return;
        }
        let (name, cwd) = {
            let ws = &mut self.workspaces[ws_idx];
            (ws.new_agent_name(), ws.path.clone())
        };

        match PiAgentPty::new(name, "pi".to_string(), cwd) {
            Ok(pty) => {
                let entity = cx.new(|_| AgentSession(Box::new(pty)));
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

    /// 把字节送给 active session 的 pty stdin。
    fn send_input(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
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
        session.update(cx, |s, _| s.0.send_input(bytes));
    }

    /// 键盘转发 (design D5, spike 已验证的机制):
    /// 可打印字符/控制键 → 字节; 每次转发都 stop_propagation,
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
        if let Some(bytes) = keystroke_to_bytes(&ks.key, ks.key_char.as_deref()) {
            self.send_input(&bytes, cx);
            cx.stop_propagation(); // 防双发 (见上)
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
            let bytes = text.as_bytes().to_vec();
            self.send_input(&bytes, cx);
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                        grid: ui::pane::terminal_grid(s.0.screen()),
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
                        this.send_input(b"\t", cx);
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
                                                            view_entity,
                                                        ),
                                                        cx,
                                                    );
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
