//! 主 App 状态: 单工作区 (workspace: Option), 以 gpui entity 形式渲染 shell。
//!
//! 行为规格见 openspec/specs/{agent-shell,workspace-management,project-model}。
//! 打开流程 (启动恢复/选择/切换门控) 见 design D5/D6; 加载失败经
//! `open_failure` 记录 + 正式构建对话框退出。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::*;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{DockArea, DockPlacement};
use gpui_kit::base::dock::PanelId;

use agent_service::contract::{
    CloseReq, CreateReq, InputClick, InputKey, InputText, InputWheel, ResizeReq, SelectReq,
    SessionId, SessionStatus, ThemeReq, KIND_CLOSE, KIND_CREATE, KIND_INPUT_CLICK, KIND_INPUT_KEY,
    KIND_INPUT_TEXT, KIND_INPUT_WHEEL, KIND_RESIZE, KIND_SELECT, KIND_THEME,
};
use service_manager::{Envelope, Payload, Route};

use crate::service_layer::CtrlOp;
use agent_service::terminal::{AgentTerminal, TerminalCopy, TerminalTab};
use crate::property_detail_manager::{OpenObject, active_center_panel_id, handler_for};
use crate::shell;
use crate::assets::{AssetManager, AssetView, traits::EnumeratorOwner};
use crate::project_model::{self, MarkerStatus, WorkspaceModel};
use crate::sidebar::{SidebarEvent, SidebarPanel};
use crate::workspace::history::History;
use crate::workspace::Workspace;

// Tab 在 Terminal context 下的转发动作: gpui-base 的 Root 全局绑定
// `tab` → 焦点遍历会吞掉 Tab (原始 key event 不再派发), 更深的
// Terminal context 绑定胜出 (design D5, spike 已验证)。
use crate::property_detail_manager::md_panel::ToggleMdView;

pub struct TerminalApp {
    /// 当前打开的工作区 (单实例, spec: Startup workspace restore;
    /// None = 空态, 等待目录选择器)
    pub workspace: Option<Workspace>,
    /// 打开代号: `begin_open` 自增, `commit_open` 校验 — 旧 load 的结果
    /// 直接丢弃 (design D5: 切换与在途加载的竞态)
    pub load_epoch: u64,
    /// 最近一次工作区打开失败 (错误框+退出路径的测试观测面;
    /// 正式构建在记录后 dialog + exit)
    pub open_failure: Option<String>,
    /// 切换门控提示 (spec: notice naming the count of active sessions)
    pub notice: Option<String>,
    /// 当前 active session 的稳定身份 (服务分配 SessionId; **镜像** —
    /// 真值在 AgentService, 由 sessions_changed 投影驱动, task 3.2 / D4)
    pub active: Option<SessionId>,
    /// 最近一次持久化失败的错误信息 (在 sidebar 顶部显示; spec:
    /// Persistence write failure surfacing — 配置写失败红条)
    pub save_error: Option<String>,
    /// 历史文件路径 (测试缝: `new_inner` 注入; 正式 = 全局应用数据目录)
    pub history_path: PathBuf,
    /// DockArea 布局引擎 (design D1): 左|中|右三槽由它编排,
    /// 是 R3 外壳容器 (`render` 的根 div) 的子节点
    pub dock_area: Entity<DockArea>,
    /// sidebar 面板实体 (D2; notify 转发目标 — spike §10.5)
    pub sidebar: Entity<SidebarPanel>,
    /// 终端面板实体 (4.1; 焦点在其内, 一步切焦点/步骤 5 进 DockArea 中心)
    pub terminal: Entity<AgentTerminal>,
    /// 文件树面板实体 (right-dock-file-tree; notify 转发名单成员)
    pub asset_view: Entity<AssetView>,
    /// 右槽数据服务层 (asset-manager-right-dock D1): 面板读视图/代数,
    /// app 钩子做注册与刷新
    pub asset_manager: Entity<AssetManager>,
    /// 服务总线接入 (service-management-framework 6.1): GUI = 拉模式参与
    /// 者 (自己的 Route + mailbox), 50ms 泵 poll; 无回调 (D9)
    pub service_layer: crate::service_layer::ServiceLayer,
    /// 节流 drain 任务; drop 会取消循环 (design D3)
    _update_task: Option<Task<()>>,
    /// 输入触发的快速 drain burst (design D7): 8ms×4 拍取到帧即停,
    /// 新输入 drop 旧 task 重置; 消除 50ms 主循环的相位抖动卡顿
    _input_drain_task: Option<Task<()>>,
    /// 进行中的异步文件夹/确认对话任务 (design D8; 换新对话框时旧任务被取消)
    _dialog_task: Option<Task<()>>,
    /// 在途工作区打开任务 (begin_open 独立存根 — 不能与对话任务互顶,
    /// 否则 picker 回调里触发的 begin_open 会取消自己)
    _open_task: Option<Task<()>>,
    /// 系统外观订阅 (add-light-dark-theme D1): 启动同步 + 热切换时重新
    /// sync 主题; 存实体上保活 (drop 即退订)
    _appearance_subscription: Subscription,
    /// 累计实际传播的网格 resize 次数 (诊断/测试观测: 输入型重绘不增)
    pub grid_resizes: usize,
    /// 上次传播的目标网格 (GUI 侧去重; task 3.2 — 未变布局零传播)
    last_grid_size: Option<(u16, u16)>,
    /// 累计经 send_key 转发给会话的 Tab 键数 (诊断/测试观测: spec
    /// Keyboard input forwarding - Tab 的端到端守卫; pi 收到 Tab 无可见
    /// 反应, 只能在转发边界计数验证 — 见 spike F3 结论)
    pub tab_forwards: usize,
    /// 累计转发给会话的滚轮事件次数 (诊断/测试观测: terminal-pointer-and-
    /// scroll-input — 行数载荷由 CountingAgent 端累计, 这里计入口次数)
    pub wheel_forwards: usize,
    /// 累计转发给会话的点击坐标次数 (同上, 载荷在假会话端断言)
    pub click_forwards: usize,
    /// 默认打开系统的 dedup 表 (center-slot-multi-tab, design D1):
    /// key = 对象绝对路径字符串 (与 AssetNode.id 同一身份体系),
    /// value = 已开面板的 PanelId; 签关闭时经 `open_tab_removed` 摘除。
    pub(crate) open_tabs: HashMap<String, PanelId>,
    /// debug section 的 dedup 表 (specs/center-tabs delta): key = section id
    /// (**独立于**文件路径身份 — 同字符串互不干扰), 签关闭时经
    /// `debug_tab_removed` 摘除。跟着 `crate::debug` 的门控走 (release
    /// 无此字段 — design D5 零成本)。
    #[cfg(debug_assertions)]
    pub(crate) debug_tabs: HashMap<String, PanelId>,
}

impl TerminalApp {
    /// 启动入口: 默认读全局历史文件 (design D4)。
    ///
    /// 打开失败不从这里返回 — 经后台探测回主线程记录 `open_failure`
    /// 并 (正式构建) 对话框+退出; 返回 Err 的旧路径保留给 main 的防御分支。
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Result<Self, String> {
        Self::new_inner(window, cx, crate::workspace::history::default_path())
    }

    /// 启动接线 (design D5): 主窗先行 → 有历史则异步 load 最近工作区
    /// (带 epoch 防竞态), 无历史/目标失效 → 空态选择器 (spec: Startup
    /// workspace restore)。`history_path` 是测试缝 (hermetic: 测试不碰
    /// 真实应用数据目录)。
    pub fn new_inner(
        window: &mut Window,
        cx: &mut Context<Self>,
        history_path: PathBuf,
    ) -> Result<Self, String> {
        // Terminal context 下的 tab 终端转发绑定 (Root 的焦点遍历
        // 绑定在更浅的 context, 被这条压过 — design D5)
        cx.bind_keys([
            KeyBinding::new("tab", TerminalTab, Some("Terminal")),
            // 复制选区 (terminal-text-selection task 4.1, spec: Cmd+C):
            // Terminal context 深于全局, 不与文本框的 cmd-c 抢;
            // modifiers.platform 已被 handle_key_down 拦, 不漏给会话
            KeyBinding::new("cmd-c", TerminalCopy, Some("Terminal")),
            // md 查看切换 (md-viewer-toggle task 3.1): context=None 任意
            // context 匹配 — MdPanel 无专属 key_context, 且无键位冲突
            KeyBinding::new("cmd-shift-m", ToggleMdView, None),
        ]);

        #[allow(unused_mut)] // cfg(test) 不走 legacy 播种分支 → mut 不被用
        let mut history = History::load_at(&history_path);
        // legacy 播种: 正式构建一次性 (legacy_seeded 落盘后不再读);
        // cfg(test) 关闭 — 仓库根可能残留 legacy 文件, 测试不得读外部状态
        // (hermetic 教训, 2026-11-09)。播种逻辑本体见 store::seed_legacy。
        #[cfg(not(test))]
        if crate::workspace::store::seed_legacy(&mut history, &crate::workspace::store::legacy_path()) {
            if let Err(e) = history.save_at(&history_path) {
                eprintln!("workspace history save failed: {e}");
            }
        }
        // 服务层启动接线 (task 6.1): 扫 plugins/ 建 Catalog + 装载核心假
        // 服务 + GUI mailbox/订阅; 失败即启动失败 (扫描目录不存在 = 空, 不错)
        let service_layer = crate::service_layer::ServiceLayer::bootstrap(Path::new("plugins"))?;

                let restore = history.most_recent().map(Path::to_path_buf);
        // 装配 DockArea 三槽 (shell::build; panel_handle 包装见其文档)
        let weak_app = cx.weak_entity();
        // 右槽数据的服务层 (asset-manager-right-dock D1) — 先建以注入
        // 面板 (asset-package-merge design D7: 构造注入, 无兄弟跳)
        let asset_manager = cx.new(|cx| AssetManager::new(weak_app.clone(), cx));
        let shell::ShellParts {
            dock_area,
            sidebar,
            terminal,
            asset_view,
        } = shell::build(&asset_manager, weak_app, window, cx);
        // 主题跟随系统外观 (add-light-dark-theme D1): 启动同步一次,
        // 并订阅外观变化 → 每次变化重新 sync (Theme::change 内部会
        // refresh_windows, 全窗重渲即热切换生效), 并把新配色推给
        // 所有活会话 (D4: 会话内容色随主题)。
        gpui_kit::component::theme::Theme::sync_system_appearance(Some(window), cx);
        let weak_for_theme = cx.weak_entity();
        let appearance_subscription = window.observe_window_appearance(move |window, cx| {
            gpui_kit::component::theme::Theme::sync_system_appearance(Some(window), cx);
            // 主题已切 → 遍历活会话 set_theme (spec: Theme change updates
            // live sessions — 不重启、不丢 scrollback)
            if let Some(app) = weak_for_theme.upgrade() {
                app.update(cx, |app, cx| app.push_tui_theme(cx));
            }
        });
        let mut app = Self {
            workspace: None,
            load_epoch: 0,
            open_failure: None,
            notice: None,
            active: None,
            save_error: None,
            history_path,
            _update_task: None,
            _input_drain_task: None,
            _dialog_task: None,
            _open_task: None,
            _appearance_subscription: appearance_subscription,
            grid_resizes: 0,
            last_grid_size: None,
            tab_forwards: 0,
            wheel_forwards: 0,
            click_forwards: 0,
            open_tabs: HashMap::new(),
            #[cfg(debug_assertions)]
            debug_tabs: HashMap::new(),
            dock_area,
            sidebar,
            terminal,
            asset_view,
            asset_manager,
            service_layer,
        };
        // 有历史 → 异步恢复最近工作区; 无历史/失效 → 留空态 (不报错)
        if let Some(path) = restore {
            app.begin_open(path, cx);
        }
        // 内置 debug sections 装配期注册 (add-debug-console 8.1, design
        // D11: 注册表先填充, 菜单后构建一次)。release 下 debug 模块为空壳
        // → 调用点同 cfg (debug_tabs 字段同款)。
        #[cfg(debug_assertions)]
        crate::debug::register_builtin();
        // 应用菜单栏 (add-debug-console 6.1/6.3, specs/app-menu-bar): 装配
        // 完成后构建一次 (design D11) + 接 Debug 菜单项 action 分发 (6.3)。
        // bind_keys 在前 — macOS 建菜单项时读 keymap 出快捷键提示 (⌘C)。
        crate::menu::install(cx);
        Ok(app)
    }

    /// 点 project 行: 只切该行折叠状态 (spec: Project row collapse
    /// toggle — 不切 active/tab, 不抢焦点; 载荷 = root PathBuf 稳定身份)。
    pub fn toggle_project_collapse(&mut self, root: &Path) {
        if let Some(ws) = self.workspace.as_mut() {
            if !ws.collapsed_projects.remove(root) {
                ws.collapsed_projects.insert(root.to_path_buf());
            }
        }
    }

    /// 用户点了某 Project 行下的 "[+ agent]": 经服务创建 (ask, D6 —
    /// 命名/active 归服务; 成功回执由 pump 切 console, 失败 notice 点名,
    /// spec: Project session creation + spec delta 场景 3)。
    pub fn create_agent(&mut self, root: &Path, cx: &mut Context<Self>) {
        if self.workspace.is_none() {
            return; // 无工作区 (按钮本不该渲染)
        }
        let result = self.service_layer.submit_control(
            Route::Service("agent".into()),
            CtrlOp::Create,
            KIND_CREATE,
            Payload::new(CreateReq {
                root: root.to_path_buf(),
            }),
            "创建会话",
        );
        match result {
            Ok(_) => self.kick_input_drain(cx), // 快泵: 回执尽快达
            Err(e) => self.notice = Some(format!("创建会话失败: {e}")),
        }
    }

    /// 用户点了某个 agent 的 "×": 经服务关闭 (ask; 关 active 后退位归服务
    /// — spec: next remaining session; 失败 notice, spec delta 场景 1)。
    pub fn close_agent(&mut self, id: SessionId, cx: &mut Context<Self>) {
        let result = self.service_layer.submit_control(
            Route::Service("agent".into()),
            CtrlOp::Close,
            KIND_CLOSE,
            Payload::new(CloseReq(id)),
            "关闭会话",
        );
        match result {
            Ok(_) => self.kick_input_drain(cx),
            Err(e) => self.notice = Some(format!("关闭会话失败: {e}")),
        }
    }

    /// 把当前主题推给服务 (服务存快照并广播全部会话; create 注入同源
    /// — add-light-dark-theme D4 语义不变, change agent-service 3.2 改走消息)。
    fn push_tui_theme(&self, cx: &mut Context<Self>) {
        let theme = crate::ui::tui_theme(cx.theme().mode);
        let _ = self.service_layer.manager.tell(Envelope::new(
            Route::Service("agent".into()),
            self.service_layer.listener.address().clone(),
            KIND_THEME,
            Payload::new(ThemeReq { theme }),
        ));
    }

    /// 把一个键送给 active session (tell, D6 静默; 输入注入归服务)。
    pub(crate) fn send_key(&mut self, key: crossterm::event::KeyEvent, cx: &mut Context<Self>) {
        if key.code == crossterm::event::KeyCode::Tab {
            self.tab_forwards += 1; // 诊断/测试观测 (spec: Tab 转发边界)
        }
        let Some(active) = self.active else {
            return;
        };
        // 代际基线 (t0 同时, 任务 2.1): 快泵窗/echo 口径共用——此刻尚未
        // tell 送入 mailbox；回显由快泵/主泵拉取。
        let _ = self
            .service_layer
            .manager
            .tell(Envelope::new(
                Route::Service("agent".into()),
                self.service_layer.listener.address().clone(),
                KIND_INPUT_KEY,
                Payload::new(InputKey {
                    session: active,
                    key,
                }),
            ));
        self.kick_input_drain(cx); // 快速 pump (design D5): 字节即到即显
    }

    /// 把一段已提交文本（IME commit / 粘贴）送给 active session。
    pub(crate) fn send_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return;
        };
        let _ = self.service_layer.manager.tell(Envelope::new(
            Route::Service("agent".into()),
            self.service_layer.listener.address().clone(),
            KIND_INPUT_TEXT,
            Payload::new(InputText {
                session: active,
                text: text.to_string(),
            }),
        ));
        self.kick_input_drain(cx); // 快速 pump (design D5)
    }

    /// 把滚轮行数送给 active session (task 2.2)。无 active session 时不
    /// 计数不转发 (与 send_text 同款: 无收者即无动作)。
    pub(crate) fn send_wheel(&mut self, lines: i16, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return;
        };
        self.wheel_forwards += 1;
        let _ = self.service_layer.manager.tell(Envelope::new(
            Route::Service("agent".into()),
            self.service_layer.listener.address().clone(),
            KIND_INPUT_WHEEL,
            Payload::new(InputWheel {
                session: active,
                lines,
            }),
        ));
        self.kick_input_drain(cx); // 快速 pump (design D5): 滚动帧即到即显
    }

    /// 把左键单元格坐标送给 active session (task 2.2; 坐标系 = 会话帧
    /// 0-based, 命中在会话内部)。
    pub(crate) fn send_click(&mut self, x: u16, y: u16, cx: &mut Context<Self>) {
        let Some(active) = self.active else {
            return;
        };
        self.click_forwards += 1;
        let _ = self.service_layer.manager.tell(Envelope::new(
            Route::Service("agent".into()),
            self.service_layer.listener.address().clone(),
            KIND_INPUT_CLICK,
            Payload::new(InputClick {
                session: active,
                x,
                y,
            }),
        ));
        self.kick_input_drain(cx); // 快速 pump (design D5)
    }

    /// 把 (cols, rows) 分发给全部会话 (tell 经服务; 服务侧逐会话去重 +
    /// 新会话迟到补传 — change agent-service 3.2)。GUI 侧按目标尺寸去重:
    /// 未变尺寸的重绘零传播零日志零计数 (spec 语义保持)。
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
        let target = (cols, rows);
        if self.last_grid_size == Some(target) {
            return; // 未变布局: 零传播 (旧逐会话去重的全局等价)
        }
        self.last_grid_size = Some(target);
        let ids: Vec<SessionId> = self
            .service_layer
            .sessions_view()
            .map(|v| v.sessions.iter().map(|s| s.id).collect())
            .unwrap_or_default();
        if ids.is_empty() {
            return; // 无会话: 不计数不日志 (同旧行为)
        }
        for session in ids {
            let _ = self.service_layer.manager.tell(Envelope::new(
                Route::Service("agent".into()),
                self.service_layer.listener.address().clone(),
                KIND_RESIZE,
                Payload::new(ResizeReq {
                    session,
                    cols,
                    rows,
                }),
            ));
        }
        let resized = self
            .service_layer
            .sessions_view()
            .map(|v| v.sessions.len())
            .unwrap_or(0);
        self.grid_resizes += resized;
        eprintln!(
            "[grid] pane {w:.0}x{h:.0}px -> {cols}x{rows} cells \
             (cell_w {:.2}, line_h {:.2}); resized {resized} session(s)",
            f32::from(cell_w),
            f32::from(line_h)
        );
        self.kick_input_drain(cx); // 快速 pump (design D5): 拖窗重排即显
    }

    /// 空态选择器 / 顶栏切换入口共用的打开流程 (spec: Workspace opening
    /// via directory picker); `gated` = 切换语义, 先过三支门控 (design D6):
    /// 活动会话拦截出 notice, 仅 exited 需确认, 无会话直通 picker。
    pub fn request_workspace_open(&mut self, gated: bool, cx: &mut Context<Self>) {
        self.notice = None;
        let gate = if gated {
            let (active, exited) = self.count_sessions(cx);
            switch_gate(active, exited)
        } else {
            GateDecision::Direct
        };
        if let GateDecision::Block(n) = gate {
            self.notice = Some(format!(
                "{n} session(s) still active — close them before switching"
            ));
            cx.notify();
            self.notify_panels(cx);
            return;
        }
        let needs_confirm = matches!(gate, GateDecision::Confirm);

        // 不能用同步 rfd 对话框: 监听器/回调期间阻塞会撞上 50ms drain 的
        // `weak.update` 重入 (design D8 实测崩溃) — 全异步。
        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            if needs_confirm {
                let ok = rfd::AsyncMessageDialog::new()
                    .set_title("Switch workspace?")
                    .set_description(
                        "Exited sessions and their preserved terminal screens will be discarded.",
                    )
                    .set_level(rfd::MessageLevel::Warning)
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show()
                    .await;
                if !matches!(ok, rfd::MessageDialogResult::Yes) {
                    return; // 取消: 工作区不变
                }
            }
            let Some(handle) = rfd::AsyncFileDialog::new().pick_folder().await else {
                return; // 取消 picker: 不变 (spec: Cancelling … unchanged)
            };
            let picked = handle.path().to_path_buf();
            let path = std::fs::canonicalize(&picked).unwrap_or(picked);
            match project_model::marker_status(&path) {
                MarkerStatus::Corrupt(msg) => {
                    // spec: 坏 marker → 错误框 + 退出 (无 update lease 时弹框,
                    // D8: 阻塞调用不进事件监听器)
                    this.update(cx, |app, _| {
                        app.open_failure = Some(msg.clone());
                    })
                    .ok();
                    surface_fatal(&msg);
                }
                MarkerStatus::Missing => {
                    // spec: 非工作区目录 → 确认后创建
                    let confirm = rfd::AsyncMessageDialog::new()
                        .set_title("Not a workspace")
                        .set_description(&format!(
                            "{} has no workspace marker. Create a new workspace here?",
                            path.display()
                        ))
                        .set_level(rfd::MessageLevel::Info)
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .show()
                        .await;
                    if !matches!(confirm, rfd::MessageDialogResult::Yes) {
                        return; // 拒绝: 无工作区被打开 (spec)
                    }
                    // fs 写在 update 外; 失败 → 红条 + 不打开 (spec:
                    // Workspace creation … write failure … no workspace opened)
                    let created = project_model::create_workspace_files(&path);
                    let err = created.err();
                    this.update(cx, |app, cx| {
                        if let Some(e) = err {
                            app.save_error = Some(e);
                        } else {
                            app.begin_open(path.clone(), cx);
                        }
                        cx.notify();
                        app.notify_panels(cx);
                    })
                    .ok();
                }
                MarkerStatus::Valid(_) => {
                    // 合法 → 异步加载 (epoch 由 begin_open 管)
                    this.update(cx, |app, cx| {
                        app.begin_open(path.clone(), cx);
                    })
                    .ok();
                }
            }
        });
        self._dialog_task = Some(task);
    }

    /// 发起一次工作区打开 (启动恢复 / picker 确认后): epoch++ 后台探测
    /// +加载, 回主线程按 epoch 提交 (design D5: 旧结果丢弃)。返回本次
    /// epoch (测试观测)。
    pub fn begin_open(&mut self, path: PathBuf, cx: &mut Context<Self>) -> u64 {
        self.load_epoch += 1;
        let epoch = self.load_epoch;
        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let probe = {
                let p = path.clone();
                cx.background_executor()
                    .spawn(async move { probe_open(&p) })
                    .await
            };
            match probe {
                OpenProbe::Failure(msg) => {
                    // epoch 检查与记录合一次 update; 对话框在 lease 外 (D8)
                    let fresh = this
                        .update(cx, |app, _| {
                            if app.load_epoch == epoch {
                                app.open_failure = Some(msg.clone());
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(false);
                    if fresh {
                        surface_fatal(&msg); // 正式: 错误框 + 退出; 测试: no-op
                    }
                }
                probe => {
                    this.update(cx, |app, cx| {
                        app.commit_open(epoch, path, probe, cx);
                    })
                    .ok();
                }
            }
        });
        self._open_task = Some(task);
        epoch
    }

    /// 提交一次打开结果。epoch 不匹配 → 丢弃并返回 false (spec/设计 D5:
    /// 竞态下的旧 load 不得覆盖新状态)。
    pub fn commit_open(
        &mut self,
        epoch: u64,
        path: PathBuf,
        probe: OpenProbe,
        cx: &mut Context<Self>,
    ) -> bool {
        if epoch != self.load_epoch {
            return false; // 过期结果 (4.2 竞态守卫)
        }
        match probe {
            // 启动恢复目标失效 (目录没了/marker 没了) → 回空态, 不报错
            OpenProbe::Empty => {
                self.workspace = None;
                self.active = None;
            }
            OpenProbe::Failure(msg) => {
                // 正式路径已在 begin_open 里 dialog+exit; 测试观测面
                self.open_failure = Some(msg);
            }
            OpenProbe::Ready(model) => {
                // 清场: 旧会话全部关停 (切换语义: exited 屏幕随切走丢弃,
                // spec: confirmation … discarded; 活动会话已被门控拦下)
                if let Some(old) = self.workspace.take() {
                    // 会话本体归服务: 逐 id tell 关闭 (切换清场, D6 静默)
                    for sid in old.agents.clone() {
                        let _ = self.service_layer.manager.tell(Envelope::new(
                            Route::Service("agent".into()),
                            self.service_layer.listener.address().clone(),
                            KIND_CLOSE,
                            Payload::new(CloseReq(sid)),
                        ));
                    }
                }
                self.active = None;
                self.notice = None;
                // 历史置顶 (便利数据: 失败仅日志, 不红条 — 非 workspace 配置)
                let mut history = History::load_at(&self.history_path);
                history.push(path.clone());
                if let Err(e) = history.save_at(&self.history_path) {
                    eprintln!("workspace history save failed: {e}");
                }
                let projects = model.projects.clone();
                self.workspace = Some(Workspace::new(path.clone(), model));
                // 注册进 asset_manager (design D6: 打开 = 全量替换;
                // 过滤规则由 Project 供给 — file-filter-prune D9)
                let owners: Vec<Box<dyn EnumeratorOwner + Send>> =
                    projects
                        .into_iter()
                        .map(|p| Box::new(p) as Box<dyn EnumeratorOwner + Send>)
                        .collect();
                self.asset_manager
                    .update(cx, |m, cx| m.register(path, owners, cx));
            }
        }
        cx.notify();
        self.notify_panels(cx);
        true
    }

    /// 空工作区的 "+ add solution" (spec: Solution creation via path picker):
    /// 选择目录 → 追加 simple item → project 行不重启出现。
    /// 写失败: 红条 + **内存变更保留** (spec: in-memory state keeps the change)。
    pub fn add_solution(&mut self, cx: &mut Context<Self>) {
        let Some(ws_dir) = self.workspace.as_ref().map(|w| w.path.clone()) else {
            return;
        };
        let task = cx.spawn(async move |this, cx: &mut AsyncApp| {
            let Some(handle) = rfd::AsyncFileDialog::new().pick_folder().await else {
                return; // 取消: 文件与视图均不变 (spec)
            };
            let picked = handle.path().to_path_buf();
            let path = std::fs::canonicalize(&picked).unwrap_or(picked);
            // 先写盘; 内存变更不受写结果影响
            let outcome = project_model::append_simple_solution(&ws_dir, path.clone());
            this.update(cx, |app, cx| app.apply_add_solution(path.clone(), outcome, cx))
                .ok();
        });
        self._dialog_task = Some(task);
    }

    /// add-solution 完成后的同步合并 (从回调抽出的测试缝 — 5.3):
    /// 写成功 → solutions 用落盘结果; 写失败 → 红条 + **内存变更保留**
    /// (spec: in-memory state keeps the change)。两条路径都进 model。
    pub(crate) fn apply_add_solution(
        &mut self,
        path: PathBuf,
        outcome: Result<Vec<project_model::SolutionItem>, String>,
        cx: &mut Context<Self>,
    ) {
        let write_err = match &outcome {
            Ok(items) => {
                if let Some(ws) = self.workspace.as_mut() {
                    ws.model.solutions = items.clone();
                }
                None
            }
            Err(e) => {
                if let Some(ws) = self.workspace.as_mut() {
                    ws.model
                        .solutions
                        .push(project_model::SolutionItem::Simple { path: path.clone() });
                }
                Some(e.clone())
            }
        };
        // 两条路径都进内存 model (项目行立即可见)
        let mut added: Option<project_model::Project> = None;
        if let Some(ws) = self.workspace.as_mut() {
            if !ws.model.projects.iter().any(|p| p.root == path) {
                let project = project_model::Project::from_root(path.clone());
                ws.model.projects.push(project.clone());
                added = Some(project);
            }
        }
        if let Some(e) = write_err {
            self.save_error = Some(e);
        }
        // 增量注册 (design D6): add-solution → 新项目无重启入右树
        if let Some(project) = added {
            self.asset_manager
                .update(cx, |m, cx| m.register_project(Box::new(project), cx));
        }
        cx.notify();
        self.notify_panels(cx);
    }

    /// 右槽 ⟳ 入口 (asset-manager-right-dock 4.1): 请求 manager 重枚举
    /// (规则重读 + supersede 在途; 保展开是面板侧的事 — spec: Refresh
    /// triggers and behavior)。落地通知由 manager 经 notify_panels 转发。
    pub fn refresh_assets(&mut self, cx: &mut Context<Self>) {
        self.asset_manager.update(cx, |m, cx| m.refresh(cx));
        cx.notify();
        self.notify_panels(cx);
    }

    /// 会话计数 (活动, exited) — 切换门控的输入 (design D6; 数据源 =
    /// 服务投影镜像, change agent-service 3.2)。
    fn count_sessions(&self, _cx: &App) -> (usize, usize) {
        let Some(view) = self.service_layer.sessions_view() else {
            return (0, 0);
        };
        let exited = view
            .sessions
            .iter()
            .filter(|s| s.status == SessionStatus::Exited)
            .count();
        (view.sessions.len() - exited, exited)
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
        // 焦点交回 active tab 面板 (design D6): dock 的左 TabGroup 会把
        // sidebar 内的点击焦点抢给 sidebar 面板, 不交回则键盘/IME 全断。
        // (console 切换已移至控制成功回执 — spec delta: 失败不切)
        self.focus_active_center_panel(window, cx);
        match ev {
            SidebarEvent::OpenWorkspace => {
                // 空态选择器: 无门控; 变更在异步回调里 (见 request_workspace_open)
                self.request_workspace_open(false, cx);
                return;
            }
            SidebarEvent::SwitchWorkspace => {
                // 顶栏切换: 先过三支门控 (design D6); 通过后异步确认/picker
                self.request_workspace_open(true, cx);
                return;
            }
            SidebarEvent::AddSolution => {
                self.add_solution(cx);
                return;
            }
            SidebarEvent::ToggleProjectCollapse(root) => self.toggle_project_collapse(&root),
            SidebarEvent::NewAgent(root) => self.create_agent(&root, cx),
            SidebarEvent::SelectAgent(id) => {
                // ask 经服务 (D6): 成功回执切 console + active 镜像由投影
                // 驱动; 失败 notice 点名 (spec delta 场景 2)
                let result = self.service_layer.submit_control(
                    Route::Service("agent".into()),
                    CtrlOp::Select,
                    KIND_SELECT,
                    Payload::new(SelectReq(id)),
                    "切换会话",
                );
                match result {
                    Ok(_) => self.kick_input_drain(cx),
                    Err(e) => self.notice = Some(format!("切换会话失败: {e}")),
                }
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
        let asset_view = self.asset_view.clone();
        cx.defer(move |cx| {
            sidebar.update(cx, |_, cx| cx.notify());
            terminal.update(cx, |_, cx| cx.notify());
            // 树面板不是裸 notify: 要走"根比对→按需重扫→同步 TreeState"
            asset_view.update(cx, |panel, cx| panel.on_data_changed(cx));
        });
    }

    /// 启动 50ms 节流的 poll 循环 (design D3): 每 tick drain 所有
    /// session, 只在消费了事件时 notify (替代旧的每帧全量重绘)。
    /// 输入后的快速 drain (design D7, 2026-11-10 headed 验收追加):
    /// 滚动卡顿主因 = 50ms 主循环相位抖动 — 输入后 8ms×4 拍快速采样,
    /// 取到新帧即停 (pi 每事件通常 1 帧, 4 拍覆盖其线程调度+重绘时延);
    /// 连续输入 drop 旧 task 重置 (风暴不叠加); 空闲仍靠主循环。
    /// 控制回执 kick (create/close/select/resize) 与输入 kick 共用。
    fn kick_input_drain(&mut self, cx: &mut Context<Self>) {
        let weak = cx.entity().downgrade();
        self._input_drain_task = Some(cx.spawn(async move |_this, cx: &mut AsyncApp| {
            // 8ms×10 = 80ms 快泵窗 (二次回改, 手动验收长尾实测驱动):
            // 旧 4 拍 (32ms) 关窗后, 服务 50ms 主 tick 推来的积压帧只能
            // 等 GUI 主循环再叠一个 50ms 相位 (实测长尾 60-81ms);
            // 泵轮询空转成本 µs 级, 扩窗换长尾消除。
            for _ in 0..10 {
                cx.background_executor()
                    .timer(Duration::from_millis(8))
                    .await;
                let got = weak
                    .update(cx, |app, cx| app.poll_sessions(cx))
                    .unwrap_or(false);
                if got {
                    break;
                }
            }
        }));
    }

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

    /// 服务总线泵 + 投影/帧/回执消费 (change agent-service 3.1/3.2:
    /// Listener poll 并入本 50ms 泵, 无回调无跨线程合同 — D9)。
    /// 返回是否有新件 (design D5: burst 据此"取到帧即停"; 主循环忽略)。
    /// `pub(crate)`: 测试模块驱动泵 (property 侧 select 成功回执 → 切 console)。
    pub(crate) fn poll_sessions(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        if self.service_layer.pump() {
            changed = true;
        }
        // 控制成功回执 → 中央切回 console (成功才切 — spec delta: 失败不切)
        if !self.service_layer.take_successes().is_empty() {
            self.switch_console_tab(cx);
            changed = true;
        }
        // 控制失败 notice (spec delta: 点名失败操作; 取最新一条)
        if let Some(n) = self.service_layer.take_notices().pop() {
            self.notice = Some(n);
            changed = true;
        }
        // agent 首次就绪 → 补推主题 (服务快照默认深色, 宿主主题为准)
        if self.service_layer.take_agent_ready() {
            self.push_tui_theme(cx);
        }
        // 投影镜像: active + id 清单 (D4 真值在服务, GUI 只读)
        if let Some(view) = self.service_layer.sessions_view().cloned() {
            if self.active != view.active {
                self.active = view.active;
                changed = true;
            }
            if let Some(ws) = self.workspace.as_mut() {
                let ids: Vec<SessionId> = view.sessions.iter().map(|s| s.id).collect();
                if ws.agents != ids {
                    ws.agents = ids;
                    changed = true;
                }
            }
            // 镜像同步进面板 (design D7 入站: set_view)
            self.sync_terminal_view(cx);
        }
        // 帧 → AgentTerminal (本地 parser 喂养; 字节零拷贝透传)
        let frames = self.service_layer.take_frames();
        if !frames.is_empty() {
            self.terminal.update(cx, |t, cx| {
                for f in frames {
                    t.feed(f.session, &f.bytes, f.prod_gen, cx);
                }
            });
            changed = true;
        }
        if changed {
            self.notify_panels(cx);
        }
        changed
    }

    /// 投影镜像 → 面板 (测试镜像自举后调用; pump 路径在 poll_sessions 内)。
    pub(crate) fn sync_terminal_view(&mut self, cx: &mut Context<Self>) {
        let view = self.service_layer.sessions_view().cloned();
        self.terminal.update(cx, |t, cx| t.set_view(view, cx));
    }

    /// 控制成功回执 → 中央切回 console 签并交焦 (spec: sidebar select
    /// returns to the console tab; 成功才切 — spec delta)。经 `with_window`
    /// 拿窗口 (TerminalApp 即窗口根视图)。
    fn switch_console_tab(&mut self, cx: &mut Context<Self>) {
        let console = PanelId::from(self.terminal.entity_id());
        let dock = self.dock_area.clone();
        let entity_id = cx.entity().entity_id();
        cx.with_window(entity_id, move |window, cx| {
            dock.update(cx, |area, cx| {
                area.select_panel(console, window, cx);
                if let Some(handle) = area.panel(console).map(|p| p.focus_handle(cx)) {
                    handle.focus(window, cx);
                }
            });
        });
    }
}

/// 打开探测结果 (纯 fs, 跑在后台执行器 — 不占 app 借用, design D5)。
/// (`PartialEq` 随 WorkspaceModel/Project 的 filter 字段改造一并摘除
/// — 无比较使用点, change file-filter-prune D2)。
#[derive(Debug, Clone)]
pub(crate) enum OpenProbe {
    /// 目录不存在 / marker 缺失 → 启动恢复回空态 (不报错; picker 流程
    /// 在此之前已走确认创建分支, 不会进这里)。
    Empty,
    /// marker 合法且 model 加载成功。
    Ready(WorkspaceModel),
    /// marker 坏 / model 校验失败 → 错误框 + 退出 (spec)。
    Failure(String),
}

/// 后台探测: 目录 → marker → model 加载 (每步对应 spec 判定链)。
pub(crate) fn probe_open(path: &Path) -> OpenProbe {
    if !path.is_dir() {
        return OpenProbe::Empty;
    }
    match project_model::marker_status(path) {
        MarkerStatus::Missing => OpenProbe::Empty,
        MarkerStatus::Corrupt(msg) => OpenProbe::Failure(msg),
        MarkerStatus::Valid(_) => match project_model::load(path) {
            Ok(model) => OpenProbe::Ready(model),
            Err(e) => OpenProbe::Failure(e.to_string()),
        },
    }
}

/// 切换门控决策 (design D6; spec: Gated workspace switching)。
/// 纯函数 — 三支可独立无头测试。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateDecision {
    /// 有 Starting/Running 会话 → 拦截 (载荷 = 活动会话数, notice 文案用)
    Block(usize),
    /// 仅剩 Exited → 需确认 (屏幕将丢弃)
    Confirm,
    /// 无会话 → 直接进 picker
    Direct,
}

pub(crate) fn switch_gate(active: usize, exited: usize) -> GateDecision {
    if active > 0 {
        GateDecision::Block(active)
    } else if exited > 0 {
        GateDecision::Confirm
    } else {
        GateDecision::Direct
    }
}

/// 致命打开失败: 错误框 + 退出 (spec: … error dialog … and exit)。
/// **只能在无 update lease 时调用** (design D8: 阻塞对话框不得位于
/// gpui 事件监听器/update 内 — 50ms drain 的 `weak.update` 会重入 panic)。
/// 测试构建: no-op (headless 弹框会挂死测试), 观测面走 `open_failure`。
#[cfg(not(test))]
fn surface_fatal(msg: &str) {
    rfd::MessageDialog::new()
        .set_title("Failed to open workspace")
        .set_description(msg)
        .set_level(rfd::MessageLevel::Error)
        .show();
    std::process::exit(1);
}

#[cfg(test)]
fn surface_fatal(_msg: &str) {}

/// IME 桥 (design D5): 平台 NSTextInputClient → 这些方法。
/// 只有 `replace_text_in_range` (最终提交/粘贴) 产生字节;
/// preedit (`replace_and_mark_text_in_range`) 按 spec 不转发。

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
    // (会话测试直构已随 task 3.2 迁服务工厂; 见 test_sessions)
    use crate::service_layer::test_sessions;
    use crate::workspace::Workspace;
    use pi_shell_rs::app::theme::TuiTheme;
    use std::path::{Path, PathBuf};
    use gpui_kit::Focusable;
    use agent_service::terminal::{MIN_GRID_COLS, MIN_GRID_ROWS, grid_metrics};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
    };
    use std::fs;
    use std::io::Write as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// hermetic: 注入测试专用 workspace (仅内存, 不写盘) — 让测试不依赖
    /// 全局历史/仓库根遗留配置 (2026-11-09 冒烟清空该文件后 4 个
    /// 测试齐挂的教训)。返回 full_path 字符串供行 id 拼装/定位;
    /// 同时注入一个同路径 root project — 项目行/`[+ agent]`/分组都锚
    /// 在它上 (project root == workspace dir 是测试约定)。
    fn inject_test_workspace(app: &mut TerminalApp, tag: &str) -> String {
        let path = std::env::temp_dir().join(format!("bw-test-{tag}-{}", std::process::id()));
        let full = path.to_string_lossy().to_string();
        let mut model = crate::project_model::WorkspaceModel::default();
        model
            .projects
            .push(crate::project_model::Project::from_root(path.clone()));
        app.workspace = Some(Workspace::new(path, model));
        full
    }


    /// task 4.2 (spec: Theme change updates live sessions): 外观切换 →
    /// push_tui_theme 把当前主题的配色推给所有活会话 (不重启)。
    #[gpui_kit::test]
    fn theme_change_pushes_tui_theme_to_live_sessions(cx: &mut TestAppContext) {
        use gpui_kit::component::theme::{Theme, ThemeMode};

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
                    cx.new(|cx| TerminalApp::new(window, cx).expect("app 启动"))
                },
            )
            .expect("open test window")
        });
        let _ = handle;

        app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "theme-push");
        });
        let ws_path = cx.update(|cx| {
            app_entity.read(cx).workspace.as_ref().expect("测试 workspace").path.clone()
        });
        // 经服务创建可检视测试会话 (task 3.2: 主题广播经服务)
        let (_id, shared) = test_sessions::create_session(cx, &app_entity, &ws_path);
        let shared = shared.expect("测试会话 (非 real-pi 路径)");

        // 启动同步后主题 = 系统外观 (测试平台 = Light) → 推浅色 (服务广播)
        app_entity.update(cx, |app, cx| app.push_tui_theme(cx));
        test_sessions::wait_until(cx, || shared.lock().unwrap().themes.len() >= 1, "首推广播");
        let first = shared.lock().unwrap().themes.clone();
        assert_eq!(
            first[0],
            crate::ui::tui_theme(ThemeMode::Light),
            "Light 主题下应推入浅色 TuiTheme"
        );

        // 切到 Dark → 再推 → 会话收到 pi 默认深色 (不重启会话)。
        // 按**值**等待 (agent-ready 的一次性补推可能插队 Light — 不按下标)。
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        app_entity.update(cx, |app, cx| app.push_tui_theme(cx));
        test_sessions::wait_until(
            cx,
            || shared.lock().unwrap().themes.iter().any(|t| *t == TuiTheme::default()),
            "切换推广播 (Dark 配色)",
        );
        let after = shared.lock().unwrap().themes.clone();
        assert!(
            after.iter().any(|t| *t == crate::ui::tui_theme(ThemeMode::Light)),
            "浅色配色仍在推送序列里"
        );
        assert!(
            after.iter().any(|t| *t == TuiTheme::default()),
            "Dark 主题下应推入 pi dark.json 默认值"
        );
        assert_ne!(after[0], *after.last().unwrap(), "两次配色必须不同");
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

    /// add-light-dark-theme task 1.2: 主题跟随系统外观, 不被强制固定。
    /// 先把 Theme 钉到 Dark (模拟“错误的固定状态”), 再构造 TerminalApp —
    /// 其启动 sync 必须把模式拉回 `cx.window_appearance()` 推导值 (测试平台
    /// 恒 Light); 若启动 sync 被删或有人恢复固定模式, 断言失败。
    #[gpui_kit::test]
    fn theme_follows_window_appearance_on_startup(cx: &mut TestAppContext) {
        use gpui_kit::component::theme::{Theme, ThemeMode};

        cx.update(gpui_kit::init);
        // 钉到与外观推导值相反的模式, 证明启动 sync 真的执行了 (而非
        // “恰好默认 Light 而已”)
        cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
        let (_handle, _entity) = cx.update(|cx| {
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
                    cx.new(|cx| TerminalApp::new(window, cx).expect("app 启动"))
                },
            )
            .expect("open test window")
        });
        cx.update(|cx| {
            let expected = ThemeMode::from(cx.window_appearance());
            assert_eq!(
                Theme::global(cx).mode,
                expected,
                "主题必须跟随系统外观 (启动 sync), 而非被强制固定"
            );
        });
    }

    /// 输入触发的快速 drain (task 5.4, design D7): 测试 app 不启主循环
    /// (`start_update_loop` 只在 main.rs 调) → 消费帧的唯一途径是输入后
    /// 的 burst。断言链: stub 启动帧经手动 poll 清场 (顺带验证
    /// `poll_sessions` 返回值), 注入输入 → 真实等待 pi 重绘写入 →
    /// run_until_parked 只跑 burst → 新帧被消费 (screen 出现注入字符)。
    #[gpui_kit::test]
    fn input_kick_drains_frames_before_main_tick(cx: &mut TestAppContext) {
        use std::time::{Duration, Instant};

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
                    cx.new(|cx| TerminalApp::new(window, cx).expect("app 启动"))
                },
            )
            .expect("open test window")
        });
        let _ = handle;

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("input-kick-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let stub = write_stub(&dir);
        app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "kick");
            assert!(
                app._input_drain_task.is_none(),
                "输入前无 burst (主循环也未启动)"
            );
        });
        // 注入目录是纯内存的 — spawn 需要 cwd 真实存在 (否则 ENOENT)
        let ws_path =
            cx.update(|cx| app_entity.read(cx).workspace.as_ref().unwrap().path.clone());
        fs::create_dir_all(&ws_path).unwrap();
        // .pi_bin 标记 → 测试工厂真 spawn stub 会话 (task 3.2: 会话归服务)
        fs::write(
            Path::new(&ws_path).join(".pi_bin"),
            stub.to_str().unwrap(),
        )
        .unwrap();
        let (_agent_id, _) = test_sessions::create_session(cx, &app_entity, Path::new(&ws_path));

        // 消费服务推送的首波事件 (poll_sessions 返回值语义 — design D5);
        // 同时把投影镜像灌进 active (send_key 依赖)。
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let got = cx.update(|cx| app_entity.update(cx, |app, cx| app.poll_sessions(cx)));
            if got {
                break;
            }
            assert!(Instant::now() < deadline, "启动事件未到达");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            cx.update(|cx| app_entity.read(cx).active),
            Some(_agent_id),
            "投影镜像灌入 active"
        );

        // 注入输入 (kick 创建 burst) → 真实等待 pi 重绘写入 buffer
        cx.update(|cx| {
            app_entity.update(cx, |app, cx| {
                app.send_key(
                    crossterm::event::KeyEvent::new(
                        crossterm::event::KeyCode::Char('汉'),
                        crossterm::event::KeyModifiers::NONE,
                    ),
                    cx,
                );
                assert!(
                    app._input_drain_task.is_some(),
                    "send_key 尾部必须 kick burst (design D5)"
                );
            });
        });
        // burst 尚未跑前: 字节最多到 mailbox, 本地 parser 未喂 → 不含注入
        // 字符 (mailbox 与 parser 解耦 — 仅 poll_sessions 喂 parser)
        let has_han = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                let app = app_entity.read(cx);
                let active = app.active;
                app.terminal
                    .read(cx)
                    .active_screen_text(active)
                    .is_some_and(|t| t.contains('汉'))
            })
        };
        assert!(!has_han(cx), "burst 尚未跑前 screen 不含注入字符");
        // 逐轮: 真实等待 pi 重绘 → 推进虚拟时钟 (burst 的 8ms timer 必须
        // 靠 advance_clock 到期 — run_until_parked 不推进时钟) → burst 跑
        // poll_sessions → 喂 parser。消费完全归因于 burst: 主循环未启动。
        // (30s 预算: 链路 = 真实 stub pi 回显 + 服务 50ms 真实时钟 tick + watch
        // 推送, 并行压力下真实时钟被严重挤压 — 单跑 0.5s 内完成; 预算只为
        // 承压不误报, 真故障仍会在预算内显形)
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            std::thread::sleep(Duration::from_millis(30)); // pi 线程重绘时延
            cx.executor().advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            if has_han(cx) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "burst 未在主循环缺席下消费到输入响应帧 (advance_clock 已推进)"
            );
        }

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ws_path);
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
                    cx.new(|cx| TerminalApp::new(window, cx).expect("app 启动"))
                },
            )
            .expect("open test window")
        });

        // 注入 stub 会话并激活 (经服务: .pi_bin 标记 → 工厂真 spawn;
        // task 3.2 — 会话本体归 AgentService)
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("grid-dispatch-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let stub = write_stub(&dir);
        app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "adaptive"); // hermetic: 不依赖本地文件
        });
        let ws_path =
            cx.update(|cx| app_entity.read(cx).workspace.as_ref().unwrap().path.clone());
        fs::create_dir_all(&ws_path).unwrap();
        fs::write(
            Path::new(&ws_path).join(".pi_bin"),
            stub.to_str().unwrap(),
        )
        .unwrap();
        let (agent_id, _) = test_sessions::create_session(cx, &app_entity, Path::new(&ws_path));

        // 等服务推送首帧并经 poll 喂进本地 parser (parser 存在才可断言尺寸)
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let fed = cx.update(|cx| {
                app_entity.update(cx, |app, cx| {
                    app.poll_sessions(cx);
                    app.terminal
                        .read(cx)
                        .active_parser_size(Some(agent_id))
                        .is_some()
                })
            });
            if fed {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "启动帧未喂进本地 parser"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // frame 1: paint 量 bounds -> defer 分发
        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
        });
        cx.run_until_parked();

        // 读取分发结果 + 网格区域 bounds + 度量 (尺寸真值 = 面板本地 parser)
        let (resizes1, screen) = app_entity.read_with(cx, |app, cx| {
            let size = app
                .terminal
                .read(cx)
                .active_parser_size(Some(agent_id))
                .expect("本地 parser 已喂帧");
            (app.grid_resizes, size)
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
            f32::from(line_h)
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
        let (rows, cols) = screen;
        assert_eq!(cols, exp_cols, "会话列数 = floor(宽/cell_w)");
        assert_eq!(rows, exp_rows, "会话行数 = floor(高/line_h)");
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
            let size = app
                .terminal
                .read(cx)
                .active_parser_size(Some(agent_id))
                .expect("本地 parser 已喂帧");
            (app.grid_resizes, size)
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
        let (rows3, cols3) = screen3;
        assert_eq!(cols3, exp_cols3, "resize 后列数跟随新 bounds");
        assert_eq!(rows3, exp_rows3, "resize 后行数跟随新 bounds");

        // 收尾: 经服务关会话、清 stub
        app_entity.update(cx, |app, cx| app.close_agent(agent_id, cx));
        cx.run_until_parked();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ws_path);
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
                    cx.new(|cx| TerminalApp::new(window, cx).expect("app 启动"))
                },
            )
            .expect("open test window")
        });

        // hermetic: 注入测试专用 workspace, 不依赖全局历史与外部配置
        let _ws_path = cx.update(|cx| {
            app_entity.update(cx, |app, _cx| inject_test_workspace(app, "row-click"))
        });
        let (path, before) = cx.update(|cx| {
            let app = app_entity.read(cx);
            let ws = app.workspace.as_ref().expect("注入的测试 workspace");
            // project root == workspace path (inject 约定); 初始未折叠
            (ws.full_path_str(), ws.collapsed_projects.contains(&ws.path))
        });

        let new_agent_id = format!("new-agent-label-{path}");
        let row_id = format!("project-name-{path}");

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
        let after = cx.update(|cx| {
            let app = app_entity.read(cx);
            let ws = app.workspace.as_ref().expect("测试 workspace");
            ws.collapsed_projects.contains(&ws.path)
        });
        assert_ne!(before, after, "点击 project 行应切换折叠 (且不得重入 panic)");
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
        let back = cx.update(|cx| {
            let app = app_entity.read(cx);
            let ws = app.workspace.as_ref().expect("测试 workspace");
            ws.collapsed_projects.contains(&ws.path)
        });
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
                    cx.new(|cx| TerminalApp::new(window, cx).expect("app 启动"))
                },
            )
            .expect("open test window")
        });

        // 注入 workspace 拿 root: 会话 cwd 必须 == project root
        // (spec: Session working directory), 否则 sidebar 分组键
        // (project_root == root) 不命中, agent 行不渲染
        let ws_path = cx.update(|cx| {
            app_entity.update(cx, |app, _cx| inject_test_workspace(app, "one-step-focus"))
        });
        // 注入目录是纯内存的 — spawn 需要 cwd 真实存在 (否则 ENOENT)
        fs::create_dir_all(&ws_path).unwrap();
        let stub = write_stub(Path::new(&ws_path));
        // .pi_bin 标记 → 测试工厂真 spawn stub 会话 (task 3.2: 归服务)
        fs::write(
            Path::new(&ws_path).join(".pi_bin"),
            stub.to_str().unwrap(),
        )
        .unwrap();
        let (agent_id, _) =
            test_sessions::create_session(cx, &app_entity, Path::new(&ws_path));
        cx.update(|cx| {
            let app = app_entity.update(cx, |app, cx| {
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

        let _ = fs::remove_dir_all(&ws_path);
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
                    cx.new(|cx| TerminalApp::new(window, cx).expect("app 启动"))
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
        app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "badge"); // hermetic
        });
        let ws_path =
            cx.update(|cx| app_entity.read(cx).workspace.as_ref().unwrap().path.clone());
        fs::create_dir_all(&ws_path).unwrap();
        // .pi_bin 标记 → 真 spawn sleepy stub (状态三态经服务观测, task 3.2)
        fs::write(
            Path::new(&ws_path).join(".pi_bin"),
            stub.to_str().unwrap(),
        )
        .unwrap();
        let (agent_id, _) = test_sessions::create_session(cx, &app_entity, Path::new(&ws_path));
        cx.update(|cx| {
            app_entity.update(cx, |app, cx| {
                // 直接注入绕过 funnel: 两个面板都需显式 notify (§10.5)
                app.sidebar.update(cx, |_, cx| cx.notify());
                app.terminal.update(cx, |_, cx| cx.notify());
            });
        });
        cx.run_until_parked();

        // 首帧: sleepy stub 的响应 ≥1s (真实时间) 才到, 此刻必为 [Starting]
        // (服务观测: 尚无字节 → Starting)
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

        // 服务 drain 循环 (真实 tokio 时钟) 消费首字节 → Running 投影事件
        // → GUI 泵 (测试时钟, advance 推进) 消费 → 面板重渲 (F4 回归)
        app_entity.update(cx, |app, cx| app.start_update_loop(cx));
        std::thread::sleep(std::time::Duration::from_millis(1300)); // 真实时间等 sleepy stub
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            cx.executor().advance_clock(std::time::Duration::from_millis(200));
            cx.run_until_parked();
            let status = cx.update(|cx| {
                let app = app_entity.read(cx);
                app.service_layer
                    .sessions_view()
                    .and_then(|v| v.sessions.iter().find(|s| s.id == agent_id))
                    .map(|s| s.status)
            });
            if status == Some(agent_service::contract::SessionStatus::Running) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "服务侧状态未翻 Running: {status:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

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

        // 收尾: 经服务关会话、清 stub
        app_entity.update(cx, |app, cx| app.close_agent(agent_id, cx));
        cx.run_until_parked();
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&ws_path);
    }


    // ===== single-workspace-project-model 新增测试 =====
    // (组 3: 启动恢复/空态/创建; 组 4: 门控/epoch; 组 5: 树/分组/状态栏;
    //  组 7: 全链路)。对话框 (rfd) 接线是正式构建胶水, 无头不触碰 —
    //  其决策点已拆成纯函数/同步方法被下面测试覆盖。

    use super::{GateDecision, OpenProbe, switch_gate};
    use crate::assets::{AssetKind, AssetState, project_id};
    use crate::project_model::{self, Project, SolutionItem, WorkspaceModel};
    use crate::workspace::history::History;

    /// hermetic 临时目录 (进程 + 纳秒隔离)。
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sw-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// hermetic 空历史文件路径 (不存在 → load_at 得默认空历史)。
    fn fresh_history_path() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "sw-hist-{}-{}.json",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        let _ = fs::remove_file(&p);
        p
    }

    /// 起带指定历史文件的 app (design D5: 主窗先行, 恢复异步)。
    fn open_app_with_history(
        cx: &mut TestAppContext,
        history: PathBuf,
    ) -> (
        gpui_kit::AnyWindowHandle,
        gpui_kit::Entity<TerminalApp>,
    ) {
        cx.update(gpui_kit::init);
        let (handle, app) = cx.update(|cx| {
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
                    cx.new(|cx| {
                        TerminalApp::new_inner(window, cx, history.clone())
                            .expect("new_inner 启动 (打开失败走 open_failure, 非 Err)")
                    })
                },
            )
            .expect("open test window")
        });
        (handle, app)
    }

    /// 等异步恢复落定（功能；输入；输出）。
    /// 功能: `begin_open` 的 probe 跑在真实后台线程 — `run_until_parked` 只
    ///   冲测试执行器不等它 — 限时轮询直到 pred 成立 (每轮冲执行器 + 真实时
    ///   让时给后台线程)。
    /// 输入: app — 被观的 app 实体; pred — 落定判据 (读 TerminalApp);
    ///   what — 超时诊断文案。
    /// 输出: pred 成立即返回; 5s 未落定 panic (带 what)。
    fn wait_startup_settled(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
        mut pred: impl FnMut(&TerminalApp) -> bool,
        what: &str,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            cx.run_until_parked();
            if cx.update(|cx| pred(app.read(cx))) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "启动恢复未落定: {what}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// task 6.1: 服务总线接入 — 状态翻转经 50ms 主循环泵落到 GUI
    /// (Listener 接入 poll 门; 无回调, D9)。事件不早产: 主循环未 advance
    /// 时 pump 不跑, disable 事件必在 tick 时被消费 (确定性)。
    /// 注意: 状态事件经 LayerRuntime 转发循环(异步任务,独立 service-layer
    /// 线程)投递, 与 mock 时钟的泵拍不同步 — 断言走限时轮询(5s, 每轮
    /// advance_clock 喂一拍泵 + 真实让时给转发线程调度窗口), 避免高负载
    /// 并行下单次断言的时序竞态(与 service_layer::disable_event_visible_
    /// to_gui_pump 的 wait_until 同款)。
    #[gpui_kit::test]
    fn service_state_flip_reaches_gui_via_update_loop(cx: &mut TestAppContext) {
        use std::time::{Duration, Instant};
        let (handle, app_entity) = open_app_with_history(cx, fresh_history_path());
        let core_id = cx.update(|cx| app_entity.read(cx).service_layer.core_id);
        assert!(
            cx.update(|cx| app_entity.read(cx).service_layer.manager.has_service(&"core.fake".into())),
            "启动已登记核心假服务"
        );

        // 装载完成: 直接读状态 (D10 拉最新快照; 事件另走订阅路由)
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = cx.update(|cx| {
                app_entity
                    .read(cx)
                    .service_layer
                    .manager
                    .supervisor()
                    .state_of(core_id)
            });
            if state == Some(service_manager::ServiceState::Available) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "core.fake 未达 available: {state:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        // 启动 50ms 主循环 (生产在 main.rs 调; 测试显式启动)
        cx.update(|cx| app_entity.update(cx, |app, cx| app.start_update_loop(cx)));

        // 主动翻转 → 事件经 LayerRuntime 转发循环(异步任务)投进 GUI
        // mailbox → 下个 tick 消费。转发与 mock 时钟泵不同步: 限时轮询,
        // 每轮 advance_clock 喂一拍泵 + 真实让时(给 service-layer 线程
        // 调度窗口), 直到本地快照翻转或超时。
        cx.update(|cx| {
            app_entity
                .read(cx)
                .service_layer
                .manager
                .disable(core_id)
                .expect("disable")
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            cx.executor().advance_clock(Duration::from_millis(50));
            cx.run_until_parked();
            let (seen, core_state) = cx.update(|cx| {
                let app = app_entity.read(cx);
                (
                    app.service_layer.events_seen,
                    app.service_layer.states.get(&core_id).cloned(),
                )
            });
            if seen >= 1 && core_state == Some(service_manager::ServiceState::Offline) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "状态事件未在 5s 内落到本地快照: seen={seen} core_state={core_state:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = handle;
    }

    /// 渲染一帧并收集全部文本 (空态/行存在性断言用)。
    fn render_texts(
        handle: gpui_kit::AnyWindowHandle,
        cx: &mut TestAppContext,
    ) -> Vec<String> {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            gpui_kit::base::test_support::texts(window)
                .iter()
                .map(|t| t.text().to_string())
                .collect()
        })
        .expect("update window")
    }

    /// 4.1: 切换门控三支 — 纯函数逐支可测 (spec: Gated workspace switching)。
    #[test]
    fn switch_gate_three_branches() {
        assert_eq!(switch_gate(2, 1), GateDecision::Block(2), "活动优先拦截");
        assert_eq!(switch_gate(1, 0), GateDecision::Block(1));
        assert_eq!(switch_gate(0, 3), GateDecision::Confirm);
        assert_eq!(switch_gate(0, 0), GateDecision::Direct);
    }

    /// 4.1 wiring: 活动会话存在时请求切换 → notice 落地且不进 picker。
    #[gpui_kit::test]
    fn gated_switch_with_active_session_shows_notice(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app_with_history(cx, history);
        app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "gate-notice"); // 含同路径 root project
        });
        let ws_path = cx.update(|cx| app_entity.read(cx).workspace.as_ref().unwrap().path.clone());
        // Running 状态 → 活动支: 注入服务投影镜像 (task 3.2 接缝)
        test_sessions::inject_view(
            cx,
            &app_entity,
            vec![(
                ws_path,
                "recorder".into(),
                agent_service::contract::SessionStatus::Running,
            )],
            Some(0),
        );
        app_entity.update(cx, |app, cx| {
            // 同步拦截, 不 spawn picker 对话框
            app.request_workspace_open(true, cx);
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app_entity.read(cx);
            let notice = app.notice.clone().expect("活动会话应落 notice");
            assert!(notice.contains("1 session(s) still active"), "文案点名计数: {notice}");
            assert!(app.workspace.is_some(), "拦截时工作区不变");
        });
        let _ = handle;
    }

    /// 4.2: 过期 epoch 的提交必须被丢弃 (design D5 竞态守卫)。
    #[gpui_kit::test]
    fn stale_epoch_commit_is_discarded(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app_with_history(cx, history);
        app_entity.update(cx, |app, cx| {
            app.load_epoch = 7;
            let discarded = app.commit_open(
                6,
                "/old".into(),
                OpenProbe::Ready(WorkspaceModel::default()),
                cx,
            );
            assert!(!discarded, "过期 epoch 必须返回 false 且不落状态");
            assert!(app.workspace.is_none(), "过期结果不得覆盖");
            let landed = app.commit_open(
                7,
                "/new".into(),
                OpenProbe::Ready(WorkspaceModel::default()),
                cx,
            );
            assert!(landed, "当前 epoch 提交成功");
            assert_eq!(
                app.workspace.as_ref().unwrap().path,
                PathBuf::from("/new")
            );
        });
    }

    /// 4.2: 连续两次 begin_open → 最终状态 = 最后一次的目标 (旧 probe 被丢)。
    #[gpui_kit::test]
    fn two_begin_opens_settle_on_latest_workspace(cx: &mut TestAppContext) {
        let ws_old = temp_dir("begin-old");
        let ws_new = temp_dir("begin-new");
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app_with_history(cx, history);
        app_entity.update(cx, |app, cx| {
            project_model::create_workspace_files(&ws_old).unwrap();
            project_model::create_workspace_files(&ws_new).unwrap();
            app.begin_open(ws_old.clone(), cx); // epoch 1
            app.begin_open(ws_new.clone(), cx); // epoch 2 — 立即覆盖
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert_eq!(
                app.workspace.as_ref().unwrap().path,
                ws_new,
                "只允许最后一次打开生效"
            );
            assert_eq!(app.load_epoch, 2);
            assert!(app.open_failure.is_none());
        });
    }

    /// 3.1: 启动恢复 — 历史最常打开且 marker+model 合法 → 异步载入落定。
    #[gpui_kit::test]
    fn startup_restores_most_recent_workspace(cx: &mut TestAppContext) {
        let ws = temp_dir("restore");
        let project_dir = ws.join("foo");
        fs::create_dir_all(&project_dir).unwrap();
        project_model::create_workspace_files(&ws).unwrap();
        project_model::append_simple_solution(&ws, project_dir.clone()).unwrap();
        let history = fresh_history_path();
        let mut h = History::default();
        h.push(ws.clone());
        h.save_at(&history).unwrap();

        let (_handle, app_entity) = open_app_with_history(cx, history);
        // 恢复 probe 在真实后台线程 — 限时轮询等落定 (run_until_parked 不等它)
        wait_startup_settled(
            cx,
            &app_entity,
            |app| app.workspace.is_some(),
            "恢复最近工作区",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert!(app.open_failure.is_none(), "合法工作区不报错");
            let ws_view = app.workspace.as_ref().expect("应恢复最近工作区");
            assert_eq!(ws_view.path, ws);
            assert_eq!(ws_view.model.projects.len(), 1);
            assert_eq!(ws_view.model.projects[0].name, "foo");
            assert_eq!(ws_view.model.projects[0].root, project_dir);
        });
    }

    /// 3.1: 坏 marker → open_failure 记录 (正式构建: 同点 dialog+exit)。
    #[gpui_kit::test]
    fn startup_corrupt_marker_records_open_failure(cx: &mut TestAppContext) {
        let ws = temp_dir("corrupt-marker");
        fs::write(ws.join(project_model::files::MARKER_FILE), "{ nope").unwrap();
        let history = fresh_history_path();
        let mut h = History::default();
        h.push(ws);
        h.save_at(&history).unwrap();

        let (_handle, app_entity) = open_app_with_history(cx, history);
        // 恢复 probe 在真实后台线程 — 限时轮询等失败落定 (单次冲执行器会
        // 在 probe 未跑完时误判 "未记录")
        wait_startup_settled(
            cx,
            &app_entity,
            |app| app.open_failure.is_some(),
            "坏 marker 记录失败",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            let msg = app.open_failure.clone().expect("坏 marker 必须记录失败");
            assert!(msg.contains("parse"), "错误点名解析失败: {msg}");
            assert!(app.workspace.is_none(), "失败不得打开工作区");
        });
    }

    /// 3.2: 恢复目标失效 (目录/ marker 不见了) → 回空态且不报错。
    #[gpui_kit::test]
    fn startup_missing_target_falls_back_to_empty_state(cx: &mut TestAppContext) {
        let ghost = std::env::temp_dir().join(format!("sw-ghost-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ghost);
        let history = fresh_history_path();
        let mut h = History::default();
        h.push(ghost);
        h.save_at(&history).unwrap();

        let (_handle, app_entity) = open_app_with_history(cx, history);
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert!(app.workspace.is_none(), "失效目标回空态");
            assert!(app.open_failure.is_none(), "失效目标不是错误 (spec)");
        });
    }

    /// 3.2: 空历史启动 → 窗内空态 + 目录选择器入口 (render 级)。
    #[gpui_kit::test]
    fn empty_state_renders_picker_affordance(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (handle, _app) = open_app_with_history(cx, history);
        cx.run_until_parked();
        let texts = render_texts(handle, cx);
        assert!(
            texts.iter().any(|t| t.contains("no workspace open")),
            "空态提示应渲染, 实际: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("open workspace…")),
            "选择器入口应渲染, 实际: {texts:?}"
        );
    }

    /// 3.3 core: 确认创建后 (两文件落盘) → 打开为空工作区。
    /// (确认对话框本身是 rfd 胶水, 无头不触碰; 创建文件契约见
    ///  project_model::files 测试)
    #[gpui_kit::test]
    fn confirmed_creation_then_open_lands_workspace(cx: &mut TestAppContext) {
        let ws = temp_dir("create-open");
        project_model::create_workspace_files(&ws).unwrap();
        assert!(ws.join(project_model::files::MARKER_FILE).exists());
        assert!(ws.join(project_model::files::SOLUTIONS_FILE).exists());

        let history = fresh_history_path();
        let (_handle, app_entity) = open_app_with_history(cx, history);
        app_entity.update(cx, |app, cx| {
            app.begin_open(ws.clone(), cx);
        });
        cx.run_until_parked();
        let back = cx.update(|cx| {
            let app = app_entity.read(cx);
            assert!(app.open_failure.is_none());
            let ws_view = app.workspace.as_ref().expect("创建后应打开");
            assert_eq!(ws_view.path, ws);
            assert!(ws_view.model.projects.is_empty(), "空 solutions = 零 project");
            // 打开即写历史 (下次启动直开)
            History::load_at(&app.history_path)
        });
        assert_eq!(back.most_recent(), Some(ws.as_path()));
    }

    /// 5.3: add-solution 合并 — Ok/Err 两支都进内存 model, Err 另落红条
    /// (spec: in-memory state keeps the change)。
    #[gpui_kit::test]
    fn add_solution_merges_and_surfaces_write_failure(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app_with_history(cx, history);
        let p1 = temp_dir("sol-one");
        let p2 = temp_dir("sol-two");
        app_entity.update(cx, |app, cx| {
            inject_test_workspace(app, "add-sol"); // 零 project 起点会更贴 spec,
            // 但合并逻辑对有无 project 同构 — 先清空模拟零 project
            app.workspace.as_mut().unwrap().model.projects.clear();
            let items = vec![SolutionItem::Simple { path: p1.clone() }];
            app.apply_add_solution(p1.clone(), Ok(items.clone()), cx);
            {
                let ws = app.workspace.as_ref().unwrap();
                assert_eq!(ws.model.projects.len(), 1);
                assert_eq!(ws.model.projects[0].root, p1);
                assert_eq!(ws.model.solutions, items);
            }
            assert!(app.save_error.is_none(), "成功写入无红条");

            // 写失败: 红条 + 内存保留 (project 与 solution item 都在)
            app.apply_add_solution(p2.clone(), Err("disk full".into()), cx);
            {
                let ws = app.workspace.as_ref().unwrap();
                assert_eq!(ws.model.projects.len(), 2, "写失败仍进内存 (spec)");
                assert!(ws.model.solutions.iter().any(|s| matches!(s, SolutionItem::Simple { path } if *path == p2)));
            }
            assert!(
                app.save_error.as_deref().is_some_and(|e| e.contains("disk full")),
                "写失败应落红条"
            );
        });
    }

    /// 5.1+5.4: agent 按 project root 分组 (含工作区外项目) + 状态栏显示
    /// project root 路径 (spec: Console status bar content)。
    #[gpui_kit::test]
    fn agents_group_under_project_roots_and_status_bar_shows_root(
        cx: &mut TestAppContext,
    ) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app_with_history(cx, history);
        let external = temp_dir("external-project"); // 工作区外的 project
        let ws_path = app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "grouping"); // project root1 = ws 目录
            let ws_path = app.workspace.as_ref().unwrap().path.clone();
            app.workspace
                .as_mut()
                .unwrap()
                .model
                .projects
                .push(Project::from_root(external.clone()));
            ws_path
        });
        // 两个会话归两个 project 根 (cwd 分组) — 注入服务投影镜像
        let ids = test_sessions::inject_view(
            cx,
            &app_entity,
            vec![
                (
                    ws_path,
                    "recorder".into(),
                    agent_service::contract::SessionStatus::Running,
                ),
                (
                    external.clone(),
                    "recorder".into(),
                    agent_service::contract::SessionStatus::Running,
                ),
            ],
            None,
        );
        let (a_ws, a_ext) = (ids[0], ids[1]);
        app_entity.update(cx, |app, cx| {
            app.active = Some(a_ext); // 状态栏显示外部 project 的 root
            app.sidebar.update(cx, |_, cx| cx.notify());
            app.terminal.update(cx, |_, cx| cx.notify());
        });
        cx.run_until_parked();

        // id 类锚点走 try_find (texts() 只收录 test_text 文本)
        let (has_ws_agent, has_ext_agent) = cx
            .update_window(handle, |_, window, _cx| {
                (
                    window
                        .try_find(format!("agent-name-{a_ws}"))
                        .is_some(),
                    window
                        .try_find(format!("agent-name-{a_ext}"))
                        .is_some(),
                )
            })
            .expect("update window");
        assert!(has_ws_agent, "ws 根下的 agent 行应渲染");
        assert!(
            has_ext_agent,
            "工作区外 project 的 agent 行也应渲染 (cwd 分组)"
        );
        let texts = render_texts(handle, cx);
        let external_str = external.to_string_lossy().into_owned();
        assert!(
            texts.iter()
                .any(|t| t.contains(&external_str) && t.contains("/pi-1")),
            "状态栏应显示 project root 绝对路径 + `<project>/pi-1` 名, 实际: {texts:?}"
        );
    }

    /// 5.2: 点 project 行只切折叠 — active 会话与 active tab 均不变
    /// (spec: Click project row does not switch main pane / collapse toggle)。
    #[gpui_kit::test]
    fn project_row_click_keeps_active_session_and_tab(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app_with_history(cx, history);
        app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "row-keep");
        });
        let ws_path = cx.update(|cx| app_entity.read(cx).workspace.as_ref().unwrap().path.clone());
        let ids = test_sessions::inject_view(
            cx,
            &app_entity,
            vec![(
                ws_path,
                "recorder".into(),
                agent_service::contract::SessionStatus::Running,
            )],
            Some(0),
        );
        let agent_id = ids[0];
        app_entity.update(cx, |app, cx| {
            app.sidebar.update(cx, |_, cx| cx.notify());
        });
        cx.run_until_parked();
        render_texts(handle, cx); // 首帧让行进观测注册表

        let (active_before, tab_before) = app_entity.update(cx, |app, _cx| {
            let area = app.dock_area.read(_cx);
            (
                app.active,
                crate::property_detail_manager::active_center_panel_id(area),
            )
        });
        let row_id = cx.update(|cx| {
            let app = app_entity.read(cx);
            format!("project-name-{}", app.workspace.as_ref().unwrap().path.display())
        });
        cx.update_window(handle, |_, window, cx| window.click(row_id, cx))
            .expect("update window");
        cx.run_until_parked();

        let (active_after, tab_after) = app_entity.update(cx, |app, _cx| {
            let area = app.dock_area.read(_cx);
            (
                app.active,
                crate::property_detail_manager::active_center_panel_id(area),
            )
        });
        assert_eq!(active_after, Some(agent_id), "active 会话不变");
        assert_eq!(active_after, active_before);
        assert_eq!(tab_after, tab_before, "active tab 不变");
        let collapsed = app_entity.update(cx, |app, _| {
            let ws = app.workspace.as_ref().unwrap();
            ws.collapsed_projects.contains(&ws.path)
        });
        assert!(collapsed, "折叠态应被切换");
    }

    /// 5.2: 零 project 工作区 → 只给 add-solution 动作, 不渲染 [+ agent]
    /// (spec: If no project exists yet, no [+ agent] button)。
    #[gpui_kit::test]
    fn no_project_workspace_renders_add_solution_not_agent_button(
        cx: &mut TestAppContext,
    ) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app_with_history(cx, history);
        app_entity.update(cx, |app, _| {
            let path = std::env::temp_dir().join(format!(
                "sw-noproj-{}-{}",
                std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            app.workspace = Some(Workspace::new(path, WorkspaceModel::default()));
        });
        let texts = render_texts(handle, cx);
        assert!(
            texts.iter().any(|t| t.contains("+ add solution")),
            "零 project 应给 add-solution 动作, 实际: {texts:?}"
        );
        assert!(
            !texts.iter().any(|t| t.contains("+ agent")),
            "零 project 不得渲染 [+ agent], 实际: {texts:?}"
        );
    }

    /// 7.1 全链路: 空态 → 创建打开 → add-solution 出项目 → 建会话
    /// (cwd=project root, 行入树, 状态栏正确) → 门控切换拦截。
    #[gpui_kit::test]
    fn full_chain_create_add_agent_and_gate(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app_with_history(cx, history);
        cx.run_until_parked();
        let texts = render_texts(handle, cx);
        assert!(
            texts.iter().any(|t| t.contains("no workspace open")),
            "链路起点: 空态"
        );

        // 模拟确认创建 + 打开
        let ws = temp_dir("chain");
        project_model::create_workspace_files(&ws).unwrap();
        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        cx.run_until_parked();
        cx.update(|cx| assert!(
            app_entity.read(cx).workspace.is_some(),
            "工作区已打开"
        ));

        // add-solution (同步合并缝)
        let project_dir = ws.join("bar");
        fs::create_dir_all(&project_dir).unwrap();
        app_entity.update(cx, |app, cx| {
            let items = project_model::append_simple_solution(&ws, project_dir.clone()).unwrap();
            app.apply_add_solution(project_dir.clone(), Ok(items), cx);
        });
        let texts = render_texts(handle, cx);
        assert!(
            texts.iter().any(|t| t.contains("bar")),
            "项目行名已渲染, 实际: {texts:?}"
        );
        let project_row_found = cx
            .update_window(handle, |_, window, _cx| {
                window
                    .try_find(format!("project-name-{}", project_dir.display()))
                    .is_some()
            })
            .expect("update window");
        assert!(project_row_found, "项目行 id 进观测注册表");

        // 建会话 (经服务创建, cwd = project root — spec: working directory;
        // .pi_bin 标记 → 工厂真 spawn stub, task 3.2)
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let stub_dir = std::env::temp_dir().join(format!("chain-stub-{unique}"));
        fs::create_dir_all(&stub_dir).unwrap();
        let stub = write_stub(&stub_dir);
        fs::write(
            project_dir.join(".pi_bin"),
            stub.to_str().unwrap(),
        )
        .unwrap();
        let (agent_id, _) =
            test_sessions::create_session(cx, &app_entity, &project_dir);
        cx.run_until_parked();

        let texts = render_texts(handle, cx);
        let agent_row_found = cx
            .update_window(handle, |_, window, _cx| {
                window
                    .try_find(format!("agent-name-{agent_id}"))
                    .is_some()
            })
            .expect("update window");
        assert!(
            agent_row_found,
            "会话行已渲染 (cwd == project root 分组命中)"
        );
        let project_str = project_dir.to_string_lossy().into_owned();
        assert!(
            texts.iter().any(|t| t.contains(&project_str)),
            "状态栏显示 project root (cwd), 实际: {texts:?}"
        );

        // 门控: Starting/Running 的会话挡住切换
        app_entity.update(cx, |app, cx| app.request_workspace_open(true, cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert!(
                app.notice.as_deref().is_some_and(|n| n.contains("still active")),
                "活动会话拦截切换 (spec)"
            );
            assert_eq!(app.workspace.as_ref().unwrap().path, ws, "工作区不变");
        });

        let _ = fs::remove_dir_all(&stub_dir);
    }

    /// change agent-service task 3.2 (spec delta: Session control failure
    /// feedback 三场景): 失败选/失败关/失败建 → 状态保持原样 + notice 点名。
    /// 失败注入: select/close = 未知 id (回执 Err 路径); create = `.pi_bin`
    /// 指向坏二进制 (工厂 spawn 失败 — spec 场景原文 "pi cannot be started")。
    #[gpui_kit::test]
    fn control_failure_feedback_covers_delta_scenarios(cx: &mut TestAppContext) {
        use agent_service::contract::SessionId;
        use crate::sidebar::SidebarEvent;
        use std::time::{Duration, Instant};

        let history = fresh_history_path();
        let (handle, app_entity) = open_app_with_history(cx, history);
        app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "delta-fail");
        });
        let ws_path =
            cx.update(|cx| app_entity.read(cx).workspace.as_ref().unwrap().path.clone());
        let (agent_id, shared) = test_sessions::create_session(cx, &app_entity, &ws_path);
        let (keys, _, _, _) = test_sessions::counters(&shared.expect("计数会话"));
        render_texts(handle, cx);

        let center_tab = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                let app = app_entity.read(cx);
                let area = app.dock_area.read(cx);
                crate::property_detail_manager::active_center_panel_id(area)
            })
        };
        let center_before = center_tab(cx);
        let wait_notice = |cx: &mut TestAppContext, needle: &str| {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                cx.run_until_parked();
                app_entity.update(cx, |app, cx| {
                    app.poll_sessions(cx);
                });
                let got = cx.update(|cx| app_entity.read(cx).notice.clone());
                if got.as_deref().is_some_and(|n| n.contains(needle)) {
                    break;
                }
                assert!(Instant::now() < deadline, "notice 未出现: {needle}");
                std::thread::sleep(Duration::from_millis(5));
            }
        };

        // ---- 场景 2: 失败选 (未知 id) → notice 点名 + active 不变 + 键仍达原会话
        cx.update_window(handle, |_, window, cx| {
            app_entity.update(cx, |app, cx| {
                app.notice = None;
                app.on_sidebar_event(SidebarEvent::SelectAgent(SessionId(9999)), window, cx);
            });
        })
        .expect("update window");
        wait_notice(cx, "切换会话失败");
        assert_eq!(
            cx.update(|cx| app_entity.read(cx).active),
            Some(agent_id),
            "失败 select 不动 active (spec delta 场景 2)"
        );
        assert_eq!(center_tab(cx), center_before, "失败 select 不切 console 签");
        app_entity.update(cx, |app, cx| app.send_key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('k'),
                crossterm::event::KeyModifiers::NONE,
            ),
            cx,
        ));
        test_sessions::wait_until(cx, || keys.get() >= 1, "原会话仍收键");

        // ---- 场景 1: 失败关 (未知 id) → 会话仍在清单 + active/签不变 + notice
        app_entity.update(cx, |app, cx| {
            app.notice = None;
            app.close_agent(SessionId(9998), cx);
        });
        wait_notice(cx, "关闭会话失败");
        let (len, active) = cx.update(|cx| {
            let app = app_entity.read(cx);
            let v = app
                .service_layer
                .sessions_view()
                .map(|v| (v.sessions.len(), v.active))
                .expect("投影在场");
            (v.0, v.1)
        });
        assert_eq!(len, 1, "失败关后会话仍在清单 (spec delta 场景 1)");
        assert_eq!(active, Some(agent_id), "失败关不动 active");
        assert_eq!(center_tab(cx), center_before, "失败关后 console 签内容不变");

        // ---- 场景 3: 失败建 (.pi_bin 指坏二进制) → 无新行 + notice 点名
        let bad_root = std::env::temp_dir().join(format!(
            "delta-badpi-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&bad_root).unwrap();
        fs::write(bad_root.join(".pi_bin"), "/nonexistent/pi-binary-xyz").unwrap();
        app_entity.update(cx, |app, cx| {
            app.notice = None;
            app.create_agent(&bad_root, cx);
        });
        wait_notice(cx, "创建会话失败");
        let len = cx.update(|cx| {
            app_entity.read(cx)
                .service_layer
                .sessions_view()
                .map(|v| v.sessions.len())
                .expect("投影在场")
        });
        assert_eq!(len, 1, "失败建无新行 (spec delta 场景 3)");
        let _ = fs::remove_dir_all(&bad_root);
        let _ = fs::remove_dir_all(&ws_path);
    }

    /// change agent-service task 3.3: 输入 tell 的信封面 — kind 进追踪面、
    /// 载荷 session id 精确投递 (双会话: 键只达 active, 非 active 零计数)。
    #[gpui_kit::test]
    fn input_tell_envelope_targets_active_session(cx: &mut TestAppContext) {
        use agent_service::contract::KIND_INPUT_KEY;

        let history = fresh_history_path();
        let (handle, app_entity) = open_app_with_history(cx, history);
        app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "envelope");
        });
        let ws_path =
            cx.update(|cx| app_entity.read(cx).workspace.as_ref().unwrap().path.clone());

        // 同 root 两个会话 (pi-1/pi-2); 后建者为 active (create 置 active)
        let (id_a, shared_a) = test_sessions::create_session(cx, &app_entity, &ws_path);
        let (id_b, shared_b) = test_sessions::create_session(cx, &app_entity, &ws_path);
        assert_ne!(id_a, id_b);
        assert_eq!(cx.update(|cx| app_entity.read(cx).active), Some(id_b));
        let (ka, _, _, _) = test_sessions::counters(&shared_a.expect("会话 A"));
        let (kb, _, _, _) = test_sessions::counters(&shared_b.expect("会话 B"));

        // send_key → tell(input.key): 载荷 session = active (B)
        app_entity.update(cx, |app, cx| {
            app.send_key(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('x'),
                    crossterm::event::KeyModifiers::NONE,
                ),
                cx,
            );
        });
        test_sessions::wait_until(cx, || kb.get() >= 1, "active 会话收到键");
        assert_eq!(ka.get(), 0, "载荷 session id 精确: 非 active 零投递");

        // kind 进追踪面 (manager 观测面: 一问一答全程留痕 — tell 也记 Out)
        let trace = cx.update(|cx| app_entity.read(cx).service_layer.manager.trace_entries());
        assert!(
            trace.iter().any(|t| t.kind == KIND_INPUT_KEY),
            "input.key 必须进追踪面, 实际 kinds: {:?}",
            trace.iter().map(|t| t.kind).collect::<Vec<_>>()
        );
        let _ = handle;
        let _ = fs::remove_dir_all(&ws_path);
    }

    // ===== 4.1: asset_manager 注册接线 =====

    /// 打开工作区 → 项目全量注册落树 (commit_open 钩子)。
    #[gpui_kit::test]
    fn workspace_open_registers_projects_into_manager(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app_with_history(cx, history);
        let ws = temp_dir("reg-open");
        let project_dir = ws.join("foo");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("lib.rs"), "x").unwrap();
        project_model::create_workspace_files(&ws).unwrap();
        project_model::append_simple_solution(&ws, project_dir.clone()).unwrap();

        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app_entity.read(cx);
            let view = app.asset_manager.read(cx).state();
            let AssetState::Forest { nodes, .. } = &view else {
                panic!("应注册落树, got {view:?}")
            };
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0].id, project_id(&project_dir));
            assert_eq!(nodes[0].kind, AssetKind::Project);
        });
    }

    /// add-solution → 增量注册, 新项目行无重启出现 (spec: register
    /// incrementally … without restarting)。
    #[gpui_kit::test]
    fn add_solution_registers_project_incrementally(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app_with_history(cx, history);
        let ws = temp_dir("reg-add");
        project_model::create_workspace_files(&ws).unwrap();
        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        cx.run_until_parked();
        // 零 solution → 空视图
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert_eq!(app.asset_manager.read(cx).state(), AssetState::Empty);
        });

        let pdir = ws.join("bar");
        fs::create_dir_all(&pdir).unwrap();
        fs::write(pdir.join("main.rs"), "x").unwrap();
        app_entity.update(cx, |app, cx| {
            let items = project_model::append_simple_solution(&ws, pdir.clone()).unwrap();
            app.apply_add_solution(pdir.clone(), Ok(items), cx);
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app_entity.read(cx);
            let view = app.asset_manager.read(cx).state();
            let AssetState::Forest { nodes, .. } = &view else {
                panic!("增量注册后应落树, got {view:?}")
            };
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0].id, project_id(&pdir));
        });
    }

    /// 切换工作区 → 注册集整体替换, 旧项目行无残留 (spec: Workspace
    /// switch replaces the registration … no stale rows)。
    #[gpui_kit::test]
    fn switching_workspace_replaces_registration(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app_with_history(cx, history);
        let ws1 = temp_dir("reg-switch-1");
        let one = ws1.join("one");
        fs::create_dir_all(&one).unwrap();
        project_model::create_workspace_files(&ws1).unwrap();
        project_model::append_simple_solution(&ws1, one.clone()).unwrap();
        let ws2 = temp_dir("reg-switch-2");
        let two = ws2.join("two");
        fs::create_dir_all(&two).unwrap();
        project_model::create_workspace_files(&ws2).unwrap();
        project_model::append_simple_solution(&ws2, two.clone()).unwrap();

        app_entity.update(cx, |app, cx| app.begin_open(ws1, cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app_entity.read(cx);
            let AssetState::Forest { nodes, .. } = app.asset_manager.read(cx).state() else {
                panic!("ws1 应落树")
            };
            assert_eq!(nodes[0].id, project_id(&one));
        });

        app_entity.update(cx, |app, cx| app.begin_open(ws2.clone(), cx));
        cx.run_until_parked();
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert_eq!(app.workspace.as_ref().unwrap().path, ws2);
            let AssetState::Forest { nodes, .. } = app.asset_manager.read(cx).state() else {
                panic!("ws2 应落树")
            };
            assert_eq!(nodes.len(), 1, "只剩 ws2 的注册");
            assert_eq!(nodes[0].id, project_id(&two));
            assert!(
                !nodes.iter().any(|n| n.id == project_id(&one)),
                "旧项目行不得残留"
            );
        });
    }
}
