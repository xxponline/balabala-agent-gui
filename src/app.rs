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
    SessionId, ThemeReq, KIND_CLOSE, KIND_CREATE, KIND_INPUT_CLICK, KIND_INPUT_KEY,
    KIND_INPUT_TEXT, KIND_INPUT_WHEEL, KIND_RESIZE, KIND_SELECT, KIND_THEME,
};
use service_manager::{Envelope, Payload, Route};
// change solution-service 2.3/5.2: 打开链路 load + create 均走服务 ask (design D8)
// solution-overview 2.5/3.x: create 经 `solution.action` 单点 (design D6),
// 回执按 action_id 路由 (design D7)
use solution_service::contract::{
    CreateReq as SolCreateReq, LoadReq, ProjectTree, TreeSnapshot,
    ACTION_CREATE as SOL_ACTION_CREATE,
};

use crate::service_layer::CtrlOp;
use agent_service::terminal::{AgentTerminal, TerminalCopy, TerminalTab};
use crate::property_detail_manager::{OpenObject, active_center_panel_id, handler_for};
use crate::shell;
use asset_service::panel::AssetOverview;
use crate::project_model::{self, MarkerStatus};
use crate::solution_overview::{SolutionOverviewEvent, SolutionOverviewPanel};
use crate::workspace::history::History;
use crate::workspace::Workspace;

// Tab 在 Terminal context 下的转发动作: gpui-base 的 Root 全局绑定
// `tab` → 焦点遍历会吞掉 Tab (原始 key event 不再派发), 更深的
// Terminal context 绑定胜出 (design D5, spike 已验证)。
use crate::property_detail_manager::md_panel::ToggleMdView;

/// 窗口内应用菜单栏 (spec: Application menu bar is always present — 非原生
/// 平台渲染在 dock 之上) 的定高, 单位 px。
///
/// 初值取 28px: 菜单项是 `Button::small()` = `size_6` = 24px, `AppMenu`
/// 又叠 `.py_0p5()` (design D5 Open Question — 真机目视后可调)。
pub const MENU_BAR_HEIGHT: f32 = 28.;

pub struct TerminalApp {
    /// 当前打开的工作区 (单实例, spec: Startup workspace restore;
    /// None = 空态, 等待目录选择器)
    pub workspace: Option<Workspace>,
    /// 树镜像 → 资产注册的装配单点记忆 `(workspace path, handoff 投影)`:
    /// 交树未变零动作 (change solution-service 5.4)。
    ///
    /// **只存交树投影而非整个快照** (change solution-overview D1): 描述符
    /// 变了但交树没变时不该触发全量资产重枚举；同时省掉整份快照的两次
    /// clone。
    last_handoff: Option<(PathBuf, ProjectTree)>,
    /// 打开代号: `begin_open` 自增, `commit_open` 校验 — 旧 load 的结果
    /// 直接丢弃 (design D5: 切换与在途加载的竞态)
    pub load_epoch: u64,
    /// 最近一次工作区打开失败 (错误框+退出路径的测试观测面;
    /// 正式构建在记录后 dialog + exit)
    pub open_failure: Option<String>,
    /// 切换门控提示 (spec: notice naming the count of active sessions)
    pub notice: Option<String>,
    /// 最近一次持久化失败的错误信息 (在 solution-overview 顶部显示; spec:
    /// Persistence write failure surfacing — 配置写失败红条)
    pub save_error: Option<String>,
    /// 历史文件路径 (测试缝: `new_inner` 注入; 正式 = 全局应用数据目录)
    pub history_path: PathBuf,
    /// DockArea 布局引擎 (design D1): 左|中|右三槽由它编排,
    /// 是 R3 外壳容器 (`render` 的根 div) 的子节点
    pub dock_area: Entity<DockArea>,
    /// 窗口内应用菜单栏 (win-in-window-app-menu-bar, specs/app-menu-bar):
    /// 非原生平台渲染在 dock 之上。`None` = 本平台走 OS 原生菜单
    /// (macOS, design D6 — 窗口内再挂一条会双份)。装配期由
    /// [`crate::menu::install`] 产出 (design D1: 先喂 GlobalState 再建)。
    menu_bar: Option<Entity<gpui_kit::component::menu::AppMenuBar>>,
    /// Copy 菜单项当前是否可用 = 有无选区 (design D2: 灰显只看选区, 焦点
    /// 不参与)。`render` 内比对翻转, **翻转才**重建菜单 (design D3)。
    /// 只存 bool 不存选区内容 — 重建的触发条件就是这个位的翻转。
    copy_enabled: bool,
    /// solution-overview 面板实体 (D2; notify 转发目标 — spike §10.5)
    pub solution_overview: Entity<SolutionOverviewPanel>,
    /// 终端面板实体 (4.1; 焦点在其内, 一步切焦点/步骤 5 进 DockArea 中心)
    pub terminal: Entity<AgentTerminal>,
    /// 资产树面板实体 `asset-overview` (原右槽文件树, right-dock-file-tree;
    /// notify 转发名单成员) —— change asset-service 5.1/5.3 改名
    pub asset_overview: Entity<AssetOverview>,
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
    /// SolutionConfig 表单签的面板实体（成功自关用 — dedup 真值仍以
    /// `open_tabs` 为准, 此处只存 Entity 供 `remove_panel` 调用）。
    pub(crate) sol_config_panel: Option<Entity<crate::solution_config::SolutionConfigPanel>>,
    /// 表单签 Create ask 在途标记（回执成败的消费门 — spec delta 结果分支）。
    pub(crate) sol_config_inflight: bool,
    /// 表单签行内错误（submit/回执失败展示; 开签即清）。
    pub(crate) sol_config_error: Option<String>,
    /// debug section 的 dedup 表 (specs/center-tabs delta): key = section id
    /// (**独立于**文件路径身份 — 同字符串互不干扰), 签关闭时经
    /// `debug_tab_removed` 摘除。跟着 `crate::debug` 的门控走 (release
    /// 无此字段 — design D5 零成本)。
    #[cfg(debug_assertions)]
    pub(crate) debug_tabs: HashMap<String, PanelId>,
}

/// 按**平台主修饰键**绑一个键位 (task 4.1/4.2, spec: Copy selection with
/// Cmd+C)。
///
/// 不能用 [`KeyBinding::new`] —— 它走无 OS 的 `Keystroke::parse`，而
/// `secondary` 分支必须有 OS 否则返回 `Err` → `new` 内的 `unwrap()` **panic**。
/// [`KeyBinding::load`] 用 `parse_with_os(cx.platform_info().os)` + 平台
/// keyboard mapper：`secondary` 解析成 macOS → `platform` (⌘)、
/// Windows/Linux → `control` (Ctrl)，正是 spec 要的平台语义。
///
/// `context` 传 `Some("Terminal")` 时内部走 `KeyBindingContextPredicate::parse`
/// 再 `into()` 成 `Rc<_>`（std blanket impl），与 `KeyBinding::new` 同款。
fn platform_binding(
    cx: &gpui_kit::App,
    keys: &'static str,
    action: impl gpui_kit::Action,
    context: Option<&str>,
) -> KeyBinding {
    KeyBinding::load(
        keys,
        Box::new(action),
        context.map(|c| {
            gpui_kit::KeyBindingContextPredicate::parse(c)
                .expect("静态 context 谓词应可解析")
                .into()
        }),
        false,
        None,
        cx,
    )
    .expect("键位串应含合法修饰键与主键")
}

impl TerminalApp {
    /// 启动入口: 默认读全局历史文件 (design D4)。
    ///
    /// 打开失败不从这里返回 — 经后台探测回主线程记录 `open_failure`
    /// 并 (正式构建) 对话框+退出; 返回 Err 的旧路径保留给 main 的防御分支。
    ///
    /// `startup` = Launcher 交棒的 path (design D2): `Some` → 开窗即异步
    /// 打开; `None` = 不启动打开流程 (测试缝 — 生产路径 Launcher 必给)。
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        startup: Option<PathBuf>,
    ) -> Result<Self, String> {
        Self::new_inner(
            window,
            cx,
            crate::workspace::history::default_path(),
            startup,
        )
    }

    /// 启动接线 (design D2): **不再读历史、不再自动恢复** — 启动 path 由
    /// Launcher 选定后传入 (spec: Startup opens the Launcher before any
    /// workspace)。`history_path` 保留为写侧通道 (`commit_open` 的 push);
    /// 它同时是测试缝 (hermetic: 测试不碰真实应用数据目录)。
    pub fn new_inner(
        window: &mut Window,
        cx: &mut Context<Self>,
        history_path: PathBuf,
        startup: Option<PathBuf>,
    ) -> Result<Self, String> {
        // Terminal context 下的 tab 终端转发绑定 (Root 的焦点遍历
        // 绑定在更浅的 context, 被这条压过 — design D5)
        //
        // 数组先在 `&App` 借用域内建好, 出块后再 `bind_keys` (需 `&mut App`)
        // — 避开同一次调用里 `&cx` / `&mut cx` 的重叠。
        let key_bindings = {
            let app: &gpui_kit::App = cx;
            [
                KeyBinding::new("tab", TerminalTab, Some("Terminal")),
                // 复制选区 (terminal-text-selection task 4.1, spec: Copy
                // selection with Cmd+C)。**用 `secondary-` 而非 `cmd-`**:
                // 见 [`platform_binding`] — `KeyBinding::new` 遇 `secondary`
                // 会 panic, 且 `cmd-` 在 Windows 解析成 Win 键而非 Ctrl。
                // Terminal context 深于全局, 不与文本框的复制键抢;
                // modifiers 已被 handle_key_down 拦, 不漏给会话。
                platform_binding(app, "secondary-c", TerminalCopy, Some("Terminal")),
                // md 查看切换 (md-viewer-toggle task 3.1): context=None 任意
                // context 匹配 — MdPanel 无专属 key_context, 且无键位冲突
                platform_binding(app, "secondary-shift-m", ToggleMdView, None),
            ]
        };
        cx.bind_keys(key_bindings);

        // 服务层启动接线 (task 6.1): 扫 plugins/ 建 Catalog + 装载核心假
        // 服务 + GUI mailbox/订阅; 失败即启动失败 (扫描目录不存在 = 空, 不错)
        let service_layer = crate::service_layer::ServiceLayer::bootstrap(Path::new("plugins"))?;

        // 装配 DockArea 三槽 (shell::build; panel_handle 包装见其文档)
        let weak_app = cx.weak_entity();
        // 面板 (asset-package-merge design D7: 构造注入, 无兄弟跳)
        let shell::ShellParts {
            dock_area,
            solution_overview,
            terminal,
            asset_overview,
        } = shell::build(weak_app, window, cx);
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
            last_handoff: None,
            load_epoch: 0,
            open_failure: None,
            notice: None,
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
            sol_config_panel: None,
            sol_config_inflight: false,
            sol_config_error: None,
            #[cfg(debug_assertions)]
            debug_tabs: HashMap::new(),
            dock_area,
            menu_bar: None, // 装配尾部由 menu::install 填入 (design D1)
            copy_enabled: false, // 装配期必无选区 (任务 3.2: 首帧即灰显)
            solution_overview,
            terminal,
            asset_overview,
            service_layer,
        };
        // Launcher 交棒的 path → 异步打开 (design D2/A1); `None` = 不启动
        // 打开流程 (测试缝 — 生产路径 Launcher 必给, spec 不允许跳过)。
        if let Some(path) = startup {
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
        // win-in-window-app-menu-bar design D1: install 内先 set_app_menus
        // 再建 AppMenuBar, 返回值存入字段 (macOS 恒 None)。
        app.menu_bar = crate::menu::install(cx, app.copy_enabled);
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

    /// 用户点了 Solution 行: 只切该行折叠 (change solution-overview D2 —
    /// 分组头左键 = 纯折叠, 无打开语义; 易失不持久化)。
    pub fn toggle_solution_collapse(&mut self, solution_id: &str) {
        if let Some(ws) = self.workspace.as_mut() {
            if !ws.collapsed_solutions.remove(solution_id) {
                ws.collapsed_solutions.insert(solution_id.to_string());
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
    /// active 会话身份 — 一律读 sessions 投影 (change solution-overview:
    /// 真值在服务, GUI 不再存本地字段; 服务未起 → None, 与旧字段初值同义)。
    pub(crate) fn active_session(&self) -> Option<SessionId> {
        self.service_layer.sessions_view().and_then(|v| v.active)
    }

    pub(crate) fn send_key(&mut self, key: crossterm::event::KeyEvent, cx: &mut Context<Self>) {
        if key.code == crossterm::event::KeyCode::Tab {
            self.tab_forwards += 1; // 诊断/测试观测 (spec: Tab 转发边界)
        }
        let Some(active) = self.active_session() else {
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
        let Some(active) = self.active_session() else {
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
        let Some(active) = self.active_session() else {
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
        let Some(active) = self.active_session() else {
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

    /// 发起一次工作区打开 (Launcher 交棒 / picker 确认后): epoch++ 后台探测
    /// +加载, 回主线程按 epoch 提交 (design D5: 旧结果丢弃)。返回本次
    /// epoch (测试观测)。
    ///
    /// 原 `request_workspace_open` (picker + 三支门控) 已随切换口删除,
    /// rfd 胶水归入 `crate::launcher` (launcher-startup 4.2)。
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
            // marker 探测后 solutions 加载归 solution 服务 (design D8:
            // GUI 一律 ask; 回执 = Result<TreeSnapshot, OpError>)
            let outcome = match probe {
                OpenProbe::Failure(msg) => OpenOutcome::Failure(msg),
                OpenProbe::Valid => {
                    // pull 模式纪律 (D6/D8): submit + 泵拉取 — one-shot await
                    // 会被确定性测试调度器判死 (外线程唤醒)。装载竞态 (loading
                    // 期提交/路由失败) 限时限次重试, 每轮真实让时给服务线程
                    // 调度窗口 (pdm 前科同款处置)。
                    const SUBMIT_TRIES: u32 = 100; // 50ms x 100 = 5s
                    const REPLY_TRIES: u32 = 1000; // 5ms x 1000 = 5s
                    let mut last_err = "solution 服务不可达".to_string();
                    let mut attempt = 0u32;
                    let corr = loop {
                        if attempt >= SUBMIT_TRIES {
                            break None;
                        }
                        let submitted = this.update(cx, |app, _| {
                            app.service_layer.submit_solution_load(Payload::new(LoadReq {
                                workspace_dir: path.clone(),
                            }))
                        });
                        match submitted {
                            Ok(Ok(corr)) => break Some(corr),
                            Ok(Err(e)) => last_err = format!("solution 服务不可达: {e}"),
                            Err(_) => break None, // app 消亡
                        }
                        attempt += 1;
                        cx.background_executor()
                            .spawn(async {
                                std::thread::sleep(Duration::from_millis(50));
                            })
                            .await;
                    };
                    let mut outcome = OpenOutcome::Failure(last_err);
                    if let Some(corr) = corr {
                        let mut waited = 0u32;
                        loop {
                            // 泵拉取 (mailbox pull — 无跨线程唤醒)
                            let got = this.update(cx, |app, _| {
                                app.service_layer.pump();
                                app.service_layer.take_load_result(corr)
                            });
                            match got {
                                Ok(Some(Ok(snap))) => {
                                    outcome = OpenOutcome::Ready(snap);
                                    break;
                                }
                                Ok(Some(Err(e))) => {
                                    outcome = OpenOutcome::Failure(e.to_string());
                                    break;
                                }
                                Ok(None) => {
                                    waited += 1;
                                    if waited >= REPLY_TRIES {
                                        this.update(cx, |app, _| {
                                            app.service_layer.abandon_load(corr);
                                        })
                                        .ok();
                                        outcome =
                                            OpenOutcome::Failure("solution 服务未响应".to_string());
                                        break;
                                    }
                                    cx.background_executor()
                                        .spawn(async {
                                            std::thread::sleep(Duration::from_millis(5));
                                        })
                                        .await;
                                }
                                Err(_) => break, // app 消亡: outcome 保持 Failure
                            }
                        }
                    }
                    outcome
                }
            };
            match outcome {
                OpenOutcome::Failure(msg) => {
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
                outcome => {
                    this.update(cx, |app, cx| {
                        app.commit_open(epoch, path, outcome, cx);
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
        outcome: OpenOutcome,
        cx: &mut Context<Self>,
    ) -> bool {
        if epoch != self.load_epoch {
            return false; // 过期结果 (4.2 竞态守卫)
        }
        match outcome {
            OpenOutcome::Failure(msg) => {
                // 正式路径已在 begin_open 里 dialog+exit; 测试观测面
                self.open_failure = Some(msg);
            }
            OpenOutcome::Ready(_snap) => {
                // 清场: 旧会话全部关停 (切换语义: exited 屏幕随切走丢弃,
                // spec: confirmation … discarded; 活动会话已被门控拦下)
                if self.workspace.take().is_some() {
                    // 会话本体归服务: 逐 id tell 关闭 (切换清场, D6 静默)。
                    // id 清单改读投影 (solution-overview 2.1: 无本地 agents
                    // 拷贝; 投影为活数据, 比旧 poll 拷贝更新鲜)
                    let stale: Vec<SessionId> = self
                        .service_layer
                        .sessions_view()
                        .map(|v| v.sessions.iter().map(|s| s.id).collect())
                        .unwrap_or_default();
                    for sid in stale {
                        let _ = self.service_layer.manager.tell(Envelope::new(
                            Route::Service("agent".into()),
                            self.service_layer.listener.address().clone(),
                            KIND_CLOSE,
                            Payload::new(CloseReq(sid)),
                        ));
                    }
                }
                self.notice = None;
                // 历史置顶 (便利数据: 失败仅日志, 不红条 — 非 workspace 配置)
                let mut history = History::load_at(&self.history_path);
                history.push(path.clone());
                if let Err(e) = history.save_at(&self.history_path) {
                    eprintln!("workspace history save failed: {e}");
                }
                // Solution/Project 域状态归服务镜像 (5.4: 视图读镜像,
                // GUI 不持本地副本); 资产注册经装配单点 (handoff 交树)
                self.workspace = Some(Workspace::new(path.clone()));
                self.sync_asset_registration(cx);
            }
        }
        cx.notify();
        self.notify_panels(cx);
        true
    }

    /// 右键菜单项 → solution 动作 ask (change solution-overview 4.3)。
    ///
    /// - **label 取自描述符** (design D3: label 归服务) — 由面板透传。
    /// - **args 现场构造** (design D4 修订 — 甲): 视图由 `checked` 取反,
    ///   不预构建 `Payload`(D3: 预构建会冻结快照期的值)。
    /// - 无参动作 `args = None`, 服务从自己持有的状态现算。
    pub(crate) fn invoke_solution_action(
        &mut self,
        node: &str,
        action_id: &str,
        label: &str,
        checked: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let args = solution_service::configure_action_args(action_id, checked).map(Payload::new);
        let result =
            self.service_layer
                .submit_action(action_id, Some(node.to_string()), args, label);
        match result {
            Ok(_) => self.kick_input_drain(cx), // 快泵: 回执尽快达
            Err(e) => {
                self.notice = Some(format!("{label}失败: {e}"));
            }
        }
        cx.notify();
        self.notify_panels(cx);
    }

    /// Create ask 发送点（表单签提交与测试共用缝; change solution-service
    /// 5.2 / solution-config-tab 3.1）。幂等前置检与 Server 同一
    /// ValidChecker（design D6）: 同 path 已存在 → 原样不变。提交入信箱
    /// 即置 `sol_config_inflight`（回执成败由 `poll_sessions` 消费 → 行内
    /// 错误 / 成功自关, specs/project-model delta 结果分支）。
    pub(crate) fn request_create_solution(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // 落盘目标由服务持有的绑定给出 (design D5) — 这里只需确认有工作区。
        if self.workspace.is_none() {
            return;
        }
        let dup = self.service_layer.tree_view().is_some_and(|t| {
            solution_service::create_is_noop(&t.solutions, &path)
                || t.tree().projects.iter().any(|p| p.root == path)
        });
        if dup {
            return; // spec: Re-adding an existing solution changes nothing
        }
        // 写入口单点 (design D6): create 也走 `solution.action`,
        // `node = None` (workspace 级动作, D4), 参数在 `args`。
        let result = self.service_layer.submit_action(
            SOL_ACTION_CREATE,
            None,
            Some(Payload::new(SolCreateReq { path })),
            "添加 solution",
        );
        match result {
            Ok(_) => {
                self.sol_config_inflight = true;
                self.sol_config_error = None;
                self.kick_input_drain(cx); // 快泵: 回执尽快达
            }
            Err(e) => {
                self.sol_config_inflight = false;
                self.sol_config_error = Some(format!("添加 solution 失败: {e}"));
                self.notice = Some(format!("添加 solution 失败: {e}"));
            }
        }
    }

    /// 树镜像变更 → 资产注册 (handoff 交树 = 注册唯一来源, 全量替换语义;
    /// change solution-service 5.1/5.2/5.4 装配单点)。**交树投影**未变 →
    /// 零动作 (O(1) 比对) — 层级里的描述符变化不进这个比对。
    fn sync_asset_registration(&mut self, cx: &mut Context<Self>) {
        // 投影现算后即脱离借用 — 后面要改 self.last_handoff
        let Some(handoff) = self.service_layer.tree_view().map(|s| s.handoff()) else {
            return;
        };
        let Some(path) = self.workspace.as_ref().map(|w| w.path.clone()) else {
            return;
        };
        if matches!(&self.last_handoff, Some((p, h)) if *p == path && *h == handoff) {
            return;
        }
        // ---- Project → 纯结构 DTO（design D3 修订：只做字段映射，
        // 不读文件、不带 filters —— 过滤集由服务每次枚举现场供给）----
        let projects: Vec<asset_service::contract::ProjectRegistration> = handoff
            .projects
            .iter()
            .map(|d| project_model::Project::from_desc(d).to_registration())
            .collect();
        let reg = asset_service::contract::AssetRegistration {
            workspace_dir: path.clone(),
            projects,
        };
        // 提交失败（服务尚未 Available）⇒ **不记 last_handoff**，本函数
        // 每个 pump 周期都会被调 ⇒ 下次自动重试（自愈，无需额外机制）。
        if self.service_layer.submit_asset_register(reg).is_err() {
            return; // 服务未 Available ⇒ 不记 last_handoff, 下次 pump 重试
        }
        self.last_handoff = Some((path.clone(), handoff));
        cx.notify();
        self.notify_panels(cx);
    }

    /// 右槽 ⟳ 入口 (asset-manager-right-dock 4.1): 请求 manager 重枚举
    /// (规则重读 + supersede 在途; 保展开是面板侧的事 — spec: Refresh
    /// triggers and behavior)。落地通知由 manager 经 notify_panels 转发。
    pub fn refresh_assets(&mut self, cx: &mut Context<Self>) {
        // ⟳ = 显式刷新 ask：服务 supersede 在途并**重读 filter 源**
        // （design D3：过滤集是每次枚举现场供给的函数，非快照）。
        let _ = self.service_layer.submit_asset_refresh();
        cx.notify();
        self.notify_panels(cx);
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

    /// solution-overview 事件漏斗单一入口 (design D3): 面板的所有交互
    /// 经此进入 model；SelectAgent 的一步切焦点也收拢在此 (3.2, spec:
    /// one-step focus — 选中即把键盘/IME 焦点交给 pane)。
    pub fn on_solution_overview_event(
        &mut self,
        ev: SolutionOverviewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 焦点交回 active tab 面板 (design D6): dock 的左 TabGroup 会把
        // 面板内的点击焦点抢给 solution-overview, 不交回则键盘/IME 全断。
        // (console 切换已移至控制成功回执 — spec delta: 失败不切)
        self.focus_active_center_panel(window, cx);
        match ev {
            SolutionOverviewEvent::AddSolution => {
                // 入口动作 = 开应用内表单签 (specs/center-tabs delta; 旧 rfd
                // 直连链已退役, task 2.2)
                self.open_solution_config_tab(window, cx);
                return;
            }
            SolutionOverviewEvent::ToggleSolutionCollapse(id) => {
                self.toggle_solution_collapse(&id)
            }
            SolutionOverviewEvent::ToggleProjectCollapse(root) => self.toggle_project_collapse(&root),
            SolutionOverviewEvent::NewAgent(root) => self.create_agent(&root, cx),
            SolutionOverviewEvent::SelectAgent(id) => {
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
            SolutionOverviewEvent::CloseAgent(id) => self.close_agent(id, cx),
        }
        cx.notify();
        self.notify_panels(cx);
    }

    /// 数据变化 → notify 展示该数据的面板 (spike §10.5 实测: 面板自读不
    /// 重建, 必须显式转发)。转发列表: solution-overview (badge/列表) + terminal
    /// (输出滚动) + asset-overview (资产树: 读镜像/代数比对/同步 TreeState —
    /// right-dock-file-tree 3.2)。
    ///
    /// **必须 `cx.defer`**: 事件 listener 挂在面板 entity 上, listener
    /// 执行期间面板持有 update lease — 同步 `panel.update` 会重入 panic
    /// (实测 crash: "cannot update SolutionOverviewPanel while it is already being
    /// updated", entity_map lease)。`App::defer` 的 effect 只在**最外层**
    /// update 的 finish 时 flush (pending_updates 守卫), 此时面板 lease
    /// 已 end_lease → 安全。
    pub(crate) fn notify_panels(&self, cx: &mut Context<Self>) {
        let panel = self.solution_overview.clone();
        let terminal = self.terminal.clone();
        let asset_overview = self.asset_overview.clone();
        // 表单签: 预检读树镜像/行内错误读 app — 变更须随轮询重绘
        // (solution-config-tab 3.1/3.3)
        let sol_config = self.sol_config_panel.clone();
        cx.defer(move |cx| {
            panel.update(cx, |_, cx| cx.notify());
            terminal.update(cx, |_, cx| cx.notify());
            // 树面板不是裸 notify: 要走"根比对→按需重扫→同步 TreeState"
            asset_overview.update(cx, |panel, cx| panel.on_data_changed(cx));
            if let Some(panel) = sol_config {
                panel.update(cx, |_, cx| cx.notify());
            }
        });
    }

    /// 统一菜单重建 (win-in-window-app-menu-bar design D3) — 两步：
    /// ① [`crate::menu::feed_global`] 把 `build()` 喂进 `GlobalState`；
    /// ② `menu_bar.reload()` 让窗口内实体重读该 GlobalState。
    ///
    /// macOS `menu_bar` 恒 `None` (design D6: 走原生, 挂载会双份), 第 ② 步
    /// no-op, 但第 ① 步照做 — 喂数据不做平台门控, 保持 macOS 侧单源。
    ///
    /// **调用方必须只在内容真正变化时调** (design D3): `reload()` 会重建
    /// 全部 `AppMenu` 实体并把 `selected_index` 清成 `None` — 每帧无脑推
    /// 等于“菜单一开就自关” (D2 自杀时序)。
    ///
    /// 先 clone 句柄再喂数据: 让 `&self` 的借用区在 `&mut App` (`feed_global`
    /// / `Entity::update` 要的) 之前就结束, 不与 `cx` 活叠。
    pub(crate) fn rebuild_menus(&self, cx: &mut Context<Self>) {
        let bar = self.menu_bar.clone(); // `self` 借用到此为止
        let copy_enabled = self.copy_enabled;
        crate::menu::feed_global(cx, copy_enabled);
        if let Some(bar) = bar {
            bar.update(cx, |bar, cx| bar.reload(cx));
        }
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
        // 树镜像变更 → 资产注册同步 (5.2/5.4 装配单点: handoff 交树)
        self.sync_asset_registration(cx);
        // 控制成功回执 → 中央切回 console (成功才切 — spec delta: 失败不切)
        if !self.service_layer.take_successes().is_empty() {
            self.switch_console_tab(cx);
            changed = true;
        }
        // solution Create 成功回执 → 表单签自关 (specs/project-model:
        // Successful creation closes the tab)。defer 落关 — 直接 remove 会
        // 在 on_removed 重入 TerminalApp (double-lease panic), 见
        // solution_config::close_solution_config_tab 文档。无签在场 = 空操作
        // (测试缝直调路径, inflight 照清)。
        let sol_oks = self.service_layer.take_sol_successes();
        if self.sol_config_inflight && sol_oks.iter().any(|id| id == SOL_ACTION_CREATE) {
            self.sol_config_inflight = false;
            self.close_solution_config_tab(cx);
            changed = true;
        }
        // 控制失败 notice (spec delta: 点名失败操作; 取最新一条)。表单签在
        // 途且动作是 create → 同步行内错误 (spec delta: failure → error
        // shown in the tab, 签保留)。
        // **路由按 action_id, 不按文案** (design D7) — 改 label 不改行为。
        if let Some((action, text)) = self.service_layer.take_notices().pop() {
            if self.sol_config_inflight && action.as_deref() == Some(SOL_ACTION_CREATE) {
                self.sol_config_inflight = false;
                self.sol_config_error = Some(text.clone());
            }
            self.notice = Some(text);
            changed = true;
        }
        // agent 首次就绪 → 补推主题 (服务快照默认深色, 宿主主题为准)
        if self.service_layer.take_agent_ready() {
            self.push_tui_theme(cx);
        }
        // 镜像同步进面板 (design D7 入站: set_view) — active/id 清单不再
        // 本地拷贝 (solution-overview: 一律读投影; 变化由 pump 消费服务
        // 事件置 changed, 通知链不丢 — design D4)
        if self.service_layer.sessions_view().is_some() {
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
    /// 目录不存在 / marker 缺失 / marker 坏 → 错误框 + 退出 (design D3/D4:
    /// Launcher 已做过建议性预检, 主窗再失效只可能是交棒后的竞态)。
    Failure(String),
    /// marker 合法 — solutions 加载归 solution 服务 (design D8)。
    Valid,
}

/// 打开结果 (solution 服务 load 回执 → commit 载荷; spec: Solution load
/// failures — 失败只有结构化 reason, 绝无部分树)。
#[derive(Debug, Clone)]
pub(crate) enum OpenOutcome {
    /// 加载失败 (marker 坏 / solutions 解析失败 / 死 path) → 错误框 + 退出。
    ///
    /// **唯一的 fatal 启动路径** (design D4): Launcher 已校验过的路径
    /// 再失效只可能是交棒后的竞态, 此时主窗已开、无处可退。
    Failure(String),
    /// 加载成功 — 载荷 = 服务交出的项目树快照 (含注册门控后的 handoff)。
    Ready(TreeSnapshot),
}

/// 后台探测: 目录 → marker (solutions 加载归 solution 服务, design D8)。
pub(crate) fn probe_open(path: &Path) -> OpenProbe {
    if !path.is_dir() {
        return OpenProbe::Failure(format!("{} does not exist", path.display()));
    }
    match project_model::marker_status(path) {
        MarkerStatus::Missing => {
            OpenProbe::Failure(format!("{} has no workspace marker", path.display()))
        }
        MarkerStatus::Corrupt(msg) => OpenProbe::Failure(msg),
        MarkerStatus::Valid(_) => OpenProbe::Valid,
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 灰显翻转检测 (task 3.2, specs/app-menu-bar: Copy item enabled state
        // follows selection changes; design D2 只看选区 + D3 翻转才重建)。
        //
        // **必须在构建元素树之前** —— 重建会 `AppMenuBar::reload()` 重建
        // 全部 AppMenu 实体, 而子实体在本帧稍后才 render; 先重建本帧才画出
        // 新状态。读 `terminal.selection` 是 pub 字段直读, 无订阅 —— 依赖
        // AgentTerminal 的 `cx.notify()` 经 mark_view_dirty 把祖先链上的
        // 本视图标脏 (design Risks 末条, task 3.3 验证)。
        let copy_enabled = self.terminal.read(cx).selection.is_some();
        if copy_enabled != self.copy_enabled {
            self.copy_enabled = copy_enabled;
            self.rebuild_menus(cx);
        }

        // 外壳 (design R3): DockArea 承担左|中|右三槽 (D1), 是本容器的
        // 子节点; 未来 titlebar / 状态栏 / 覆盖层挂同一容器。
        // (行数据/终端块的渲染在各面板 entity 内 — 3.1/4.1)
        //
        // 两个 wrapper 是 win-in-window-app-menu-bar design D5 的硬性要求:
        // 菜单栏 (AppMenuBar) 与 DockArea 的根节点都是 `size_full`
        // (w:100% h:100%), 直接并列作 flex_col 兄弟会各要 100% 高 →
        // 总高 200%, 菜单栏被压成半窗。故菜单栏位给定高 + 不收缩,
        // dock 位改 flex_1 + min_h(0) 让出剩余全部空间 (assets/panel.rs
        // 「flex 子项收缩 (pane 尺寸教训)」同款)。
        div()
            .size_full()
            .flex()
            .flex_col()
            // 菜单栏位: 定高 + 不收缩, 内部子节点 size_full 落到该定高上
            .child(
                div()
                    .h(px(MENU_BAR_HEIGHT))
                    .w_full()
                    .flex_shrink_0()
                    .id("app-menu-bar-slot")
                    // 观测锚点 (task 1.2): 无 test-support 特性时为 no-op
                    .test_support()
                    // 窗口内菜单栏实体 (task 2.2, spec: Application menu bar
                    // is always present)。**平台门控落在 menu.rs 的实例创建
                    // 处** (D6: macOS 走原生, `install` 恒返回 None) —
                    // Option 迭代即空, 天然不挂第二条菜单栏, 单一事实源。
                    // 相比把 cfg 分散到字段/字面量/赋值/挂载四处, 这里只判值。
                    .children(self.menu_bar.clone()),
            )
            // dock 位: 吃掉菜单栏之外的全部剩余高度
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    // 观测锚点 (task 1.2): 无 test-support 特性时为 no-op
                    .id("dock-slot")
                    .test_support()
                    .child(self.dock_area.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    //! 自适应网格分发的无头验证 (spec: Adaptive terminal grid sizing /
    //! Grid resize propagation): canvas paint 量出的 bounds 经 defer 分发,
    //! 换算与去重在真实渲染路径上成立。显式导入、不用 glob (避免
    //! `gpui_kit::*` 把 `#[test]` 宏遮蔽掉)。

    use super::TerminalApp;
    // 3.2 路由键测试: 直接发 action (绕过 request_create_solution 的固定文案)
    use super::{SOL_ACTION_CREATE, SolCreateReq};
    use service_manager::Payload;
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
        // 项目树经服务镜像注入 (5.4: 视图读镜像, 无本地副本 — 测试缝直写)
        use solution_service::{ProjectDesc, TreeSnapshot};
        let desc = ProjectDesc {
            name: path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/".to_string()),
            root: path.clone(),
            is_git: false,
        };
        app.service_layer.set_tree_view_for_test(TreeSnapshot {
            solutions: std::sync::Arc::from(vec![]),
            hierarchy: std::sync::Arc::from(vec![solution_service::SolutionNode {
                item: solution_service::SolutionItem::Simple {
                    path: path.clone(),
                    register_to_asset_manager: true,
                    git_enabled: false,
                },
                projects: std::sync::Arc::from(vec![solution_service::ProjectNode {
                    desc,
                    actions: std::sync::Arc::from([]),
                }]),
                actions: std::sync::Arc::from([]),
            }]),
        });
        app.workspace = Some(Workspace::new(path));
        full
    }

    /// 1.5: 交树投影收窄 (change solution-overview D1) —
    /// **仅描述符变化不触发**资产注册, 交树变化才触发。
    ///
    /// 起点故意把注册开关关掉: 此时 handoff 为空 → `register()` 因 owners
    /// 空而不 spawn 枚举 ⇒ 没有在途异步落树, `view_generation` 的断言是
    /// 确定性的, 不受后台枚举落地竞态影响。
    #[gpui_kit::test]
    fn asset_registration_ignores_action_descriptor_changes(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app(cx, history, None);

        // 代数 = 镜像代数（服务落地才推进）。
        let generation = |cx: &mut TestAppContext| -> u64 {
            cx.update(|cx| {
                app_entity
                    .read(cx)
                    .service_layer
                    .asset_view()
                    .map(|s| s.generation)
                    .unwrap_or(0)
            })
        };
        // 「不得启动枚举」的直接观测：`asset.register` 发送次数（GUI 侧
        // O(1) 守卫 —— 交树投影未变 ⇒ 连消息都不发）。
        let starts = |cx: &mut TestAppContext| -> u64 {
            cx.update(|cx| app_entity.read(cx).service_layer.asset_register_sends as u64)
        };
        /// 改镜像里首个 solution 节点的某个字段后重跑装配单点。
        fn mutate_and_sync(
            cx: &mut TestAppContext,
            app_entity: &gpui_kit::Entity<crate::app::TerminalApp>,
            f: impl FnOnce(&mut solution_service::SolutionNode),
        ) {
            app_entity.update(cx, |app, cx| {
                let mut snap = app.service_layer.tree_view().unwrap().clone();
                let mut hierarchy = snap.hierarchy.to_vec();
                f(&mut hierarchy[0]);
                snap.hierarchy = hierarchy.into();
                app.service_layer.set_tree_view_for_test(snap);
                app.sync_asset_registration(cx);
            });
            // 总线异步: 把注册的回执/事件消费进镜像再返回 —— 否则
            // base 快照可能在发布落地前取到 (A 会被晚到的发布踩脏)。
            for _ in 0..8 {
                cx.run_until_parked();
                app_entity.update(cx, |app, cx| {
                    app.poll_sessions(cx);
                });
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }

        // 起点: 注入 workspace + 树镜像, 再把注册开关关掉
        // → handoff 空 → register 被调但不 spawn 枚举
        app_entity.update(cx, |app, _cx| inject_test_workspace(app, "handoff-narrow"));
        mutate_and_sync(cx, &app_entity, |n| {
            n.item.set_register_to_asset_manager(false)
        });
        let base_generation = generation(cx);
        let base_starts = starts(cx);

        // A. 只改描述符 — 交树投影不变 ⇒ 不得重枚举
        mutate_and_sync(cx, &app_entity, |n| {
            n.actions = std::sync::Arc::from(vec![solution_service::ActionDesc {
                id: "configure.git_enabled".into(),
                label: "git 管理".into(),
                checked: Some(true),
                execution: solution_service::ExecutedBy::Service,
            }]);
        });
        assert_eq!(generation(cx), base_generation, "仅描述符变化不得触发资产注册");
        assert_eq!(starts(cx), base_starts, "…且不得启动枚举");

        // B. 交树变化 (注册开关打开) ⇒ 必须重枚举
        mutate_and_sync(cx, &app_entity, |n| {
            n.item.set_register_to_asset_manager(true)
        });
        assert!(generation(cx) > base_generation, "交树变化必须触发注册");
        assert!(starts(cx) > base_starts, "交树变化必须启动枚举");
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
                    cx.new(|cx| TerminalApp::new(window, cx, None).expect("app 启动"))
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
                    cx.new(|cx| TerminalApp::new(window, cx, None).expect("app 启动"))
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
                    cx.new(|cx| TerminalApp::new(window, cx, None).expect("app 启动"))
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
            cx.update(|cx| app_entity.read(cx).active_session()),
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
                let active = app.active_session();
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
                    cx.new(|cx| TerminalApp::new(window, cx, None).expect("app 启动"))
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
        // bounds 应排除 solution-overview (240px) 与 chrome: 1000px 窗口下明显小于全宽
        assert!(
            f32::from(bounds.size.width) < 800.,
            "网格 bounds 应排除 solution-overview 等 chrome: {}",
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

    /// win-in-window-app-menu-bar task 1.2 (spec: Windowed menu bar leaves
    /// the dock its full remaining space): 菜单栏位定高占顶, dock 位吃掉剩余
    /// 全部高度, 终端网格随之缩短但仍 >= 最小行数 —— 没有面板被挤出窗口。
    ///
    /// 与 `adaptive_grid_dispatch` 的关键区别: 那条写 `.pi_bin` 走真 spawn
    /// (Windows 上 `fake-pi.sh` 无法执行 → 既有 A 类失败, 本机不可执行);
    /// 本测试用无标记 root → `factory()` 产出进程内 `TestSession`, `feed`
    /// 同步建 parser, **全程零 spawn**, 故本机可跑。
    #[gpui_kit::test]
    fn menu_bar_slot_leaves_full_height_to_dock(cx: &mut TestAppContext) {
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
                    cx.new(|cx| TerminalApp::new(window, cx, None).expect("app 启动"))
                },
            )
            .expect("open test window")
        });

        // 不写 `.pi_bin` → factory 走进程内 TestSession (零 spawn)
        app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "menu-bar-slot");
        });
        let ws_path =
            cx.update(|cx| app_entity.read(cx).workspace.as_ref().unwrap().path.clone());
        fs::create_dir_all(&ws_path).unwrap();
        let (agent_id, _) = test_sessions::create_session(cx, &app_entity, &ws_path);
        // feed 同步建本地 parser → `terminal-grid` 才会渲染
        app_entity.update(cx, |app, cx| {
            app.terminal.update(cx, |t, cx| {
                t.feed(
                    agent_id,
                    b"probe\r\n",
                    agent_service::frame_gen::current(),
                    cx,
                );
            });
        });
        cx.run_until_parked();

        let _ = cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
        });
        cx.run_until_parked();

        let (win, slot, dock, grid, (_cell_w, line_h)) = cx
            .update_window(handle.into(), |_, window, _cx| {
                let win = window.bounds();
                let slot = window.find("app-menu-bar-slot").bounds();
                let dock = window.find("dock-slot").bounds();
                let grid = window.find("terminal-grid").bounds();
                let metrics = grid_metrics(window);
                (win, slot, dock, grid, metrics)
            })
            .expect("update window");

        let win_h = f32::from(win.size.height);
        let menu_h = super::MENU_BAR_HEIGHT;
        let eps = 1.; // 设备像素吸附容差

        // 菜单栏位: 从窗口顶起, 恰为定高
        assert!(
            f32::from(slot.origin.y) <= eps,
            "菜单栏位应贴窗口顶, 得 y={}",
            f32::from(slot.origin.y)
        );
        assert!(
            (f32::from(slot.size.height) - menu_h).abs() <= eps,
            "菜单栏位应恰为 {menu_h}px, 得 {}",
            f32::from(slot.size.height)
        );

        // dock 位: 紧接菜单栏之下, 一直铺到窗口底 (占满剩余空间)
        assert!(
            (f32::from(dock.origin.y) - menu_h).abs() <= eps,
            "dock 位应从菜单栏下沿 ({menu_h}px) 起, 得 {}",
            f32::from(dock.origin.y)
        );
        let dock_bottom = f32::from(dock.origin.y) + f32::from(dock.size.height);
        assert!(
            (dock_bottom - win_h).abs() <= eps,
            "dock 位应铺到窗口底 (占满剩余空间): bottom={dock_bottom} win_h={win_h}"
        );

        // spec 场景: 网格随让出的高度缩短, 但仍容得下最小行数
        assert!(
            f32::from(grid.size.height) < win_h,
            "网格高度应小于窗口高度 (菜单栏让位): grid={} win={}",
            f32::from(grid.size.height),
            win_h
        );
        let min_h = f32::from(MIN_GRID_ROWS) * f32::from(line_h);
        assert!(
            f32::from(grid.size.height) > min_h,
            "网格高度应大于 {MIN_GRID_ROWS} 行 ({min_h}px), 得 {}",
            f32::from(grid.size.height)
        );

        app_entity.update(cx, |app, cx| app.close_agent(agent_id, cx));
        cx.run_until_parked();
        let _ = fs::remove_dir_all(&ws_path);
    }

    /// 回归 (crash fix): solution-overview 行点击的 listener 在**面板 lease 内**执行,
    /// funnel 的 notify_panels 必须 `cx.defer` — 同步 panel.update 会重入
    /// panic/abort ("cannot update SolutionOverviewPanel while it is already being
    /// updated")。同时覆盖: 2.2 稳定身份载荷 +漏斗后面板确实重渲
    /// (defer 生效: 折叠后 [+ agent] 按钮从观测中消失)。
    #[gpui_kit::test]
    fn solution_overview_row_click_toggles_collapse(cx: &mut TestAppContext) {
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
                    cx.new(|cx| TerminalApp::new(window, cx, None).expect("app 启动"))
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
            (ws.path.to_string_lossy().into_owned(), ws.collapsed_projects.contains(&ws.path))
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
    fn solution_overview_select_grants_one_step_focus(cx: &mut TestAppContext) {
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
                    cx.new(|cx| TerminalApp::new(window, cx, None).expect("app 启动"))
                },
            )
            .expect("open test window")
        });

        // 注入 workspace 拿 root: 会话 cwd 必须 == project root
        // (spec: Session working directory), 否则 solution-overview 分组键
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
                app.solution_overview.update(cx, |_, cx| cx.notify());
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
                    cx.new(|cx| TerminalApp::new(window, cx, None).expect("app 启动"))
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
                app.solution_overview.update(cx, |_, cx| cx.notify());
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

    use super::OpenOutcome;
    use solution_service::contract::TreeSnapshot;
    use asset_service::contract::{AssetKind, AssetState, project_id};
    use crate::project_model::{self};
    use crate::workspace::history::History;

    /// hermetic 临时目录 (进程 + 纳秒隔离)。
    /// 等资产镜像满足条件（总线异步落地 ⇒ 取代旧 `AssetManager` 的同步
    /// 读；生产侧由 50ms 泵自愈，测试要显式泵到条件）。形态仿
    /// `wait_sol_settled`。
    fn wait_asset_settled(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
        pred: impl Fn(&AssetState) -> bool,
        what: &str,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            cx.run_until_parked();
            app.update(cx, |app, cx| {
                app.poll_sessions(cx);
            });
            if cx.update(|cx| pred(&asset_state(app.read(cx)))) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "资产镜像未落定: {what}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// 资产视图：读**服务推来的镜像**（change asset-service 4.5 ——
    /// `AssetManager` 已退役，无镜像按空态）。
    fn asset_state(app: &crate::app::TerminalApp) -> AssetState {
        app.service_layer
            .asset_view()
            .map(|s| s.state.clone())
            .unwrap_or(AssetState::Empty)
    }

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

    /// 起 app 并**显式指定启动 path** (3.1: 自动恢复退场后的入口)。
    /// `Some(path)` → `new_inner` 开窗即异步打开; `None` → 只装配不打开。
    fn open_app(
        cx: &mut TestAppContext,
        history: PathBuf,
        startup: Option<PathBuf>,
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
                    let startup = startup.clone();
                    cx.new(move |cx| {
                        TerminalApp::new_inner(window, cx, history.clone(), startup)
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

    /// solution 操作落定轮询 (change solution-service 5.2): 回执经
    /// mailbox 泵拉取 (pull 模式) — 每轮冲执行器 + 驱动 poll_sessions +
    /// 真实让时给服务线程, 5s 未落定 panic (带 what)。
    fn wait_sol_settled(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
        mut pred: impl FnMut(&TerminalApp) -> bool,
        what: &str,
    ) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            cx.run_until_parked();
            app.update(cx, |app, cx| {
                app.poll_sessions(cx);
            });
            if cx.update(|cx| pred(app.read(cx))) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "solution 操作未落定: {what}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// 让服务落盘必败 (design D5): 把 solutions 文件换成同名目录 →
    /// `fs::write` 必 EISDIR。落盘目标由服务绑定, 不能再靠改 view 侧的
    /// `workspace.path` 来影响写入位置。
    fn break_persist(ws: &std::path::Path) {
        let p = project_model::files::solutions_path(ws);
        let _ = fs::remove_file(&p);
        fs::create_dir_all(&p).unwrap();
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
        let (handle, app_entity) = open_app(cx, fresh_history_path(), None);
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

    /// 4.2: 过期 epoch 的提交必须被丢弃 (design D5 竞态守卫)。
    #[gpui_kit::test]
    fn stale_epoch_commit_is_discarded(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app(cx, history, None);
        app_entity.update(cx, |app, cx| {
            app.load_epoch = 7;
            let discarded = app.commit_open(
                6,
                "/old".into(),
                OpenOutcome::Ready(TreeSnapshot::default()),
                cx,
            );
            assert!(!discarded, "过期 epoch 必须返回 false 且不落状态");
            assert!(app.workspace.is_none(), "过期结果不得覆盖");
            let landed = app.commit_open(
                7,
                "/new".into(),
                OpenOutcome::Ready(TreeSnapshot::default()),
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
        let (_handle, app_entity) = open_app(cx, history, None);
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

    /// 主窗侧: 显式启动 path → 打开该工作区并载入其树。
    ///
    /// "从历史里挑最近的"已随自动恢复退场 — 那一半归 Launcher
    /// (`launcher::tests::history_entries_render_most_recent_first`);
    /// 本条只验交棒之后主窗侧的打开与装载 (launcher-startup 6.1)。
    #[gpui_kit::test]
    fn startup_opens_the_chosen_workspace(cx: &mut TestAppContext) {
        let ws = temp_dir("restore");
        let project_dir = ws.join("foo");
        fs::create_dir_all(&project_dir).unwrap();
        project_model::create_workspace_files(&ws).unwrap();
        fs::write(
            project_model::files::solutions_path(&ws),
            format!(
                r#"[{{"type": "simple", "path": "{}"}}]"#,
                project_dir.display()
            ),
        )
        .unwrap();
        let history = fresh_history_path();

        let (_handle, app_entity) = open_app(cx, history, Some(ws.clone()));
        // probe 在真实后台线程 — 限时轮询等落定 (run_until_parked 不等它)
        wait_startup_settled(
            cx,
            &app_entity,
            |app| app.workspace.is_some(),
            "打开交棒的 path",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert!(app.open_failure.is_none(), "合法工作区不报错");
            let ws_view = app.workspace.as_ref().expect("应打开交棒的 path");
            assert_eq!(ws_view.path, ws);
            let tree = app.service_layer.tree_view().expect("镜像在场");
            assert_eq!(tree.tree().projects.len(), 1);
            assert_eq!(tree.tree().projects[0].name, "foo");
            assert_eq!(tree.tree().projects[0].root, project_dir);
        });
    }

    /// 3.1: 坏 marker → open_failure 记录 (正式构建: 同点 dialog+exit)。
    /// (启动期的 marker 预检已前移到 Launcher — 本条验的是主窗侧
    ///  `probe_open` 的权威探测, 即 Launcher 交棒后的竞态路径)
    #[gpui_kit::test]
    fn startup_corrupt_marker_records_open_failure(cx: &mut TestAppContext) {
        let ws = temp_dir("corrupt-marker");
        fs::write(ws.join(project_model::files::MARKER_FILE), "{ nope").unwrap();
        let history = fresh_history_path();
        let mut h = History::default();
        h.push(ws.clone());
        h.save_at(&history).unwrap();

        let (_handle, app_entity) = open_app(cx, history, Some(ws));
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

    /// 3.2: 交棒后目录失效 → 记 open_failure 走 fatal, 而非静默回空态。
    /// (design D3: `Empty` 并入 `Failure` — Launcher 已预检过, 主窗再失效
    ///  只可能是竞态; `open_failure` 即错误框+退出路径的测试观测面)
    #[gpui_kit::test]
    fn startup_missing_target_is_fatal_not_empty_state(cx: &mut TestAppContext) {
        let ghost = std::env::temp_dir().join(format!("sw-ghost-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ghost);
        let history = fresh_history_path();
        let mut h = History::default();
        h.push(ghost.clone());
        h.save_at(&history).unwrap();

        let (_handle, app_entity) = open_app(cx, history, Some(ghost));
        wait_startup_settled(
            cx,
            &app_entity,
            |app| app.open_failure.is_some(),
            "交棒后目录失效应记 open_failure (fatal), 而非静默回空态",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert!(app.workspace.is_none(), "失效路径不得打开工作区");
            assert!(
                app.open_failure.as_ref().is_some_and(|m| m.contains("does not exist")),
                "错误点名目录不存在: {:?}",
                app.open_failure
            );
        });
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
        let (_handle, app_entity) = open_app(cx, history, None);
        app_entity.update(cx, |app, cx| {
            app.begin_open(ws.clone(), cx);
        });
        cx.run_until_parked();
        let back = cx.update(|cx| {
            let app = app_entity.read(cx);
            assert!(app.open_failure.is_none());
            let ws_view = app.workspace.as_ref().expect("创建后应打开");
            assert_eq!(ws_view.path, ws);
            assert!(
                app.service_layer
                    .tree_view()
                    .is_none_or(|t| t.tree().projects.is_empty()),
                "空 solutions = 零 project"
            );
            // 打开即写历史 (下次启动直开)
            History::load_at(&app.history_path)
        });
        assert_eq!(back.most_recent(), Some(ws.as_path()));
    }

    /// 5.2: create ask 合并 — 成功支进镜像 + 注册; 写失败支落 notice +
    /// **内存变更保留** (spec: in-memory state keeps the change — 服务侧
    /// 保留, 镜像照发; workspace-management: Save failure shown in UI)。
    #[gpui_kit::test]
    fn add_solution_merges_and_surfaces_write_failure(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app(cx, history, None);
        let ws = temp_dir("sol-merge");
        fs::create_dir_all(&ws).unwrap();
        project_model::create_workspace_files(&ws).unwrap();
        let p1 = ws.join("one");
        fs::create_dir_all(&p1).unwrap();
        let p2 = ws.join("two");
        fs::create_dir_all(&p2).unwrap();
        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        wait_sol_settled(cx, &app_entity, |app| app.workspace.is_some(), "打开落定");

        // 成功支: create p1 → 镜像 1 项, 无红条无 notice
        app_entity.update(cx, |app, cx| app.request_create_solution(p1.clone(), cx));
        wait_sol_settled(
            cx,
            &app_entity,
            |app| {
                app.service_layer
                    .tree_view()
                    .is_some_and(|t| t.tree().projects.len() == 1)
            },
            "create p1 落定",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            let t = app.service_layer.tree_view().unwrap();
            assert_eq!(t.tree().projects[0].root, p1);
            assert!(app.save_error.is_none(), "成功写入无红条");
            assert!(app.notice.is_none(), "成功无 notice");
        });

        // 写失败支: 把 solutions 文件换成同名目录 → 服务落盘必败。
        // (design D5 后落盘目标由服务绑定, 改 view 侧 `workspace.path`
        //  已不再影响写入位置 — 只能从绑定的目标下手)
        break_persist(&ws);
        app_entity.update(cx, |app, cx| {
            app.request_create_solution(p2.clone(), cx);
        });
        wait_sol_settled(cx, &app_entity, |app| app.notice.is_some(), "写失败 notice");
        cx.update(|cx| {
            let app = app_entity.read(cx);
            let n = app.notice.clone().unwrap();
            assert!(!n.is_empty(), "失败产生 notice: {n}");
            // 路由按 action_id (design D7) — 落签即证明, 不再匹配文案
            assert!(
                app.sol_config_error.is_some(),
                "失败按 action_id 路由到表单签"
            );
            // 内存变更保留 (spec): 镜像仍含 p2
            let t = app.service_layer.tree_view().unwrap();
            assert_eq!(t.tree().projects.len(), 2, "写失败仍进内存 (spec)");
            assert!(t.tree().projects.iter().any(|p| p.root == p2));
        });
    }

    /// 3.2: 路由键 = action_id — **改 label 文案不影响行内错误落签**。
    /// (design D7: 之前靠对 notice 文案做子串匹配来路由 — 文案一改错误
    ///  就静默漏掉; 现按 action_id)
    #[gpui_kit::test]
    fn inline_error_routing_ignores_label_wording(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);
        cx.run_until_parked();

        // 开表单签 (走真实入口)
        cx.update_window(handle, |_, window, cx| {
            app_entity.update(cx, |app, cx| app.open_solution_config_tab(window, cx));
        })
        .expect("open solution config tab");
        cx.run_until_parked();

        // 用一个与 "添加 solution" 毫无共同子串的 label 提交; 无工作区 →
        // 服务回 "no workspace bound" 失败
        let path = std::env::temp_dir().join(format!("lbl-fail-{}", std::process::id()));
        app_entity.update(cx, |app, _cx| {
            app.service_layer
                .submit_action(
                    SOL_ACTION_CREATE,
                    None,
                    Some(Payload::new(SolCreateReq { path })),
                    "ZZZ-unrelated-wording",
                )
                .expect("submit create");
            app.sol_config_inflight = true;
        });
        wait_sol_settled(
            cx,
            &app_entity,
            |a| a.sol_config_error.is_some(),
            "行内错误按 action_id 落签",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            let err = app.sol_config_error.clone().expect("行内错误应落签");
            assert!(
                err.contains("ZZZ-unrelated-wording"),
                "文案原样显示 (label 只管显示): {err}"
            );
            assert!(
                app.sol_config_panel.is_some(),
                "失败时签保留 (spec: the tab stays)"
            );
            assert!(!app.sol_config_inflight, "回执已消费");
        });
    }

    /// 3.2: 路由键 = action_id — **改 label 文案不影响表单签自关**。
    #[gpui_kit::test]
    fn tab_auto_close_ignores_label_wording(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);
        let ws = temp_dir("lbl-close");
        fs::create_dir_all(&ws).unwrap();
        project_model::create_workspace_files(&ws).unwrap();
        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        wait_sol_settled(cx, &app_entity, |a| a.workspace.is_some(), "工作区打开");

        cx.update_window(handle, |_, window, cx| {
            app_entity.update(cx, |app, cx| app.open_solution_config_tab(window, cx));
        })
        .expect("open solution config tab");
        cx.run_until_parked();

        let target = ws.join("newproj");
        fs::create_dir_all(&target).unwrap();
        app_entity.update(cx, |app, _cx| {
            app.service_layer
                .submit_action(
                    SOL_ACTION_CREATE,
                    None,
                    Some(Payload::new(SolCreateReq { path: target })),
                    "ZZZ-unrelated-wording",
                )
                .expect("submit create");
            app.sol_config_inflight = true;
        });
        wait_sol_settled(
            cx,
            &app_entity,
            |a| !a.sol_config_inflight,
            "成功回执清 inflight",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert!(
                app.sol_config_panel.is_none(),
                "label 文案不同, 表单签仍自关 (路由按 action_id)"
            );
        });
        let _ = fs::remove_dir_all(&ws);
    }

    /// 5.1+5.4: agent 按 project root 分组 (含工作区外项目) + 状态栏显示
    /// project root 路径 (spec: Console status bar content)。
    #[gpui_kit::test]
    fn agents_group_under_project_roots_and_status_bar_shows_root(
        cx: &mut TestAppContext,
    ) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);
        let external = temp_dir("external-project"); // 工作区外的 project
        let ws_path = app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "grouping"); // project root1 = ws 目录
            let ws_path = app.workspace.as_ref().unwrap().path.clone();
            // 工作区外 project 经镜像追加 (5.4: 视图读镜像)。
            // tree() 从 hierarchy 派生 → 追加即塞一个 solution 节点
            // (spec: Root outside the workspace is accepted — 外部目录本
            //  就是一个合法 solution 的 project)。
            let mut snap = app.service_layer.tree_view().unwrap().clone();
            let mut hierarchy = snap.hierarchy.to_vec();
            hierarchy.push(solution_service::SolutionNode {
                item: solution_service::SolutionItem::Simple {
                    path: external.clone(),
                    register_to_asset_manager: true,
                    git_enabled: false,
                },
                projects: std::sync::Arc::from(vec![solution_service::ProjectNode {
                    desc: solution_service::ProjectDesc {
                        name: "external-project".into(),
                        root: external.clone(),
                        is_git: false,
                    },
                    actions: std::sync::Arc::from([]),
                }]),
                actions: std::sync::Arc::from([]),
            });
            snap.hierarchy = hierarchy.into();
            app.service_layer.set_tree_view_for_test(snap);
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
            // 状态栏外部 root: 投影已置 active (服务新建即 active,
            // create_session 自举已 sync_terminal_view — 无需直写)
            app.solution_overview.update(cx, |_, cx| cx.notify());
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
        let (handle, app_entity) = open_app(cx, history, None);
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
            app.solution_overview.update(cx, |_, cx| cx.notify());
        });
        cx.run_until_parked();
        render_texts(handle, cx); // 首帧让行进观测注册表

        let (active_before, tab_before) = app_entity.update(cx, |app, _cx| {
            let area = app.dock_area.read(_cx);
            (
                app.active_session(),
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
                app.active_session(),
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

    /// 取中心槽的 (签列表, 活动下标) — 断言"没切 tab"用。
    fn center_tabs(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
    ) -> (Vec<gpui_kit::base::dock::PanelId>, usize) {
        cx.update(|cx| {
            let app = app.read(cx);
            let area = app.dock_area.read(cx);
            let tree = area
                .layout(gpui_kit::component::dock::DockPlacement::Center)
                .expect("center 槽在场");
            let mut found = (Vec::new(), 0usize);
            tree.root().walk(&mut |node| {
                if let gpui_kit::base::dock::layout::PaneRef::Tabs { panels, active_ix } =
                    node.kind()
                {
                    if found.0.is_empty() {
                        found = (panels.to_vec(), active_ix);
                    }
                }
            });
            found
        })
    }

    /// 取元素 id 列表 (文档序) 与 test_text 列表。
    fn ids_and_texts(
        handle: gpui_kit::AnyWindowHandle,
        cx: &mut TestAppContext,
    ) -> (Vec<String>, Vec<String>) {
        use gpui_kit::test::TestQueryExt;
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let ids = window
                .elements()
                .iter()
                .filter_map(|e| e.path().last().map(|id| id.to_string()))
                .collect::<Vec<_>>();
            let texts = gpui_kit::base::test_support::texts(window)
                .iter()
                .map(|t| t.text().to_string())
                .collect::<Vec<_>>();
            (ids, texts)
        })
        .expect("update window")
    }

    /// 2.5: 表单提交经 `ActionInvoke` → solution 行出现;
    /// `create_is_noop` 前置检行为不变 (重复提交零变化)。
    #[gpui_kit::test]
    fn form_submit_creates_a_solution_row_via_action_invoke(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);

        let ws = temp_dir("form-create");
        fs::create_dir_all(&ws).unwrap();
        project_model::create_workspace_files(&ws).unwrap();
        let target = ws.join("newproj");
        fs::create_dir_all(&target).unwrap();

        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        wait_sol_settled(cx, &app_entity, |a| a.workspace.is_some(), "工作区打开");

        // 提交 — 走 request_create_solution → `ActionInvoke` 单点 (design D6)
        app_entity.update(cx, |app, cx| {
            app.request_create_solution(target.clone(), cx)
        });
        wait_sol_settled(
            cx,
            &app_entity,
            |a| {
                a.service_layer
                    .tree_view()
                    .is_some_and(|t| t.hierarchy.len() == 1)
            },
            "create 落定",
        );

        let texts = render_texts(handle, cx);
        assert!(
            texts.iter().any(|t| t.contains("· simple")),
            "提交后 solution 行出现, 实际: {texts:?}"
        );

        // create_is_noop 前置检 (design D6: view 与 Server 同一 ValidChecker)
        app_entity.update(cx, |app, cx| {
            app.request_create_solution(target.clone(), cx)
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let t = app_entity
                .read(cx)
                .service_layer
                .tree_view()
                .expect("镜像在场");
            assert_eq!(t.hierarchy.len(), 1, "重复提交零变化 (create_is_noop)");
            assert_eq!(t.tree().projects.len(), 1, "无重复 project");
        });
        let _ = fs::remove_dir_all(&ws);
    }

    /// 4.4: 空态条件 = **零 solution** (project-model delta) —
    /// 零 solution 渲染 add-solution 入口, 有 solution 后消失。
    #[gpui_kit::test]
    fn add_solution_entry_disappears_once_a_solution_exists(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);

        // workspace 已开但镜像里零 solution → 入口在场
        app_entity.update(cx, |app, _cx| {
            app.workspace = Some(Workspace::new(
                std::env::temp_dir().join(format!("add-sol-empty-{}", std::process::id())),
            ));
        });
        cx.run_until_parked();
        let texts = render_texts(handle, cx);
        assert!(
            texts.iter().any(|t| t.contains("+ add solution")),
            "零 solution 应渲染 add-solution 入口, 实际: {texts:?}"
        );

        // 镜像出现一个 solution → 入口消失
        app_entity.update(cx, |app, _cx| inject_test_workspace(app, "add-sol-has"));
        cx.run_until_parked();
        let texts = render_texts(handle, cx);
        assert!(
            !texts.iter().any(|t| t.contains("+ add solution")),
            "有 solution 后入口消失, 实际: {texts:?}"
        );
    }

    /// 4.1: solution 行渲染 (change solution-overview D2) —
    /// 位于其 project 之上、id 带 `solution://` 命名空间前缀、与 project
    /// 行 id 不相等 (v1 两者 path 相同, 裸路径会撞)。
    #[gpui_kit::test]
    fn solution_row_renders_above_its_project_with_namespaced_id(
        cx: &mut TestAppContext,
    ) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);
        app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "sol-row"); // ws 目录即 project root
        });
        cx.run_until_parked();

        // 用内层 `*-name-*` 观测锚点 (外层 row div 无 test_support, 且
        // 点击/观测都走内层 — 冒泡是本仓既定模式)
        let (ids, texts) = ids_and_texts(handle, cx);
        let sol_pos = ids
            .iter()
            .position(|i| i.starts_with("solution-name-solution://"))
            .unwrap_or_else(|| panic!("solution 行应渲染, 实际 id: {ids:?}"));
        let proj_pos = ids
            .iter()
            .position(|i| i.starts_with("project-name-/"))
            .unwrap_or_else(|| panic!("project 行应渲染, 实际 id: {ids:?}"));

        assert!(
            sol_pos < proj_pos,
            "solution 行在其 project 之上 (文档序), ids: {ids:?}"
        );
        let sol_id = &ids[sol_pos];
        let proj_id = &ids[proj_pos];
        assert!(
            sol_id.contains("solution://"),
            "solution 行 id 带命名空间前缀: {sol_id}"
        );
        assert_ne!(sol_id, proj_id, "两行 id 不相等 (D2: v1 path 相同)");

        assert!(
            texts.iter().any(|t| t.contains("· simple")),
            "label 带类型标记 (与同名 project 行的唯一区分), 实际: {texts:?}"
        );
    }

    /// 4.3: 右键 Solution 行 → 菜单**来自服务声明的 actions** (同步、零往返)
    /// → 点击发 `ActionInvoke` → 服务处理 (交树收缩)。
    /// (specs/node-actions: Menu is built from the snapshot;
    ///  spec: Solution row context menu offers declared actions)
    #[gpui_kit::test]
    fn solution_row_context_menu_dispatches_declared_actions(
        cx: &mut TestAppContext,
    ) {
        use gpui_kit::Role;
        use gpui_kit::test::{TestQueryExt, TestWindowExt};

        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);

        // 真实打开: 服务侧载入一个 register=true 的 simple solution
        let ws = temp_dir("sol-menu");
        fs::create_dir_all(&ws).unwrap();
        project_model::create_workspace_files(&ws).unwrap();
        let proj = ws.join("proj");
        fs::create_dir_all(&proj).unwrap();
        fs::write(
            project_model::files::solutions_path(&ws),
            format!(r#"[{{"type": "simple", "path": "{}"}}]"#, proj.display()),
        )
        .unwrap();
        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        wait_sol_settled(cx, &app_entity, |a| a.workspace.is_some(), "工作区打开");
        cx.run_until_parked();

        // 期望 label 从**镜像的服务声明**取 — 不是视图自己的文案表
        let expected: Vec<String> = cx.update(|cx| {
            app_entity
                .read(cx)
                .service_layer
                .tree_view()
                .expect("镜像在场")
                .hierarchy[0]
                .actions
                .iter()
                .map(|a| a.label.clone())
                .collect::<Vec<_>>()
        });
        assert!(!expected.is_empty(), "服务应声明至少一个动作");

        let sol_id = format!("solution-name-solution://{}", proj.display());
        let tabs_before = center_tabs(cx, &app_entity);

        // 左键 = 纯折叠, 不打开任何签 (D2)
        cx.update_window(handle, |_, window, cx| window.click(sol_id.clone(), cx))
            .expect("left click solution row");
        cx.run_until_parked();
        assert_eq!(
            center_tabs(cx, &app_entity),
            tabs_before,
            "左键不打开任何签"
        );
        // 再点展开, 以免折叠影响右键命中
        cx.update_window(handle, |_, window, cx| window.click(sol_id.clone(), cx))
            .expect("left click again");
        cx.run_until_parked();

        // 右键 → 菜单
        cx.update_window(handle, |_, window, cx| window.right_click(sol_id, cx))
            .expect("right click solution row");
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .expect("render");

        let labels = cx.update_window(handle, |_, window, _| {
            window
                .find_all_by_role(Role::MenuItem)
                .iter()
                .filter_map(|i| i.label().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .expect("menu items");
        assert_eq!(
            labels, expected,
            "菜单项与服务声明的 label 一致 (label 归服务, design D3)"
        );

        // 点第一项 (注册到资产树, 当前 true → 翻 false) → 交树收缩
        let item_id = cx.update_window(handle, |_, window, _| {
            window.find_all_by_role(Role::MenuItem)[0]
                .path()
                .last()
                .expect("菜单项路径")
                .clone()
        })
        .expect("menu item id");
        cx.update_window(handle, |_, window, cx| {
            let mut menu = window.within("popup-menu");
            menu.click(item_id, cx);
        })
        .expect("click menu item");
        wait_sol_settled(
            cx,
            &app_entity,
            |a| {
                a.service_layer
                    .tree_view()
                    .is_some_and(|t| t.handoff().projects.is_empty())
                },
            "点击后交树收缩 (ActionInvoke 已被服务处理)",
        );
        // 零声明 → 无菜单 (specs/node-actions: A node with no declared
        // actions has no menu): project 行不挂 context_menu
        let proj_row = format!("project-name-{}", proj.display());
        cx.update_window(handle, |_, window, cx| window.right_click(proj_row, cx))
            .expect("right click project row");
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .expect("render");
        let on_project = cx.update_window(handle, |_, window, _| {
            window.find_all_by_role(Role::MenuItem).len()
        })
        .expect("menu items on project row");
        assert_eq!(on_project, 0, "未声明动作的 project 行右键无菜单");

        let _ = fs::remove_dir_all(&ws);
    }

    /// 4.2: Solution 行左键 = **纯折叠** (D2) — 隐藏子行, 不动 active
    /// session、不切 tab、不抢焦点; 折叠态易失 (类型上就无持久化)。
    #[gpui_kit::test]
    fn solution_row_click_toggles_collapse_only(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);
        let full = app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "sol-collapse")
        });
        cx.run_until_parked();

        let sol_id = format!("solution-name-solution://{}", full);
        let (ids_before, _) = ids_and_texts(handle, cx);
        assert!(
            ids_before.iter().any(|i| i.starts_with("project-name-/")),
            "折叠前 project 行在场, 实际: {ids_before:?}"
        );
        let active_before = cx.update(|cx| {
            app_entity
                .read(cx)
                .service_layer
                .sessions_view()
                .and_then(|v| v.active)
        });
        let tabs_before = center_tabs(cx, &app_entity);

        // 点 solution 行 → 折叠
        cx.update_window(handle, |_, window, cx| window.click(sol_id.clone(), cx))
            .expect("click solution row");
        cx.run_until_parked();

        let (ids_after, _) = ids_and_texts(handle, cx);
        assert!(
            !ids_after.iter().any(|i| i.starts_with("project-name-/")),
            "折叠隐藏子行, 实际: {ids_after:?}"
        );
        assert!(
            ids_after.iter().any(|i| i.starts_with("solution-name-solution://")),
            "分组头自身仍可见, 实际: {ids_after:?}"
        );
        let active_after = cx.update(|cx| {
            app_entity
                .read(cx)
                .service_layer
                .sessions_view()
                .and_then(|v| v.active)
        });
        assert_eq!(active_after, active_before, "不切 active session");
        assert_eq!(center_tabs(cx, &app_entity), tabs_before, "不切 tab");
        assert!(
            cx.update(|cx| {
                app_entity
                    .read(cx)
                    .workspace
                    .as_ref()
                    .unwrap()
                    .collapsed_solutions
                    .contains(&format!("solution://{}", full))
            }),
            "折叠态记在 collapsed_solutions (键 = 节点 id)"
        );

        // 再点 → 展开
        cx.update_window(handle, |_, window, cx| window.click(sol_id, cx))
            .expect("click solution row again");
        cx.run_until_parked();
        let (ids_again, _) = ids_and_texts(handle, cx);
        assert!(
            ids_again.iter().any(|i| i.starts_with("project-name-/")),
            "再点恢复子行, 实际: {ids_again:?}"
        );

        // 易失: 新建的 Workspace 不带折叠态 (类型上无持久化)
        assert!(
            crate::workspace::Workspace::new(std::path::PathBuf::from("/fresh"))
                .collapsed_solutions
                .is_empty(),
            "折叠态不持久化 — 新 workspace 默认展开"
        );
    }

    /// 5.2: 零 project 工作区 → 只给 add-solution 动作, 不渲染 [+ agent]
    /// (spec: If no project exists yet, no [+ agent] button)。
    #[gpui_kit::test]
    fn no_project_workspace_renders_add_solution_not_agent_button(
        cx: &mut TestAppContext,
    ) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);
        app_entity.update(cx, |app, _| {
            let path = std::env::temp_dir().join(format!(
                "sw-noproj-{}-{}",
                std::process::id(),
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
            ));
            app.workspace = Some(Workspace::new(path));
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

    /// 4.1: 切换入口与空态提示两块渲染已删 (launcher-startup 4.1)。
    /// 按**元素 id** 断言 (文案会随文案改版漂移, id 不会)。
    #[gpui_kit::test]
    fn solution_overview_renders_neither_switch_entry_nor_empty_hint(
        cx: &mut TestAppContext,
    ) {
        use gpui_kit::test::TestQueryExt;

        fn element_ids(
            cx: &mut TestAppContext,
            handle: gpui_kit::AnyWindowHandle,
        ) -> Vec<String> {
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window
                    .elements()
                    .iter()
                    .filter_map(|e| e.path().last().map(|id| id.to_string()))
                    .collect()
            })
            .expect("update window")
        }

        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);
        cx.run_until_parked();

        // 无 workspace: 原空态提示块
        let ids = element_ids(cx, handle);
        assert!(
            !ids.iter().any(|i| i == "workspace-empty-hint"),
            "空态提示块已删, 实际: {ids:?}"
        );

        // 有 workspace: 切换入口原本恒在 (空态与已开态都是同一个控件)
        app_entity.update(cx, |app, _| {
            let path = std::env::temp_dir().join(format!("sw-noswitch-{}", std::process::id()));
            app.workspace = Some(Workspace::new(path));
        });
        let ids = element_ids(cx, handle);
        assert!(
            !ids.iter().any(|i| i == "workspace-switch"),
            "切换入口块已删, 实际: {ids:?}"
        );

        // 不受影响: add-solution 动作照旧 (spec: Solution creation path picker)
        let texts = render_texts(handle, cx);
        assert!(
            texts.iter().any(|t| t.contains("+ add solution")),
            "add-solution 动作仍在, 实际: {texts:?}"
        );
    }

    /// 7.1 全链路: 打开 → add-solution 出项目 → 建会话
    /// (cwd=project root, 行入树, 状态栏正确)。
    ///
    /// 原链路起点的空态断言与门控尾部已随切换口删除
    /// (launcher-startup 4.4 / 6.2 — 空态不可达)。
    #[gpui_kit::test]
    fn full_chain_create_add_agent(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);
        cx.run_until_parked();

        // 模拟确认创建 + 打开
        let ws = temp_dir("chain");
        project_model::create_workspace_files(&ws).unwrap();
        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        cx.run_until_parked();
        cx.update(|cx| assert!(
            app_entity.read(cx).workspace.is_some(),
            "工作区已打开"
        ));

        // add-solution (5.2: create ask + 镜像/注册同步)
        let project_dir = ws.join("bar");
        fs::create_dir_all(&project_dir).unwrap();
        app_entity.update(cx, |app, cx| app.request_create_solution(project_dir.clone(), cx));
        wait_sol_settled(
            cx,
            &app_entity,
            |app| {
                app.service_layer
                    .tree_view()
                    .is_some_and(|t| t.tree().projects.iter().any(|p| p.root == project_dir))
            },
            "add-solution 落定",
        );
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

        let _ = fs::remove_dir_all(&stub_dir);
    }

    /// change agent-service task 3.2 (spec delta: Session control failure
    /// feedback 三场景): 失败选/失败关/失败建 → 状态保持原样 + notice 点名。
    /// 失败注入: select/close = 未知 id (回执 Err 路径); create = `.pi_bin`
    /// 指向坏二进制 (工厂 spawn 失败 — spec 场景原文 "pi cannot be started")。
    #[gpui_kit::test]
    fn control_failure_feedback_covers_delta_scenarios(cx: &mut TestAppContext) {
        use agent_service::contract::SessionId;
        use crate::solution_overview::SolutionOverviewEvent;
        use std::time::{Duration, Instant};

        let history = fresh_history_path();
        let (handle, app_entity) = open_app(cx, history, None);
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
                app.on_solution_overview_event(SolutionOverviewEvent::SelectAgent(SessionId(9999)), window, cx);
            });
        })
        .expect("update window");
        wait_notice(cx, "切换会话失败");
        assert_eq!(
            cx.update(|cx| app_entity.read(cx).active_session()),
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
        let (handle, app_entity) = open_app(cx, history, None);
        app_entity.update(cx, |app, _cx| {
            inject_test_workspace(app, "envelope");
        });
        let ws_path =
            cx.update(|cx| app_entity.read(cx).workspace.as_ref().unwrap().path.clone());

        // 同 root 两个会话 (pi-1/pi-2); 后建者为 active (create 置 active)
        let (id_a, shared_a) = test_sessions::create_session(cx, &app_entity, &ws_path);
        let (id_b, shared_b) = test_sessions::create_session(cx, &app_entity, &ws_path);
        assert_ne!(id_a, id_b);
        assert_eq!(
            cx.update(|cx| app_entity.read(cx).active_session()),
            Some(id_b)
        );
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

    // ===== 4.1: 资产注册接线（现走总线 → 镜像）=====

    /// 打开工作区 → 项目全量注册落树 (commit_open 钩子)。
    #[gpui_kit::test]
    fn workspace_open_registers_projects_into_manager(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app(cx, history, None);
        let ws = temp_dir("reg-open");
        let project_dir = ws.join("foo");
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("lib.rs"), "x").unwrap();
        project_model::create_workspace_files(&ws).unwrap();
        fs::write(
            project_model::files::solutions_path(&ws),
            format!(
                r#"[{{"type": "simple", "path": "{}"}}]"#,
                project_dir.display()
            ),
        )
        .unwrap();

        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        wait_asset_settled(
            cx,
            &app_entity,
            |state| matches!(state, AssetState::Forest { .. }),
            "workspace 打开后注册落树",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            let view = asset_state(app);
            let AssetState::Forest { nodes, .. } = &view else {
                panic!("应注册落树, got {view:?}")
            };
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0].id, project_id(&project_dir));
            assert_eq!(nodes[0].kind, AssetKind::Project);
        });
    }

    // ===== change solution-service 2.3: 三个 load 边界的错误契约 =====

    /// 启动边界 (spec: Solution load failures): corrupt solutions
    /// → open_failure + 工作区不打开。
    #[gpui_kit::test]
    fn startup_load_failure_shows_error_and_no_workspace(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let ws = temp_dir("load-fail-startup");
        fs::create_dir_all(&ws).unwrap();
        project_model::create_workspace_files(&ws).unwrap();
        fs::write(project_model::files::solutions_path(&ws), "{oops").unwrap();
        {
            let mut h = crate::workspace::history::History::load_at(&history);
            h.push(ws.clone());
            h.save_at(&history).unwrap();
        }
        let (_handle, app_entity) = open_app(cx, history, Some(ws));
        wait_startup_settled(
            cx,
            &app_entity,
            |app| app.open_failure.is_some(),
            "启动 restore 的 load 失败应记录 open_failure",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert!(
                app.open_failure
                    .as_ref()
                    .is_some_and(|m| m.contains("solution configuration invalid")),
                "失败文案点名解析失败: {:?}",
                app.open_failure
            );
            assert!(app.workspace.is_none(), "load 失败不得打开工作区");
        });
    }

    /// 手动切换边界 (spec): unknown SolutionType → open_failure + 不打开。
    #[gpui_kit::test]
    fn switch_load_failure_shows_error_and_no_workspace(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app(cx, history, None);
        let ws = temp_dir("load-fail-switch");
        fs::create_dir_all(&ws).unwrap();
        project_model::create_workspace_files(&ws).unwrap();
        fs::write(
            project_model::files::solutions_path(&ws),
            r#"[{"type": "nope", "path": "/x"}]"#,
        )
        .unwrap();
        // 切换流的 picker 确认后走同一入口 (begin_open) — 测试直驱该入口
        app_entity.update(cx, |app, cx| app.begin_open(ws, cx));
        wait_startup_settled(
            cx,
            &app_entity,
            |app| app.open_failure.is_some(),
            "切换 load 失败应记录 open_failure",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert!(
                app.open_failure
                    .as_ref()
                    .is_some_and(|m| m.contains("unknown solution type")),
                "失败文案点名未知类型: {:?}",
                app.open_failure
            );
            assert!(app.workspace.is_none(), "失败不打开工作区");
        });
    }

    /// 显式 reload 边界 (spec): 一项合法 + 一项死 path → MissingPath,
    /// 失败绝不缓存部分树 — 旧工作区原样保留 (不 2 不 0)。
    #[gpui_kit::test]
    fn reload_failure_keeps_previous_workspace_without_partial(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app(cx, history, None);
        let ws = temp_dir("load-fail-reload");
        let foo = ws.join("foo");
        fs::create_dir_all(&foo).unwrap();
        project_model::create_workspace_files(&ws).unwrap();
        fs::write(
            project_model::files::solutions_path(&ws),
            format!(r#"[{{"type": "simple", "path": "{}"}}]"#, foo.display()),
        )
        .unwrap();
        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        wait_startup_settled(
            cx,
            &app_entity,
            |app| app.workspace.is_some(),
            "首次打开落定",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert_eq!(
                app.service_layer.tree_view().unwrap().tree().projects.len(),
                1
            );
        });

        // 显式 reload: solutions 改为 [合法 foo, 死 path] → MissingPath
        let dead = ws.join("nope-never");
        fs::write(
            project_model::files::solutions_path(&ws),
            format!(
                r#"[{{"type": "simple", "path": "{}"}}, {{"type": "simple", "path": "{}"}}]"#,
                foo.display(),
                dead.display()
            ),
        )
        .unwrap();
        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        wait_startup_settled(
            cx,
            &app_entity,
            |app| app.open_failure.is_some(),
            "reload 失败应记录 open_failure",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert!(
                app.open_failure
                    .as_ref()
                    .is_some_and(|m| m.contains("solution path does not exist")),
                "失败文案点名死 path: {:?}",
                app.open_failure
            );
            app.workspace
                .as_ref()
                .expect("旧工作区保留 (失败不缓存部分树)");
            let t = app.service_layer.tree_view().expect("旧镜像保留");
            assert_eq!(
                t.tree().projects.len(),
                1,
                "旧状态原样保留 — 无部分树 (不 2 不 0)"
            );
        });
    }

    /// add-solution → 增量注册, 新项目行无重启出现 (spec: register
    /// incrementally … without restarting)。
    #[gpui_kit::test]
    fn add_solution_registers_project_incrementally(cx: &mut TestAppContext) {
        let history = fresh_history_path();
        let (_handle, app_entity) = open_app(cx, history, None);
        let ws = temp_dir("reg-add");
        project_model::create_workspace_files(&ws).unwrap();
        app_entity.update(cx, |app, cx| app.begin_open(ws.clone(), cx));
        cx.run_until_parked();
        // 零 solution → 空视图
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert_eq!(asset_state(app), AssetState::Empty);
        });

        let pdir = ws.join("bar");
        fs::create_dir_all(&pdir).unwrap();
        fs::write(pdir.join("main.rs"), "x").unwrap();
        // 5.2: create 走 ask (picker 之后的发送点), 回执/树变更经泵消费
        app_entity.update(cx, |app, cx| app.request_create_solution(pdir.clone(), cx));
        wait_sol_settled(
            cx,
            &app_entity,
            |app| {
                app.service_layer
                    .tree_view()
                    .is_some_and(|t| t.tree().projects.iter().any(|p| p.root == pdir))
            },
            "create 落定",
        );
        // 注册与枚举走总线 → 泵到镜像真的落树 (旧 manager 是同步的)
        wait_asset_settled(
            cx,
            &app_entity,
            |state| matches!(state, AssetState::Forest { .. }),
            "增量注册后落树",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            let view = asset_state(app);
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
        let (_handle, app_entity) = open_app(cx, history, None);
        let ws1 = temp_dir("reg-switch-1");
        let one = ws1.join("one");
        fs::create_dir_all(&one).unwrap();
        project_model::create_workspace_files(&ws1).unwrap();
        fs::write(
            project_model::files::solutions_path(&ws1),
            format!(r#"[{{"type": "simple", "path": "{}"}}]"#, one.display()),
        )
        .unwrap();
        let ws2 = temp_dir("reg-switch-2");
        let two = ws2.join("two");
        fs::create_dir_all(&two).unwrap();
        project_model::create_workspace_files(&ws2).unwrap();
        fs::write(
            project_model::files::solutions_path(&ws2),
            format!(r#"[{{"type": "simple", "path": "{}"}}]"#, two.display()),
        )
        .unwrap();

        app_entity.update(cx, |app, cx| app.begin_open(ws1, cx));
        wait_asset_settled(
            cx,
            &app_entity,
            |state| matches!(state, AssetState::Forest { nodes } if nodes.iter().any(|n| n.id == project_id(&one))),
            "ws1 落树",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            let AssetState::Forest { nodes, .. } = asset_state(app) else {
                panic!("ws1 应落树")
            };
            assert_eq!(nodes[0].id, project_id(&one));
        });

        app_entity.update(cx, |app, cx| app.begin_open(ws2.clone(), cx));
        // 等「只剩 ws2 的注册」—— 仅等 Forest 会被 ws1 的旧树满足
        wait_asset_settled(
            cx,
            &app_entity,
            |state| matches!(state, AssetState::Forest { nodes }
                if nodes.len() == 1 && nodes[0].id == project_id(&two)),
            "ws2 替换落树",
        );
        cx.update(|cx| {
            let app = app_entity.read(cx);
            assert_eq!(app.workspace.as_ref().unwrap().path, ws2);
            let AssetState::Forest { nodes, .. } = asset_state(app) else {
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
