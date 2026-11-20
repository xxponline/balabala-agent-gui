//! 右 dock 资产树面板 `AssetView`（原 `AssetView`；design 归属:
//! asset-manager-right-dock D7，包归并: change asset-package-merge）。
//!
//! part2 只做**显示**：拉 `AssetManager` 的 `AssetState`、把中立 DTO
//! 适配成 gpui 元素、渲染与分发；⟳ 经 app 请求重枚举，左键打开意图与
//! 右键菜单描述/执行全部来自 manager。数据语义（注册、异步枚举、过滤、
//! 空/降级、O(1) 与在途守卫）归同包 `manager`——本模块零文件语义，
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
use gpui_kit::component::dock::{BasePanel, DockPlacement, Panel, PanelEvent};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::tree::{TreeEvent, TreeItem, TreeState, tree};
use gpui_kit::*;

use crate::app::TerminalApp;
use super::state::{AssetEffect, AssetKind, AssetMenu, AssetNode, AssetState};

pub struct AssetView {
    app: WeakEntity<TerminalApp>,
    /// `Focusable` 契约要求；dock 交互时会聚焦此 handle（spike E2 实测）。
    focus: FocusHandle,
    /// gpui 树状态（虚拟滚动 + 展开/选中由库管；数据由本面板推入）。
    tree_state: Entity<TreeState>,
    /// 展开态集合（design D4：UI 状态不跨 asset_manager 边界）。
    expanded: HashSet<String>,
    /// 模型行 (Project/Module) 的**折叠**集 — 反转语义: 默认展开
    /// (内容可见优先), 用户折叠后由 `TreeEvent::Collapsed` 记入;
    /// 普通目录行仍走 `expanded` 正向集合 (默认折叠)。
    model_collapsed: HashSet<String>,
    /// 全部文件夹行 id（Project/Module/Dir — 图标/chevron 兜底）。
    /// Rc 包裹: render 每帧只需引用计数 +1 — 不做整集合克隆
    /// (spec: Keystrokes do not pay tree cost)。
    dir_ids: Rc<HashSet<String>>,
    /// 最近一次同步进 `tree_state` 的视图（render 读缓存，避免每帧克隆）。
    last_synced: Option<AssetState>,
    /// 最近同步的视图代数 (design D5): 代数相等比对替代整树深比较 —
    /// 代数未变的通知零工作 (spec: Data-change notification is O(1))。
    last_generation: Option<u64>,
    /// `TreeEvent` 订阅（drop 即退订 → 必须存住）。
    _tree_events: Subscription,
}

impl AssetView {
    pub fn new(app: WeakEntity<TerminalApp>, cx: &mut Context<Self>) -> Self {
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
            app,
            focus: cx.focus_handle(),
            tree_state,
            expanded: HashSet::new(),
            model_collapsed: HashSet::new(),
            dir_ids: Rc::new(HashSet::new()),
            last_synced: None,
            last_generation: None,
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
    pub(crate) fn on_data_changed(&mut self, cx: &mut Context<Self>) {
        if !self.right_dock_open(cx) {
            return; // 隐藏期间的通知零面板工作 (spec: Hidden panel costs nothing)
        }
        self.sync_if_generation_changed(cx);
        cx.notify();
    }

    /// 右 dock 是否开着（闸门依据）：dock 关闭 = 面板未渲染；
    /// app 实体已释放也按未渲染处理。
    fn right_dock_open(&self, cx: &App) -> bool {
        self.app.upgrade().is_some_and(|app| {
            app.read(cx)
                .dock_area
                .read(cx)
                .is_dock_open(DockPlacement::Right)
        })
    }

    /// 代数同步（`on_data_changed` 与 render 惰性检查共用）：
    /// 代数变化才拉视图 → 推入 `TreeState`；相等即早退（不调 `state()`）。
    fn sync_if_generation_changed(&mut self, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let generation = app.read(cx).asset_manager.read(cx).view_generation();
        if self.last_generation == Some(generation) {
            return; // O(1): 代数相等 → 不拉视图 (spec)
        }
        let view = app.read(cx).asset_manager.read(cx).state();
        self.sync_tree(&view, cx);
        self.last_synced = Some(view);
        self.last_generation = Some(generation);
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

    /// 左键: 经 manager 的打开意图分发到默认打开后端
    /// (spec: through the asset manager's default-open dispatch)。
    fn open_asset(&mut self, item_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let obj = app
            .read(cx)
            .asset_manager
            .read(cx)
            .open_object_for(item_id);
        if let Some(obj) = obj {
            app.update(cx, |app, cx| app.open_object(obj, window, cx));
        }
    }

    /// 右键菜单描述 (经 manager; spec: items provided by the asset manager)。
    fn asset_menu(&self, item_id: &str, cx: &App) -> Vec<AssetMenu> {
        self.app
            .upgrade()
            .map(|app| app.read(cx).asset_manager.read(cx).menu(item_id))
            .unwrap_or_default()
    }

    /// 菜单执行 → 效果意图 (经 manager; 面板在 `apply_effects` 落地)。
    fn asset_execute(&self, item_id: &str, func_id: &str, cx: &App) -> Vec<AssetEffect> {
        self.app
            .upgrade()
            .map(|app| app.read(cx).asset_manager.read(cx).execute(item_id, func_id))
            .unwrap_or_default()
    }
}

/// 效果落地（design D2）：manager 返回**意图**，面板执行——剪贴板要窗口
/// 上下文、reveal 要系统命令，manager 保持可单测、可跨边界。
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

impl BasePanel for AssetView {
    fn panel_name(&self) -> &'static str {
        "file-tree"
    }
    fn closable(&self, _: &App) -> bool {
        false // 关的是 dock 不是 panel（ClosePanel 会移除面板, 不用）
    }
    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for AssetView {
    fn title_bar(&self, _: &App) -> bool {
        false // D8: 自带 header chrome
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for AssetView {}

impl Focusable for AssetView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AssetView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 惰性代数检查 (design D7; spec: Reopening the dock shows the
        // current tree): 隐藏期间错过的失效在渲染路径补同步 — 重开 dock
        // 即显示最后失效时的内容, 不因曾隐藏而重枚举。
        self.sync_if_generation_changed(cx);
        let app_refocus = self.app.clone();
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
                let app = app_refocus.clone();
                app.update(cx, |app, cx| app.focus_active_center_panel(window, cx))
                    .ok();
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
                                        let app = this.app.clone();
                                        app.update(cx, |app, cx| app.refresh_assets(cx))
                                            .ok();
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
                                        let app = this.app.clone();
                                        app.update(cx, |app, cx| app.close_right_dock(window, cx))
                                            .ok();
                                    })),
                            ),
                    ),
            )
            .child(body)
    }
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
