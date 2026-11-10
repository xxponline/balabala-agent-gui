//! 主 App 状态: 持有 workspaces 列表, 以 gpui entity 形式渲染 shell。
//!
//! 行为规格见 openspec/specs/{agent-shell,workspace-management}。
//! sidebar 3.x 接入, 终端面板 4.x 接入。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::*;
use gpui_kit::component::dock::{DockArea, DockPlacement};
use gpui_kit::base::dock::PanelId;

use crate::agent_entities::{AgentSession, PiAgentShell};
use crate::pane::TerminalPanel;
use crate::property_detail_manager::{OpenObject, active_center_panel_id, handler_for};
use crate::shell;
use crate::sidebar::{SidebarEvent, SidebarPanel};
use crate::tree::TreePanel;
use crate::workspace::{Workspace, store};

// Tab 在 Terminal context 下的转发动作: gpui-base 的 Root 全局绑定
// `tab` → 焦点遍历会吞掉 Tab (原始 key event 不再派发), 更深的
// Terminal context 绑定胜出 (design D5, spike 已验证)。
actions!(terminal, [TerminalTab]);

pub struct TerminalApp {
    /// 所有 workspace (每个持有一个路径 + 该路径下的 agents)
    pub workspaces: Vec<Workspace>,
    /// 当前 active session 的稳定身份 (D4: gpui EntityId, 索引删除不错位)
    pub active: Option<EntityId>,
    /// 最近一次持久化失败的错误信息 (在 sidebar 顶部显示)
    pub save_error: Option<String>,
    /// DockArea 布局引擎 (design D1): 左|中|右三槽由它编排,
    /// 是 R3 外壳容器 (`render` 的根 div) 的子节点
    pub dock_area: Entity<DockArea>,
    /// sidebar 面板实体 (D2; notify 转发目标 — spike §10.5)
    pub sidebar: Entity<SidebarPanel>,
    /// 终端面板实体 (4.1; 焦点在其内, 一步切焦点/步骤 5 进 DockArea 中心)
    pub terminal: Entity<TerminalPanel>,
    /// 文件树面板实体 (right-dock-file-tree; notify 转发名单成员)
    pub tree: Entity<TreePanel>,
    /// 节流 drain 任务; drop 会取消循环 (design D3)
    _update_task: Option<Task<()>>,
    /// 进行中的异步文件夹选择任务 (design D8; 换新对话框时旧任务被取消)
    _dialog_task: Option<Task<()>>,
    /// 累计实际传播的网格 resize 次数 (诊断/测试观测: 输入型重绘不增)
    pub grid_resizes: usize,
    /// 累计经 send_key 转发给会话的 Tab 键数 (诊断/测试观测: spec
    /// Keyboard input forwarding - Tab 的端到端守卫; pi 收到 Tab 无可见
    /// 反应, 只能在转发边界计数验证 — 见 spike F3 结论)
    pub tab_forwards: usize,
    /// 默认打开系统的 dedup 表 (center-slot-multi-tab, design D1):
    /// key = 对象绝对路径字符串 (与 SourceNode.id 同一身份体系),
    /// value = 已开面板的 PanelId; 签关闭时经 `open_tab_removed` 摘除。
    pub(crate) open_tabs: HashMap<String, PanelId>,
}

impl TerminalApp {
    /// 启动时从 `.workspaces.json` 读取, 失败时返回 Err 让 main 退出。
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Result<Self, String> {
        // Terminal context 下的 tab 终端转发绑定 (Root 的焦点遍历
        // 绑定在更浅的 context, 被这条压过 — design D5)
        cx.bind_keys([KeyBinding::new("tab", TerminalTab, Some("Terminal"))]);

        let paths = store::load()?;
        let workspaces = paths.into_iter().map(Workspace::new).collect();
        // 装配 DockArea 三槽 (shell::build; panel_handle 包装见其文档)
        let weak_app = cx.weak_entity();
        let shell::ShellParts {
            dock_area,
            sidebar,
            terminal,
            tree,
        } = shell::build(weak_app, window, cx);
        Ok(Self {
            workspaces,
            active: None,
            save_error: None,
            _update_task: None,
            _dialog_task: None,
            grid_resizes: 0,
            tab_forwards: 0,
            open_tabs: HashMap::new(),
            dock_area,
            sidebar,
            terminal,
            tree,
        })
    }

    /// 点 workspace 行: 只切折叠状态。
    /// (spec: 不切 active, 不抢键盘焦点; 载荷 = PathBuf 稳定身份, D3)
    pub fn toggle_collapse(&mut self, path: &Path) {
        if let Some(ws) = self.workspaces.iter_mut().find(|w| w.path == path) {
            ws.collapsed = !ws.collapsed;
        }
    }

    /// 点 workspace 的 "×": 只在 workspace 为空时删除 + 立即持久化。
    pub fn close_workspace(&mut self, path: &Path) {
        let Some(ws_idx) = self.workspaces.iter().position(|w| w.path == path) else {
            return;
        };
        if !self.workspaces[ws_idx].agents.is_empty() {
            return; // 还有 agent, 禁止删除
        }
        self.workspaces.remove(ws_idx);

        // 无需调整 active (对比原版索引位移逻辑): EntityId 不随移位改变;
        // 且只有空 workspace 才能删 — active 不可能指向其中的会话。

        self.persist();
    }

    /// 用户点了某个 workspace 下的 "[+ agent]": 创建嵌入式 agent 会话
    /// (进程内 pi-shell-rs + pi --mode rpc),
    /// 命名 <dirname>/pi-N (每 workspace 独立计数), 并设为 active。
    pub fn create_agent(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(ws_idx) = self.workspaces.iter().position(|w| w.path == path) else {
            return;
        };
        let (name, cwd) = {
            let ws = &mut self.workspaces[ws_idx];
            (ws.new_agent_name(), ws.path.clone())
        };

        match PiAgentShell::new(name, cwd) {
            Ok(shell) => {
                let entity = cx.new(|_| AgentSession(Box::new(shell)));
                self.active = Some(entity.entity_id());
                self.workspaces[ws_idx].agents.push(entity);
            }
            Err(e) => {
                eprintln!("failed to spawn agent: {e}");
            }
        }
    }

    /// 用户点了某个 agent 的 "×": hard kill 子进程 + 移除 + 调整 active。
    /// (载荷 = EntityId 稳定身份, D3: 不因兄弟删除错位)
    pub fn close_agent(&mut self, id: EntityId, cx: &mut Context<Self>) {
        let Some(ws_idx) = self
            .workspaces
            .iter()
            .position(|w| w.agents.iter().any(|e| e.entity_id() == id))
        else {
            return;
        };
        let ws = &mut self.workspaces[ws_idx];
        let Some(agent_idx) = ws.agents.iter().position(|e| e.entity_id() == id) else {
            return;
        };

        // hard kill (spec: 立即终止子进程)
        let session = ws.agents.remove(agent_idx);
        let removed_id = session.entity_id();
        session.update(cx, |s, _| s.0.shutdown());

        // 关掉的是 active -> 切到同 workspace 内最后一个 / 无则 None
        // (对比原版: EntityId 不因兄弟移除而错位, 其余情况 active 无需调整)
        if self.active == Some(removed_id) {
            self.active = self.workspaces[ws_idx].agents.last().map(|e| e.entity_id());
        }
    }

    /// 按稳定 id 找 active session 实体 (D4: 遍历即可, 规模小)。
    pub(crate) fn active_session(&self) -> Option<&Entity<AgentSession>> {
        let active = self.active?;
        self.workspaces
            .iter()
            .flat_map(|w| w.agents.iter())
            .find(|e| e.entity_id() == active)
    }

    /// 把一个键送给 active session。
    pub(crate) fn send_key(&mut self, key: crossterm::event::KeyEvent, cx: &mut Context<Self>) {
        if key.code == crossterm::event::KeyCode::Tab {
            self.tab_forwards += 1; // 诊断/测试观测 (spec: Tab 转发边界)
        }
        let Some(session) = self.active_session() else {
            return;
        };
        session.update(cx, |s, _| s.0.send_key(key));
    }

    /// 把一段已提交文本（IME commit / 粘贴）送给 active session。
    fn send_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        session.update(cx, |s, _| s.0.send_text(text));
    }

    /// 把 (cols, rows) 分发给所有会话 (Exited 会话自行跳过, spec:
    /// Grid resize propagation); 仅当至少一个会话实际改变尺寸时才打
    /// 日志并累加 `grid_resizes` — 未变尺寸的重绘零传播零日志 (去重
    /// 在 `AgentEntity::resize` 内按会话进行)。
    ///
    /// 输入: cols/rows — 目标网格尺寸 (已夹取下限); w/h — 面板像素面积;
    ///       cell_w/line_h — 度量值 (日志用); cx — 实体上下文
    /// 输出: 副作用为传播 resize + notify + 诊断日志
    pub(crate) fn propagate_grid_size(
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
                app.notify_panels(cx); // 面板自读不重建 (spike §10.5), 显式转发
            })
            .ok();
        });
        // 存住任务 (drop 会取消 future); 再次点击时旧任务被替换
        self._dialog_task = Some(task);
    }

    /// 关闭右 dock（dock-layout spec: 关闭 + 焦点交回）：`toggle_dock`
    /// 只切 open（面板状态保留；重启恢复默认 — grill 定案，本轮无会话内
    /// 重开路径）。spike E2：点 `×` 会把焦点交给即将隐藏的面板，因此
    /// 关闭后显式把键盘/IME 焦点交回终端面板。
    pub fn close_right_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dock_area.update(cx, |area, cx| {
            area.toggle_dock(DockPlacement::Right, window, cx);
        });
        let focus = self.terminal.read(cx).focus_handle(cx);
        focus.focus(window, cx);
    }

    /// 默认打开系统统一后端 (center-slot-multi-tab, design D1)。
    ///
    /// 接受数据源封装的 [`OpenObject`] → 查 dedup: 命中 `select_panel`
    /// 聚焦既有签 (不重复开); 未命中按注册表顺序找认领 handler, 尾插
    /// 新签并登记。无 handler 认领 → no-op (spec: Unclaimed object opens
    /// nothing)。
    pub fn open_object(
        &mut self,
        obj: OpenObject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = obj.path.to_string_lossy().into_owned();
        if let Some(&panel) = self.open_tabs.get(&key) {
            self.dock_area
                .update(cx, |area, cx| area.select_panel(panel, window, cx));
            // 库的 select_panel/add 不内建聚焦 (只有点击 tab 的
            // TabGroup::select_tab 才 focus_active_panel) — 键盘/IME 随
            // active tab 要求显式交焦 (spec: Keystrokes follow the active tab)
            self.focus_active_center_panel(window, cx);
            return;
        }
        let Some(handler) = handler_for(&obj) else {
            return; // 无认领者: 不可见 (spec: Unclaimed object opens nothing)
        };
        let panel = handler.create_panel(&obj, window, cx);
        let panel_id = panel.panel_id(cx);
        self.dock_area.update(cx, |area, cx| {
            // 尾插 + activate (InsertTarget::Tabs{ix:None}) — console 保 index 0
            area.add_panel_view(panel, DockPlacement::Center, None, window, cx);
        });
        self.open_tabs.insert(key, panel_id);
        self.focus_active_center_panel(window, cx);
    }

    /// 签关闭回调 (EditorPanel::on_removed → design D5): 从 dedup 摘除,
    /// 同对象后续可重开 (spec: Reopen after close creates a fresh tab)。
    pub fn open_tab_removed(&mut self, panel_id: PanelId) {
        self.open_tabs.retain(|_, id| *id != panel_id);
    }

    /// 把键盘/IME 焦点交回中心 active tab 的面板 (design D6-3):
    /// console active 时即终端面板 (与旧“树点击交回终端”逐字同效),
    /// viewer active 时归 viewer。无 center 面板时不动作。
    pub fn focus_active_center_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let area = self.dock_area.read(cx);
        let Some(panel_id) = active_center_panel_id(area) else {
            return;
        };
        let Some(handle) = area.panel(panel_id).map(|panel| panel.focus_handle(cx)) else {
            return;
        };
        handle.focus(window, cx);
    }

    /// sidebar 事件漏斗单一入口 (design D3): SidebarPanel 的所有交互
    /// 经此进入 model；SelectAgent 的一步切焦点也收拢在此 (3.2, spec:
    /// one-step focus — 选中即把键盘/IME 焦点交给 pane)。
    pub fn on_sidebar_event(
        &mut self,
        ev: SidebarEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 会话相关事件先把中心切回 console 签 (spec: sidebar select returns
        // to the console tab — viewer 点 sidebar agent 必须一步回 console
        // + session 焦点); 然后统一把焦点交回 active tab 面板 (design D6):
        // dock 的左 TabGroup 会把 sidebar 内的点击焦点抢给 sidebar 面板
        // (真机实测 focus -> elsewhere), 不交回则键盘/IME 全断。放 match
        // 前 = 所有 arm (含 NewWorkspace 对话框路径) 共用。
        if matches!(
            ev,
            SidebarEvent::NewAgent(_)
                | SidebarEvent::SelectAgent(_)
                | SidebarEvent::CloseAgent(_)
        ) {
            let console = PanelId::from(self.terminal.entity_id());
            self.dock_area
                .update(cx, |area, cx| area.select_panel(console, window, cx));
        }
        self.focus_active_center_panel(window, cx);
        match ev {
            SidebarEvent::NewWorkspace => {
                // 异步对话框: 变更在完成回调里, 面板转发也在那里 (见 create_workspace)
                self.create_workspace(cx);
                return;
            }
            SidebarEvent::ToggleCollapse(path) => self.toggle_collapse(&path),
            SidebarEvent::CloseWorkspace(path) => self.close_workspace(&path),
            SidebarEvent::NewAgent(path) => self.create_agent(&path, cx),
            SidebarEvent::SelectAgent(id) => {
                self.active = Some(id);
            }
            SidebarEvent::CloseAgent(id) => self.close_agent(id, cx),
        }
        cx.notify();
        self.notify_panels(cx);
    }

    /// 数据变化 → notify 展示该数据的面板 (spike §10.5 实测: 面板自读不
    /// 重建, 必须显式转发)。转发列表: sidebar (badge/列表) + terminal
    /// (输出滚动) + tree (文件树: 拉视图/根比对/同步 TreeState —
    /// right-dock-file-tree 3.2)。
    ///
    /// **必须 `cx.defer`**: 事件 listener 挂在面板 entity 上, listener
    /// 执行期间面板持有 update lease — 同步 `panel.update` 会重入 panic
    /// (实测 crash: "cannot update SidebarPanel while it is already being
    /// updated", entity_map lease)。`App::defer` 的 effect 只在**最外层**
    /// update 的 finish 时 flush (pending_updates 守卫), 此时面板 lease
    /// 已 end_lease → 安全。
    pub(crate) fn notify_panels(&self, cx: &mut Context<Self>) {
        let sidebar = self.sidebar.clone();
        let terminal = self.terminal.clone();
        let tree = self.tree.clone();
        cx.defer(move |cx| {
            sidebar.update(cx, |_, cx| cx.notify());
            terminal.update(cx, |_, cx| cx.notify());
            // 树面板不是裸 notify: 要走"根比对→按需重扫→同步 TreeState"
            tree.update(cx, |panel, cx| panel.on_data_changed(cx));
        });
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
    /// session 自身 notify 走根读取链路 (现行为); 同时转发到面板
    /// (spike §10.5: 面板自读不重建) — 输出滚动 + sidebar badge 同帧刷新。
    fn poll_sessions(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for ws in &self.workspaces {
            for session in &ws.agents {
                session.update(cx, |s, cx| {
                    if s.0.poll() {
                        cx.notify();
                        changed = true;
                    }
                });
            }
        }
        if changed {
            self.notify_panels(cx);
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
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // 外壳 (design R3): DockArea 承担左|中|右三槽 (D1), 是本容器的
        // 子节点; 未来 titlebar / 状态栏 / 覆盖层挂同一容器。
        // (行数据/终端块的渲染在各面板 entity 内 — 3.1/4.1)
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.dock_area.clone())
    }
}

#[cfg(test)]
mod tests {
    //! 自适应网格分发的无头验证 (spec: Adaptive terminal grid sizing /
    //! Grid resize propagation): canvas paint 量出的 bounds 经 defer 分发,
    //! 换算与去重在真实渲染路径上成立。显式导入、不用 glob (避免
    //! `gpui_kit::*` 把 `#[test]` 宏遮蔽掉)。

    use super::TerminalApp;
    use crate::agent_entities::{AgentSession, PiAgentShell};
    use crate::workspace::Workspace;
    use gpui_kit::Focusable;
    use crate::pane::{MIN_GRID_COLS, MIN_GRID_ROWS, grid_metrics};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
    };
    use std::fs;
    use std::io::Write as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// hermetic: 注入测试专用 workspace (仅内存, 不写盘) — 让测试不依赖
    /// 本地 `.workspaces.json` 的内容 (2026-11-09 冒烟清空该文件后 4 个
    /// 测试齐挂的教训; 原 adaptive 测试本就依赖本地状态)。返回 full_path
    /// 字符串供行 id 拼装/定位; 注入的 workspace 恒为列表末位。
    fn inject_test_workspace(app: &mut TerminalApp, tag: &str) -> String {
        let path = std::env::temp_dir().join(format!("bw-test-{tag}-{}", std::process::id()));
        let full = path.to_string_lossy().to_string();
        app.workspaces.push(Workspace::new(path));
        full
    }

    /// 假 pi (sleepy 版): 每个响应前 sleep 1s — 让 badge 测试首帧确定为
    /// [Starting] (与 write_stub 同款 RPC 行为, 仅加 sleep)。
    fn write_stub_sleepy(dir: &std::path::Path) -> std::path::PathBuf {
        let pwd_file = dir.join("pwd.txt");
        let script = format!(
            "#!/bin/sh\npwd > '{}'\nwhile IFS= read -r line; do\n  sleep 1\n  id=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\n  printf '{{\"type\":\"response\",\"id\":\"%s\",\"success\":true,\"data\":{{}}}}\\n' \"$id\"\ndone\n",
            pwd_file.display()
        );
        let path = dir.join("fake-pi-sleepy.sh");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(script.as_bytes()).unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

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
                |window, cx| {
                    cx.new(|cx| TerminalApp::new(window, cx).expect(".workspaces.json 加载"))
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
            inject_test_workspace(app, "adaptive"); // hermetic: 不依赖本地文件
            let shell = PiAgentShell::new_with_pi_bin(
                "pi-t".into(),
                std::env::temp_dir(),
                Some(stub.to_str().unwrap().to_string()),
            )
            .expect("stub embed spawn");
            let e = cx.new(|_| AgentSession(Box::new(shell)));
            let id = e.entity_id(); // 先取 id 再 move (D4)
            app.workspaces.last_mut().expect("测试 workspace").agents.push(e);
            app.active = Some(id);
        });

        // frame 1: paint 量 bounds -> defer 分发
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
        });
        cx.run_until_parked();

        // 读取分发结果 + 网格区域 bounds + 度量
        let (resizes1, screen) = app_entity.read_with(cx, |app, cx| {
            let s = app.workspaces.last().expect("测试 workspace").agents[0].read(cx);
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
            let s = app.workspaces.last().expect("测试 workspace").agents[0].read(cx);
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
    /// 回归 (crash fix): sidebar 行点击的 listener 在**面板 lease 内**执行,
    /// funnel 的 notify_panels 必须 `cx.defer` — 同步 panel.update 会重入
    /// panic/abort ("cannot update SidebarPanel while it is already being
    /// updated")。同时覆盖: 2.2 稳定身份载荷 +漏斗后面板确实重渲
    /// (defer 生效: 折叠后 [+ agent] 按钮从观测中消失)。
    #[gpui_kit::test]
    fn sidebar_row_click_toggles_collapse(cx: &mut TestAppContext) {
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
                |window, cx| {
                    cx.new(|cx| TerminalApp::new(window, cx).expect(".workspaces.json 加载"))
                },
            )
            .expect("open test window")
        });

        // hermetic: 注入测试专用 workspace, 不依赖本地 .workspaces.json
        let _ws_path = cx.update(|cx| {
            app_entity.update(cx, |app, _cx| inject_test_workspace(app, "row-click"))
        });
        let (path, before) = cx.update(|cx| {
            let app = app_entity.read(cx);
            let ws = app.workspaces.last().expect("注入的测试 workspace");
            (ws.full_path_str(), ws.collapsed)
        });

        let new_agent_id = format!("new-agent-label-{path}");
        let row_id = format!("ws-name-{path}");

        // 首帧: 行/按钮进入观测注册表 (初始未折叠 -> 按钮可见)
        let id = new_agent_id.clone();
        let _ = cx.update_window(handle.into(), move |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window.try_find(id).is_some(),
                "初始应展开, [+ agent] 可见"
            );
        });

        // 点击行 → listener (面板 lease 内) → funnel → defer notify
        // (修复前: 此处重入 panic → abort)
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.click(row_id.clone(), cx);
        });
        cx.run_until_parked();
        let after = cx.update(|cx| app_entity.read(cx).workspaces.last().expect("测试 workspace").collapsed);
        assert_ne!(before, after, "点击 workspace 行应切换折叠 (且不得重入 panic)");
        // defer 生效: 面板已重渲, 折叠后 [+ agent] 消失 (旧观测不能残留)
        let id = new_agent_id.clone();
        let _ = cx.update_window(handle.into(), move |_, window, _cx| {
            assert!(
                window.try_find(id).is_none(),
                "折叠后面板必须重渲, [+ agent] 应消失"
            );
        });

        // 再点恢复
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.click(row_id, cx);
        });
        cx.run_until_parked();
        let back = cx.update(|cx| app_entity.read(cx).workspaces.last().expect("测试 workspace").collapsed);
        assert_eq!(before, back, "再次点击应恢复原状态");
        let id = new_agent_id.clone();
        let _ = cx.update_window(handle.into(), move |_, window, _cx| {
            assert!(
                window.try_find(id).is_some(),
                "展开后面板必须重渲, [+ agent] 应回来"
            );
        });
    }
    /// 回归 (6.1 手动发现 F1/F3): 点击 agent 行后**一步切焦点**必须把
    /// 键盘/IME 焦点交给终端面板 (spec: Sidebar selection grants keyboard
    /// and IME focus in one step), 且 Tab 派发后焦点保持 (若焦点不在
    /// Terminal context, Root 的 tab 绑定会做焦点遍历 -> 焦点跑掉 = F3 症状)。
    /// 坑点: 点击 dock 内面板时 dock 机制可能在 frame-end 把焦点交给该
    /// 面板 (spike E2 实测) — 本测试用 run_until_parked 冲掉延迟任务后再断言。
    #[gpui_kit::test]
    fn sidebar_select_grants_one_step_focus(cx: &mut TestAppContext) {
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
                |window, cx| {
                    cx.new(|cx| TerminalApp::new(window, cx).expect(".workspaces.json 加载"))
                },
            )
            .expect("open test window")
        });

        // 注入 stub 会话 (与 adaptive_grid_dispatch 同款, 绕过真实 pi)
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("one-step-focus-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let stub = write_stub(&dir);
        let shell = PiAgentShell::new_with_pi_bin(
            "pi-t".into(),
            std::env::temp_dir(),
            Some(stub.to_str().unwrap().to_string()),
        )
        .expect("stub embed spawn");
        let e = cx.new(|_| AgentSession(Box::new(shell)));
        let agent_id = e.entity_id();
        cx.update(|cx| {
            let app = app_entity.update(cx, |app, cx| {
                inject_test_workspace(app, "one-step-focus"); // hermetic
                app.workspaces.last_mut().expect("测试 workspace").agents.push(e);
                app.active = Some(agent_id);
                // 直接注入绕过 funnel, 面板需显式 notify 才会重渲 (§10.5)
                app.sidebar.update(cx, |_, cx| cx.notify());
            });
            let _ = app;
        });
        cx.run_until_parked();

        let term_focus = cx.update(|cx| app_entity.read(cx).terminal.read(cx).focus_handle(cx));

        // 渲染 + 点击 agent 行 (内层观测锚点冒泡到行 on_select)
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.click(format!("agent-name-{agent_id}"), cx);
        });
        cx.run_until_parked(); // 冲掉 frame-end 任务 (dock 焦点 handoff)

        // 断言 1: 一步切焦点 — 焦点必须在终端面板
        let focused = cx.update_window(handle.into(), |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(
            focused,
            Some(term_focus.clone()),
            "点击 agent 行后焦点必须在终端面板 (spec: one-step focus)"
        );

        // 断言 2: Tab 派发后焦点保持 (焦点在 Terminal context -> TerminalTab
        // action 转发并 stop_propagation; 若焦点错位, Root tab 绑定会做
        // 焦点遍历 -> 焦点跑掉)
        let forwards_before = cx.update(|cx| app_entity.read(cx).tab_forwards);
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.press("tab", cx);
        });
        cx.run_until_parked();
        let focused_after = cx.update_window(handle.into(), |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(
            focused_after,
            Some(term_focus),
            "Tab 派发后焦点应保持在终端面板 (TerminalTab 截获而非 Root 遍历)"
        );
        // 断言 3: Tab 必须到达 send_key 转发边界 (spec: Keyboard input
        // forwarding - Tab; pi 收到 Tab 无可见反应, 只能在边界计数)
        let forwards_after = cx.update(|cx| app_entity.read(cx).tab_forwards);
        assert!(
            forwards_after > forwards_before,
            "Tab 应经 send_key 转发给会话 (spec: Tab SHALL be delivered),              实际 {forwards_before} -> {forwards_after}"
        );

        let _ = fs::remove_dir_all(&dir);
    }
    /// 回归 (6.1 手动发现 F4): agent 输出到达 -> drain 循环 (50ms 测试时钟)
    /// -> notify 转发 -> 面板重渲, 状态 starting -> running 实时刷新
    /// (spec: status badge reflect running)。sleepy stub 把首个响应推迟到
    /// ~1s (真实时间), 保证首帧确定为 [Starting]; drain 计时器走测试时钟,
    /// 用 advance_clock 推进。
    #[gpui_kit::test]
    fn status_badge_refreshes_via_drain_loop(cx: &mut TestAppContext) {
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
                |window, cx| {
                    cx.new(|cx| TerminalApp::new(window, cx).expect(".workspaces.json 加载"))
                },
            )
            .expect("open test window")
        });

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("badge-refresh-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let stub = write_stub_sleepy(&dir);
        let shell = PiAgentShell::new_with_pi_bin(
            "pi-t".into(),
            std::env::temp_dir(),
            Some(stub.to_str().unwrap().to_string()),
        )
        .expect("stub embed spawn");
        let e = cx.new(|_| AgentSession(Box::new(shell)));
        let agent_id = e.entity_id();
        app_entity.update(cx, |app, cx| {
            inject_test_workspace(app, "badge"); // hermetic
            app.workspaces.last_mut().expect("测试 workspace").agents.push(e);
            app.active = Some(agent_id);
            // 直接注入绕过 funnel: 两个面板都需显式 notify (§10.5)
            app.sidebar.update(cx, |_, cx| cx.notify());
            app.terminal.update(cx, |_, cx| cx.notify());
        });
        cx.run_until_parked();

        // 首帧: sleepy stub 的响应 ≥1s (真实时间) 才到, 此刻必为 [Starting]
        let starting = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let all: Vec<String> = gpui_kit::base::test_support::texts(window)
                .iter()
                .map(|t| t.text().to_string())
                .collect();
            eprintln!("[F4-diag] texts({}) = {all:?}", all.len());
            all.iter().any(|t| t.contains("[Starting]"))
        })
        .expect("update window");
        assert!(starting, "首帧状态条应显示 [Starting]");

        // 启动 drain 循环 (50ms 测试时钟) -> 真实时间等 stub 响应进缓冲
        // -> 推进测试时钟触发 drain -> poll 消费 -> Running + notify 转发
        app_entity.update(cx, |app, cx| app.start_update_loop(cx));
        std::thread::sleep(std::time::Duration::from_millis(1300));
        cx.executor().advance_clock(std::time::Duration::from_millis(200));
        cx.run_until_parked();

        let running = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            gpui_kit::base::test_support::texts(window)
                .iter()
                .any(|t| t.text().contains("[Running]"))
        })
        .expect("update window");
        assert!(
            running,
            "drain -> notify 转发后面板状态必须刷新为 [Running] (F4 回归)"
        );

        // 收尾: 关会话、清 stub
        app_entity.update(cx, |app, cx| {
            if let Some(e) = app.workspaces.get_mut(0).and_then(|w| w.agents.pop()) {
                e.update(cx, |s, _| s.0.shutdown());
            }
        });
        let _ = fs::remove_dir_all(&dir);
    }
}
