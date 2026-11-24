//! 右 dock 资产树面板 `AssetOverview`（原 `AssetView`；change asset-service 5.1 迁入本 crate。design 归属:
//! asset-manager-right-dock D7，包归并: change asset-package-merge）。
//!
//! part2 只做**显示**：读服务推来的视图镜像、把中立 DTO
//! 适配成 gpui 元素、渲染与分发；⟳ 经 app 请求重枚举，左键打开意图与
//! 右键菜单描述/执行由**镜像视图派生**（change asset-service 4.4）。
//! 数据语义（注册、异步枚举、过滤、空/降级、O(1) 与在途守卫）全部归
//! `crates/asset-service` 的服务端——本模块零文件语义，
//! 是旧 `workspace_property_source` 显示半边的继任者。
//!
//! chrome（design D9 / dock-layout spec 修订）：header = 标题 + ⟳ 刷新 +
//! 只读规则摘要 + `×`；body = 空态 / 降级态 / gpui Tree。
//!
//! **焦点纪律（spec: 键盘转发）**：v1 面板内无可聚焦文本输入——根 div
//! 的 `on_click` 冒泡覆盖全部点击（行/按钮），把键盘/IME 焦点交回终端
//! 面板，键盘唯一归宿仍是 active session。
//!
//! **展开态面板自持（design D4）**：`expanded` 集合由面板维护（订阅
//! `TreeEvent`），构造 `TreeItem` 时贴回——刷新与过滤重建都不丢展开；
//! 同步时按新树裁剪陈旧 id。

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::tree::{TreeEvent, TreeItem, TreeState, tree};
use gpui_kit::*;

use std::path::PathBuf;
use std::sync::Arc;

use crate::contract::{
    ACTION_COPY_PATH, ACTION_REVEAL, AssetKind, AssetNode, AssetState, TreeSnapshot,
};
use contracts::shared::ActionDesc;


/// 面板 → GUI 宿主的**窄缝**（change asset-service 5.1；形态对齐
/// `agent_service::TerminalBus`：*面板只表达语义，不触 `TerminalApp` 类型*）。
///
/// 面板因此可以住在库 crate 里 —— 宿主实现由 `src/shell` 提供，持
/// `WeakEntity<TerminalApp>`。
pub trait AssetHost: Send + Sync + 'static {
    /// 右 dock 是否开着 —— 渲染闸门依据（spec: Tree panel skips work
    /// while not rendered）。
    fn right_dock_open(&self, cx: &App) -> bool;

    /// 资产视图镜像（服务经 `asset.tree_changed` 推来的纯数据树）。
    fn asset_view(&self, cx: &App) -> Option<TreeSnapshot>;

    /// 默认打开分发（认领 → 中心签；spec: Left-click open hook）。
    fn open_object(&self, path: PathBuf, kind: AssetKind, window: &mut Window, cx: &mut App);

    /// 把键盘焦点交回当前中心面板（spec: Row clicks SHALL NOT capture
    /// keyboard focus —— 点击后归宿是 active tab 的面板）。
    fn focus_active_center_panel(&self, window: &mut Window, cx: &mut App);

    /// ⟳ 重枚举（supersede 在途 + 重读 filter 源）。
    fn refresh_assets(&self, cx: &mut App);

    /// 关闭右 dock。
    fn close_right_dock(&self, window: &mut Window, cx: &mut App);
}

pub struct AssetOverview {
    /// 宿主窄缝（design D1 / task 5.1）：dock 闸门、镜像读取、打开分发、
    /// 焦点归还、⟳ 与关坞全部经它 —— 面板不触 `TerminalApp` 类型。
    host: Arc<dyn AssetHost>,
    /// `Focusable` 契约要求；dock 交互时会聚焦此 handle（spike E2 实测）。
    focus: FocusHandle,
    /// gpui 树状态（虚拟滚动 + 展开/选中由库管；数据由本面板推入）。
    tree_state: Entity<TreeState>,
    /// 展开态集合（design D4：UI 状态不跨服务边界）。
    ///
    /// 下列观测字段（expanded/model_collapsed/dir_ids/last_*/view_pulls）
    /// 一律 `pub`：面板已是库类型，而集成测试在 bin 侧（task 5.5 修订）。
    pub expanded: HashSet<String>,
    /// 模型行 (Project/Module) 的**折叠**集 — 反转语义: 默认展开
    /// (内容可见优先), 用户折叠后由 `TreeEvent::Collapsed` 记入;
    /// 普通目录行仍走 `expanded` 正向集合 (默认折叠)。
    pub model_collapsed: HashSet<String>,
    /// 全部文件夹行 id（Project/Module/Dir — 图标/chevron 兜底）。
    /// Rc 包裹: render 每帧只需引用计数 +1 — 不做整集合克隆
    /// (spec: Keystrokes do not pay tree cost)。
    pub dir_ids: Rc<HashSet<String>>,
    /// 最近一次同步进 `tree_state` 的视图（render 读缓存，避免每帧克隆）。
    pub last_synced: Option<AssetState>,
    /// 最近同步的视图代数 (design D5): 代数相等比对替代整树深比较 —
    /// 代数未变的通知零工作 (spec: Data-change notification is O(1))。
    pub last_generation: Option<u64>,
    /// 观测面：真正推过 `TreeState` 的次数（= 旧 `AssetManager::
    /// view_pulls`）—— 代数相等的通知不推进它。
    ///
    /// **不加 `cfg(test)`**：面板已入库 crate，而 `panel_tests` 是 bin 侧
    /// 集成测试 —— bin 依赖的 crate 不带 `cfg(test)`，测试缝必须常驻
    /// （change asset-service 5.5 修订）。仅一个加法点，生产开销可忽略。
    pub view_pulls: usize,
    /// `TreeEvent` 订阅（drop 即退订 → 必须存住）。
    _tree_events: Subscription,
}

impl AssetOverview {
    /// 构造面板。
    ///
    /// 输入: `app` (宿主: 右 dock 可见性查询、打开分发、**镜像读取**)、
    /// `cx`。输出: 就绪待挂载的面板实体 (TreeState 订阅已建)。
    pub fn new(host: Arc<dyn AssetHost>, cx: &mut Context<Self>) -> Self {
        let tree_state = cx.new(|cx| TreeState::new(cx));
        let tree_events = cx.subscribe(&tree_state, |this, _state, event: &TreeEvent, cx| {
            match event {
                TreeEvent::Expanded(id) => {
                    this.expanded.insert(id.to_string());
                    this.model_collapsed.remove(id.as_str());
                }
                TreeEvent::Collapsed(id) => {
                    this.expanded.remove(id.as_str());
                    // 模型行按折叠集判定 (目录行忽略此集 — 双写无害)
                    this.model_collapsed.insert(id.to_string());
                }
            }
            let _ = cx;
        });
        Self {
            host,
            focus: cx.focus_handle(),
            tree_state,
            expanded: HashSet::new(),
            model_collapsed: HashSet::new(),
            dir_ids: Rc::new(HashSet::new()),
            last_synced: None,
            last_generation: None,
            view_pulls: 0,
            _tree_events: tree_events,
        }
    }

    /// 数据变化入口（app 的 `notify_panels` 转发名单对树面板调用）：
    /// 0. **可见性闸门**（design D7; spec: Tree panel skips work while not
    ///    rendered）：右 dock 关闭 → 面板未渲染 → 触碰 manager 之前早退；
    /// 1. **代数比对**（design D5）：代数未变 → 零工作（不拉视图、无整树
    ///    深比较）；变化才 `state()` → 推入 `TreeState` → 重渲。
    ///    数据变更通知**不触碰在途枚举** —— 注册/⟳ 是唯一失效源（spec:
    ///    In-flight enumeration is not restarted by notifications）。
    pub fn on_data_changed(&mut self, cx: &mut Context<Self>) {
        if !self.right_dock_open(cx) {
            return; // 隐藏期间的通知零面板工作 (spec: Hidden panel costs nothing)
        }
        self.sync_if_generation_changed(cx);
        cx.notify();
    }

    /// 右 dock 是否开着（闸门依据）：dock 关闭 = 面板未渲染；
    /// app 实体已释放也按未渲染处理。
    fn right_dock_open(&self, cx: &App) -> bool {
        self.host.right_dock_open(cx)
    }

    /// 代数同步（`on_data_changed` 与 render 惰性检查共用）：
    /// 代数变化才拉视图 → 推入 `TreeState`；相等即早退（不调 `state()`）。
    fn sync_if_generation_changed(&mut self, cx: &mut Context<Self>) {
        let Some(snap) = self.mirror(cx) else {
            return; // 尚无镜像 (服务未推过) — 等首推
        };
        let generation = snap.generation;
        if self.last_generation == Some(generation) {
            return; // O(1): 代数相等 → 不拉视图 (spec)
        }
        let view = snap.state.clone();
        self.view_pulls += 1;
        self.sync_tree(&view, cx);
        self.last_synced = Some(view);
        self.last_generation = Some(generation);
    }

    /// 读服务推来的**纯数据视图镜像**（`asset.tree_changed`；design D2）。
    /// 无镜像按空态处理 — 面板只显示，不造数据。
    fn mirror(&self, cx: &App) -> Option<TreeSnapshot> {
        self.host.asset_view(cx)
    }

    /// 当前视图（无镜像 = 空态）。
    fn view(&self, cx: &App) -> AssetState {
        self.mirror(cx)
            .map(|s| s.state)
            .unwrap_or(AssetState::Empty)
    }

    /// DTO → `TreeItem` 推入（适配器核心）：
    /// - 注册的 Project 行即根节点（spec: registered projects as root
    ///   nodes），Module/文件递归其下；
    /// - 展开态从 `self.expanded` 贴回，随后按新树裁剪陈旧 id；
    /// - `dir_ids` 收集供图标兜底。
    fn sync_tree(&mut self, view: &AssetState, cx: &mut Context<Self>) {
        let (items, dirs, all_ids) = match view {
            AssetState::Forest { nodes, .. } => {
                let mut dirs = HashSet::new();
                let mut all_ids = HashSet::new();
                for node in nodes {
                    let (d, a) = collect_ids(node);
                    dirs.extend(d);
                    all_ids.extend(a);
                }
                let items: Vec<TreeItem> = nodes
                    .iter()
                    .map(|child| to_tree_item(child, &self.expanded, &self.model_collapsed))
                    .collect();
                (items, dirs, all_ids)
            }
            _ => (Vec::new(), HashSet::new(), HashSet::new()),
        };
        self.expanded.retain(|id| all_ids.contains(id)); // 重建后裁剪
        self.model_collapsed.retain(|id| all_ids.contains(id));
        self.dir_ids = Rc::new(dirs);
        self.tree_state
            .update(cx, |state, cx| state.set_items(items, cx));
    }

    /// 左键: 从镜像派生打开意图, 再分发到默认打开后端
    /// (spec: Left-click open hook — 只读视图, 无服务往返)。
    fn open_asset(&mut self, item_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.view(cx);
        if let Some((path, kind)) = open_target(&view, item_id) {
            self.host.open_object(path, kind, window, cx);
        }
    }

    /// 右键菜单描述 —— **完全由服务声明决定**（task 6.1）：读镜像里
    /// 该节点的 `actions`（服务拥有标签），视图不自带任何文案或规则。
    fn asset_menu(&self, item_id: &str, cx: &App) -> Vec<ActionDesc> {
        menu_for(&self.view(cx), item_id)
    }

    /// 菜单执行 → 效果意图 (面板在 `apply_effects` 落地)。
    fn asset_execute(&self, item_id: &str, func_id: &str, cx: &App) -> Vec<AssetEffect> {
        effects_for(&self.view(cx), item_id, func_id)
    }
}

/// 按 id 找节点 —— 取 `node.actions` 的入口（菜单与执行都读它）。
fn find_node<'a>(view: &'a AssetState, id: &str) -> Option<&'a AssetNode> {
    fn walk<'a>(node: &'a AssetNode, id: &str) -> Option<&'a AssetNode> {
        if node.id == id {
            return Some(node);
        }
        node.children.iter().find_map(|c| walk(c, id))
    }
    match view {
        AssetState::Forest { nodes, .. } => nodes.iter().find_map(|n| walk(n, id)),
        _ => None,
    }
}

/// 打开目标：仅 File 行产出（目录/模块行由面板切展开）——
/// 纯视图函数，无服务往返（`AssetManager::open_object_for` 的继任者）。
/// 返回 `(path, kind)` 而非 `OpenObject`：该类型属 GUI 侧（中心签域），
/// 面板不触它，由 [`AssetHost::open_object`] 的实现方组装。
fn open_target(view: &AssetState, item_id: &str) -> Option<(PathBuf, AssetKind)> {
    match find_kind(view, item_id)? {
        AssetKind::File => Some((PathBuf::from(item_id), AssetKind::File)),
        _ => None,
    }
}

/// 右键菜单项 —— **完全由服务声明决定**（task 6.1/6.3）：节点没声明
/// 动作（Project 行、未知 id）⇒ 空 ⇒ 无菜单。标签随 `ActionDesc` 来自
/// 服务，视图不再自带任何文案。
fn menu_for(view: &AssetState, item_id: &str) -> Vec<ActionDesc> {
    find_node(view, item_id)
        .map(|node| node.actions.to_vec())
        .unwrap_or_default()
}

/// 菜单功能 → 效果意图（task 6.2：**视图执行**，不发 invoke）。
///
/// 前置是"服务确实为该节点声明了这个动作" —— 未声明 / 节点不在树 → no-op。
/// 两条都是 `ExecutedBy::View`：能力在视图（剪贴板、OS 集成），服务只
/// 声明"有这个功能"（design D5: 执行归属跟着状态走）。
fn effects_for(view: &AssetState, item_id: &str, func_id: &str) -> Vec<AssetEffect> {
    let Some(node) = find_node(view, item_id) else {
        return Vec::new();
    };
    if !node.actions.iter().any(|a| a.id == func_id) {
        return Vec::new(); // 服务未声明 ⇒ 不执行
    }
    match func_id {
        ACTION_COPY_PATH => vec![AssetEffect::CopyText(item_id.to_string())],
        ACTION_REVEAL => vec![AssetEffect::RevealPath(std::path::PathBuf::from(item_id))],
        _ => Vec::new(),
    }
}

/// 效果落地（task 6.2：**面板内执行**）—— 剪贴板要窗口上下文、reveal
/// 要系统命令，两者本就是 view 层能力（design D5）。服务声明动作、
/// 视图执行 ⇒ 全程不产生 invoke 消息。
fn apply_effects(effects: Vec<AssetEffect>, _window: &mut Window, cx: &mut App) {
    for effect in effects {
        match effect {
            AssetEffect::CopyText(text) => {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
            }
            AssetEffect::RevealPath(path) => reveal_in_finder(&path),
        }
    }
}

/// 在系统文件管理器中定位该项（spec v1: Finder, macOS）。
/// Windows 的 reveal 记 pending-decisions（本轮不做）。
#[cfg(target_os = "macos")]
fn reveal_in_finder(path: &std::path::Path) {
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
}

#[cfg(not(target_os = "macos"))]
fn reveal_in_finder(_path: &std::path::Path) {
    // 非 macOS: v1 无 reveal 目标 (spec 场景即 Finder)
}

/// 行的展开判定: 模型行 (Project/Module) 默认展开、按折叠集反转;
/// 普通目录行正向走 `expanded` (默认折叠)。
fn row_expanded(node: &AssetNode, expanded: &HashSet<String>, collapsed: &HashSet<String>) -> bool {
    match node.kind {
        AssetKind::Project | AssetKind::Module => {
            !collapsed.contains(&node.id)
        }
        _ => expanded.contains(&node.id),
    }
}

/// 递归构造 `TreeItem`（children 全量给出，可见性由展开判定控制——
/// 展开时库直接从已有 children 拉平，无需重新推数）。
fn to_tree_item(
    node: &AssetNode,
    expanded: &HashSet<String>,
    collapsed: &HashSet<String>,
) -> TreeItem {
    let mut item = TreeItem::new(node.id.clone(), node.label.clone())
        .expanded(row_expanded(node, expanded, collapsed));
    if node.kind.is_folder() && !node.children.is_empty() {
        item = item.children(
            node.children
                .iter()
                .map(|c| to_tree_item(c, expanded, collapsed)),
        );
    }
    item
}

/// 收集 (文件夹 id 集合, 全部 id 集合)。
fn collect_ids(node: &AssetNode) -> (HashSet<String>, HashSet<String>) {
    let mut dirs = HashSet::new();
    let mut all = HashSet::new();
    fn walk(node: &AssetNode, dirs: &mut HashSet<String>, all: &mut HashSet<String>) {
        all.insert(node.id.clone());
        if node.kind.is_folder() {
            dirs.insert(node.id.clone());
        }
        for child in &node.children {
            walk(child, dirs, all);
        }
    }
    walk(node, &mut dirs, &mut all);
    (dirs, all)
}

impl BasePanel for AssetOverview {
    fn panel_name(&self) -> &'static str {
        // task 5.2: 与能力名 `asset-overview` 对齐 (原 "file-tree")
        "asset-overview"
    }
    fn closable(&self, _: &App) -> bool {
        false // 关的是 dock 不是 panel（ClosePanel 会移除面板, 不用）
    }
    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for AssetOverview {
    fn title_bar(&self, _: &App) -> bool {
        false // D8: 自带 header chrome
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for AssetOverview {}

impl Focusable for AssetOverview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AssetOverview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 惰性代数检查 (design D7; spec: Reopening the dock shows the
        // current tree): 隐藏期间错过的失效在渲染路径补同步 — 重开 dock
        // 即显示最后失效时的内容, 不因曾隐藏而重枚举。
        self.sync_if_generation_changed(cx);
        let host_refocus = self.host.clone();
        let title = "文件树";
        // 语义 token 快照 (Hsla: Copy) — 闭包捕获值 (design D3)
        let theme_muted = cx.theme().muted_foreground;
        let theme_danger = cx.theme().danger;
        let theme_border = cx.theme().border;
        let theme_accent = cx.theme().accent;
        // 规则摘要位已撤除 (D9: 规则源 = 项目根
        // .gitignore, 头部无数据源 — dock-layout delta 同步收窄)
        // render 读同步缓存（on_data_changed 已保证先于 notify 同步）
        let body = match &self.last_synced {
            None | Some(AssetState::Empty) => div()
                .id("tree-empty")
                .test_support()
                .flex_1()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .text_sm()
                .text_color(theme_muted)
                .child("no projects")
                .child("添加 Solution 后此处显示项目文件")
                .into_any_element(),
            Some(AssetState::Degraded { message }) => div()
                .id("tree-degraded")
                .test_support()
                .flex_1()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .text_sm()
                .text_color(theme_danger)
                .child("无法枚举资产")
                .child(message.clone())
                .into_any_element(),
            Some(AssetState::Forest { .. }) => {
                let dir_ids = self.dir_ids.clone();
                let panel_weak = cx.entity().downgrade();
                let menu_weak = panel_weak.clone(); // tree 的 move 闭包会吃掉 panel_weak
                div()
                    .id("tree-body")
                    .test_support()
                    .flex_1()
                    .min_h(px(0.)) // flex 子项收缩 (pane 尺寸教训)
                    .w_full()
                    .child(tree(&self.tree_state, move |_ix, entry, _selected, _w, _cx| {
                        let id = entry.item().id.clone();
                        let is_dir = dir_ids.contains(id.as_str());
                        let marker = match (is_dir, entry.is_expanded()) {
                            (true, true) => "▾ ",
                            (true, false) => "▸ ",
                            (false, _) => "  ",
                        };
                        // 行内 cell 自注册观测锚点 (find/click 用); 缩进归内容
                        // 左键: 内容 on_click 分发打开意图 — 库保证内容
                        // handler **先于**其内置"选中+切换"执行 (共存各恰一次),
                        // 焦点交回由根级 on_click 冒泡统一处理 (spec: 键盘转发)
                        ListItem::new(format!("row-{id}"))
                            .on_click({
                                let panel_weak = panel_weak.clone();
                                let open_id = id.clone();
                                move |_, window, cx| {
                                    panel_weak
                                        .update(cx, |panel, cx| {
                                            panel.open_asset(&open_id, window, cx)
                                        })
                                        .ok();
                                }
                            })
                            .child(
                                div()
                                    .id(format!("cell-{id}"))
                                    .test_support()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .whitespace_nowrap()
                                    .pl(px(6. + entry.depth() as f32 * 14.))
                                    .text_sm()
                                    .child(format!("{marker}{}", entry.item().label)),
                            )
                    })
                    .context_menu({
                        // 右键: manager 描述功能 → PopupMenu; 项点击 → manager
                        // 执行 → 面板落地效果 (spec: Right-click context menu)
                        let panel_weak = menu_weak.clone();
                        move |_ix, entry, menu, _window, cx| {
                            let item_id = entry.item().id.clone();
                            let descs = match panel_weak
                                .update(cx, |panel, cx| panel.asset_menu(&item_id, cx))
                            {
                                Ok(descs) => descs,
                                Err(_) => return menu,
                            };
                            let mut menu = menu;
                            for desc in descs {
                                let item_id = item_id.clone();
                                let func_id = desc.id.clone();
                                let pw = panel_weak.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(desc.label).on_click(
                                        move |_, window, cx| {
                                            let effects = pw
                                                .update(cx, |panel, cx| {
                                                    panel.asset_execute(&item_id, &func_id, cx)
                                                })
                                                .unwrap_or_default();
                                            apply_effects(effects, window, cx);
                                        },
                                    ),
                                );
                            }
                            menu
                        }
                    })
                    )

                    .into_any_element()
            }
        };

        div()
            .id("tree-root")
            .test_support() // 观测锚点 (无 test-support 特性时为 no-op)
            .size_full()
            .flex()
            .flex_col()
            // 根级点击冒泡: 行 / ⟳ / × 全部经此把键盘交回 **active tab 的
            // 面板** (design D6-3: console active 时即终端; viewer active
            // 时归 viewer; × 自身的 close_right_dock 也会聚焦, 幂等)
            .on_click(cx.listener(move |_, _event: &ClickEvent, window, cx| {
                let host = host_refocus.clone();
                host.focus_active_center_panel(window, cx);
            }))
            .child(
                div()
                    .id("tree-header")
                    .test_support()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(theme_border)
                    .child(div().id("tree-title").test_support().text_sm().child(title))
                    .child(
                        div()
                            .id("tree-actions")
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("tree-refresh")
                                    .test_support()
                                    .cursor_pointer()
                                    .px_1()
                                    .text_sm()
                                    .hover(|style| style.text_color(theme_accent))
                                    .child("⟳")
                                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                                        // ⟳ → app → manager 重枚举 (入口代数已推进,
                                        // 同步立即生效; 异步落树经 app 转发通知)
                                        this.host.refresh_assets(cx);
                                        this.on_data_changed(cx);
                                    })),
                            )
                            .child(
                                div()
                                    .id("tree-close")
                                    .test_support()
                                    .cursor_pointer()
                                    .px_1()
                                    .text_sm()
                                    .hover(|style| style.text_color(theme_danger))
                                    .child("×")
                                    .on_click(cx.listener(|this, _event: &ClickEvent, window, cx| {
                                        this.host.close_right_dock(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(body)
    }
}

// ------------------------------------------------------------------------------------
// 面板侧的动作/效果 DTO（自 `src/assets/state.rs` 迁入 — task 5.1）
// ------------------------------------------------------------------------------------

/// 菜单功能的**效果意图**, 由面板落地（change asset-service 4.4/6.2）:
/// 剪贴板要窗口上下文、reveal 要系统命令 —— 这些本就是 view 层能力
/// （design D5: 执行归属跟着状态走）。
#[derive(Clone, PartialEq, Debug)]
pub enum AssetEffect {
    CopyText(String),
    RevealPath(PathBuf),
}

/// 在视图中按 id 找资产种类 (打开钩子的 kind 依据; 纯遍历)。
pub fn find_kind(view: &AssetState, id: &str) -> Option<AssetKind> {
    fn walk(node: &AssetNode, id: &str) -> Option<AssetKind> {
        if node.id == id {
            return Some(node.kind);
        }
        node.children.iter().find_map(|c| walk(c, id))
    }
    match view {
        AssetState::Forest { nodes, .. } => nodes.iter().find_map(|n| walk(n, id)),
        _ => None,
    }
}
