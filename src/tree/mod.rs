//! 右 dock 资产树面板（change: asset-manager-right-dock, design D7）。
//!
//! part2 只做**显示**：拉 `asset_manager` 的 `AssetView`、把中立 DTO
//! 适配成 gpui 元素、渲染与分发；⟳ 经 app 请求重枚举，左键打开意图与
//! 右键菜单描述/执行全部来自 manager。数据语义（注册、异步枚举、过滤、
//! 空/降级、O(1) 与在途守卫）归 `asset_manager`——本模块零文件语义，
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
use crate::asset_manager::{AssetEffect, AssetMenu, AssetNode, AssetView};

pub struct TreePanel {
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
    last_synced: Option<AssetView>,
    /// 最近同步的视图代数 (design D5): 代数相等比对替代整树深比较 —
    /// 代数未变的通知零工作 (spec: Data-change notification is O(1))。
    last_generation: Option<u64>,
    /// `TreeEvent` 订阅（drop 即退订 → 必须存住）。
    _tree_events: Subscription,
}

impl TreePanel {
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
    ///    深比较）；变化才 `view()` → 推入 `TreeState` → 重渲。
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
    /// 代数变化才拉视图 → 推入 `TreeState`；相等即早退（不调 `view()`）。
    fn sync_if_generation_changed(&mut self, cx: &mut Context<Self>) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let generation = app.read(cx).asset_manager.read(cx).view_generation();
        if self.last_generation == Some(generation) {
            return; // O(1): 代数相等 → 不拉视图 (spec)
        }
        let view = app.read(cx).asset_manager.read(cx).view();
        self.sync_tree(&view, cx);
        self.last_synced = Some(view);
        self.last_generation = Some(generation);
    }

    /// DTO → `TreeItem` 推入（适配器核心）：
    /// - 注册的 Project 行即根节点（spec: registered projects as root
    ///   nodes），Module/文件递归其下；
    /// - 展开态从 `self.expanded` 贴回，随后按新树裁剪陈旧 id；
    /// - `dir_ids` 收集供图标兜底。
    fn sync_tree(&mut self, view: &AssetView, cx: &mut Context<Self>) {
        let (items, dirs, all_ids) = match view {
            AssetView::Forest { nodes, .. } => {
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
        crate::asset_manager::AssetKind::Project | crate::asset_manager::AssetKind::Module => {
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

impl BasePanel for TreePanel {
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

impl Panel for TreePanel {
    fn title_bar(&self, _: &App) -> bool {
        false // D8: 自带 header chrome
    }
    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl EventEmitter<PanelEvent> for TreePanel {}

impl Focusable for TreePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TreePanel {
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
        // 规则摘要位已撤除 (file-filter-prune D9: 规则源 = 项目根
        // .gitignore, 头部无数据源 — dock-layout delta 同步收窄)
        // render 读同步缓存（on_data_changed 已保证先于 notify 同步）
        let body = match &self.last_synced {
            None | Some(AssetView::Empty) => div()
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
            Some(AssetView::Degraded { message }) => div()
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
            Some(AssetView::Forest { .. }) => {
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
mod tests {
    //! 面板 (纯显示) 的无头契约: 注册缝直连 asset_manager —
    //! 渲染 / 展开贴回 / 渲染缓存 / 隐藏零开销 / 交互分发。
    //!
    //! 显式导入, 不用 glob — 防 `gpui_kit::*` 把属性宏内层 `#[test]`
    //! 解析到 `gpui_kit::test` → 递归爆 (app.rs 同款教训)。

    use crate::app::TerminalApp;
    use crate::asset_manager::{AssetManager, AssetView};
    use crate::project_model::Project;
    use gpui_kit::test::{TestQueryExt, TestWindowExt};
    use gpui_kit::{
        AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
    };
    use std::path::{Path, PathBuf};

    /// 起完整 app（拿 weak 引用 + 真实右槽），返回 (窗口句柄, app 实体)。
    fn open_app(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<TerminalApp>) {
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
                |window, cx| cx.new(|cx| TerminalApp::new(window, cx).expect("app 启动")),
            )
            .expect("open test window")
        });
        // hermetic: 清空可能的恢复态 (注册制下树只认 manager)
        app.update(cx, |app, _| {
            app.workspace = None;
            app.active = None;
        });
        (handle, app)
    }

    /// manager 实体 (测试缝观测: 代数 / 枚举计数 / 拉取计数)。
    fn manager(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
    ) -> gpui_kit::Entity<AssetManager> {
        cx.update(|cx| app.read(cx).asset_manager.clone())
    }

    /// hermetic 临时目录 (进程 id 隔离, 先清后建)。
    fn temp_ws(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn render(handle: gpui_kit::AnyWindowHandle, cx: &mut TestAppContext) {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
        })
        .expect("update window");
        cx.run_until_parked();
    }

    /// 写 hermetic 规则 (项目根 `.gitignore` + `.git` 标记 —
    /// 过滤源 = 项目根 git 供给, file-filter-prune D9)。
    fn write_rules(ws: &Path, rules: &[&str]) {
        std::fs::create_dir_all(ws.join(".git")).unwrap();
        let body: String = rules.iter().map(|s| format!("{s}\n")).collect();
        std::fs::write(ws.join(".gitignore"), body).unwrap();
    }

    /// 注入一个以 `ws` 为根的注册项目 (真实 FileEnumerator 链)。
    /// 旧 `set_source` 测试缝的继任: 注册缝 (design D7 — 测试直连 manager)。
    fn register_ws(app: &gpui_kit::Entity<TerminalApp>, ws: &Path, cx: &mut TestAppContext) {
        app.update(cx, |app, cx| {
            let project = Project::from_root(ws.to_path_buf());
            app.asset_manager
                .update(cx, |m, cx| m.register(ws.to_path_buf(), vec![project], cx));
        });
    }

    /// 注册 + 规则 + 首次落地 (旧 setup_filetree 同位替换; 不再需要
    /// 活动会话 — spec: Tree scope follows registered projects)。
    fn setup_filetree(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
        ws: &Path,
        rules: Option<&[&str]>,
    ) {
        if let Some(r) = rules {
            write_rules(ws, r);
        }
        register_ws(app, ws, cx);
        app.update(cx, |app, cx| app.notify_panels(cx));
        cx.run_until_parked();
    }

    fn click_id(handle: gpui_kit::AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        cx.update_window(handle, |_, window, cx| {
            window.click(id.to_string(), cx);
        })
        .expect("update window");
        cx.run_until_parked();
    }

    /// 5.1 渲染冒烟: 注册项目树进右槽 + header 锚点在场
    /// (was: panel_renders_stub_source — 源缝 → 注册缝)。
    #[gpui_kit::test]
    fn panel_renders_registered_tree(cx: &mut TestAppContext) {
        let (handle, app_entity) = open_app(cx);
        let ws = temp_ws("render");
        std::fs::write(ws.join("stub-top.rs"), "x").unwrap();
        setup_filetree(cx, &app_entity, &ws, None);

        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("stub-top.rs").is_some(),
                "注册项目的文件应渲染进右槽"
            );
            assert!(
                window.try_find("tree-title").is_some(),
                "header 标题锚点在场"
            );
            assert!(
                window.try_find("tree-refresh").is_some(),
                "header 刷新按钮在场"
            );
        })
        .expect("update window");
    }

    /// DTO→TreeItem 适配 roundtrip: 顶层可见、默认折叠、展开贴回集合。
    #[gpui_kit::test]
    fn tree_adapter_roundtrip(cx: &mut TestAppContext) {
        let (handle, app_entity) = open_app(cx);
        let ws = temp_ws("adapter");
        std::fs::create_dir_all(ws.join("stub-dir")).unwrap();
        std::fs::write(ws.join("stub-dir/stub-app.rs"), "x").unwrap();
        std::fs::write(ws.join("stub-top.rs"), "x").unwrap();
        setup_filetree(cx, &app_entity, &ws, None);

        render(handle, cx);
        // 模型行 (Project/Module) 默认展开 → ws 顶层条目首屏即可见;
        // 普通目录 (stub-dir) 默认折叠 → 其子不渲染
        cx.update_window(handle, |_, window, _cx| {
            assert!(window.try_find_by_label("stub-dir").is_some(), "顶层目录在场");
            assert!(window.try_find_by_label("stub-top.rs").is_some(), "顶层文件在场");
            assert!(
                window.try_find_by_label("stub-app.rs").is_none(),
                "折叠时子节点不渲染"
            );
        })
        .expect("update window");

        // 展开: 库内置切换 + TreeEvent → 面板集合同步
        let dir_cell = format!("cell-{}/stub-dir", ws.display());
        click_id(handle, &dir_cell, cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("stub-app.rs").is_some(),
                "展开后子节点渲染"
            );
        })
        .expect("update window");
        app_entity.update(cx, |app, cx| {
            let panel = app.tree.read(cx);
            let ws_path = ws.to_string_lossy().to_string();
            assert!(
                panel.expanded.contains(&format!("{ws_path}/stub-dir")),
                "展开态集合应记录该目录"
            );
            assert!(
                panel.dir_ids.contains(&format!("{ws_path}/stub-dir")),
                "目录 id 集合应含该目录"
            );
        });

        // 折叠: 视图与集合同时收回
        click_id(handle, &dir_cell, cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("stub-app.rs").is_none(),
                "折叠后子节点不渲染"
            );
        })
        .expect("update window");
        app_entity.update(cx, |app, cx| {
            let panel = app.tree.read(cx);
            assert!(
                !panel
                    .expanded
                    .contains(&format!("{}/stub-dir", ws.display())),
                "折叠后集合应移除该目录"
            );
        });
    }

    /// 5.3 (spec: Tree scope follows registered projects / Session
    /// switching leaves the tree alone): 树只认注册集 — **无任何活动
    /// 会话**照样渲染; 会话状态从不进入 manager 的失效输入 (枚举只由
    /// 注册/⟳ 触发, 见 in-flight 测试), 二者解耦即覆盖"切会话不重扫"。
    #[gpui_kit::test]
    fn tree_shows_registered_projects_without_active_session(cx: &mut TestAppContext) {
        let (handle, app_entity) = open_app(cx);
        let ws = temp_ws("no-session");
        std::fs::write(ws.join("no-session-file.txt"), "x").unwrap();
        setup_filetree(cx, &app_entity, &ws, None);

        let (active, registered) = cx.update(|cx| {
            let app = app_entity.read(cx);
            (app.active, app.workspace.is_some())
        });
        assert!(active.is_none(), "无活动会话");
        assert!(!registered, "甚至无需工作区实体 — 注册即可显树");

        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("no-session-file.txt").is_some(),
                "注册项目在无会话时照样显示"
            );
            assert!(window.try_find("tree-empty").is_none(), "非空态");
        })
        .expect("update window");
    }

    /// 5.3 (spec: No registered projects shows empty state): 零注册 → 空态。
    #[gpui_kit::test]
    fn zero_projects_shows_empty_state(cx: &mut TestAppContext) {
        let (handle, app_entity) = open_app(cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find("tree-empty").is_some(),
                "零注册 → 空态提示"
            );
            assert!(
                window.try_find("tree-body").is_none(),
                "空态不出树体"
            );
        })
        .expect("update window");
        let _ = app_entity;
    }

    /// 5.2 (spec: Left-click open hook): 目录行 = 切展开 + 打开意图
    /// no-op; 文件行经 manager → 默认打开后端 (dedup: 两次点击恰一个签)。
    #[gpui_kit::test]
    fn file_click_invokes_open_once(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let ws = temp_ws("clicks");
        std::fs::create_dir_all(ws.join("stub-dir")).unwrap();
        std::fs::write(ws.join("stub-dir/stub-child.txt"), "x").unwrap();
        std::fs::write(ws.join("stub-top.rs"), "x").unwrap();
        setup_filetree(cx, &app, &ws, None);
        render(handle, cx);

        // 目录行: 库内置展开生效 (共存证明), 打开意图为 None → 不入 dedup
        click_id(handle, &format!("cell-{}/stub-dir", ws.display()), cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("stub-child.txt").is_some(),
                "库内置切换生效 (共存: 内容 handler 与选中/切换各跑一次)"
            );
        })
        .expect("update window");
        let dir_opened = cx.update(|cx| {
            let app = app.read(cx);
            app.open_tabs.contains_key(&format!("{}/stub-dir", ws.display()))
        });
        assert!(!dir_opened, "目录行不产生打开意图 (kind 门禁)");

        // 文件行: 点两次 → manager 打开意图 → open_object 认领 .rs →
        // 中槽 viewer 签登记; dedup 保证恰一个 (恰一次语义)
        click_id(handle, &format!("cell-{}/stub-top.rs", ws.display()), cx);
        click_id(handle, &format!("cell-{}/stub-top.rs", ws.display()), cx);
        let opened = cx.update(|cx| {
            let app = app.read(cx);
            app.open_tabs.contains_key(&format!("{}/stub-top.rs", ws.display()))
                && app.open_tabs.len() == 1
        });
        assert!(
            opened,
            "文件行点击端到端分发到 open_object (.rs → viewer 签, 恰一个)"
        );
    }

    /// 树行点击不夺键盘: console active 时点未认领行 → 焦点回终端面板
    /// (was tree_click_keeps_keyboard_with_session — 会话依赖已除)。
    #[gpui_kit::test]
    fn tree_click_keeps_keyboard_with_console_tab(cx: &mut TestAppContext) {
        use gpui_kit::Focusable;

        let (handle, app) = open_app(cx);
        let ws = temp_ws("kbd");
        std::fs::create_dir_all(ws.join("stub-dir")).unwrap();
        setup_filetree(cx, &app, &ws, None);
        render(handle, cx);

        let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));
        let tree_focus = cx.update(|cx| app.read(cx).tree.read(cx).focus_handle(cx));

        // 模拟 dock TabGroup 把焦点交给树面板 (spike E2)
        cx.update_window(handle, |_, window, cx| {
            tree_focus.clone().focus(window, cx);
        })
        .expect("update window");
        cx.run_until_parked();

        // 点目录行 (无打开意图) → 根级 on_click 冒泡把键盘/IME 交回
        // active tab 面板 = 终端面板 (design D6-3)
        click_id(handle, &format!("cell-{}/stub-dir", ws.display()), cx);

        let focused = cx.update_window(handle, |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(
            focused,
            Some(term_focus),
            "console active 时树行点击后键盘焦点必须回到终端面板 (spec: 键盘转发 — 树不持焦)"
        );
    }

    /// viewer active 时树行点击 → 焦点交回 viewer 面板 (design D6-3)。
    #[gpui_kit::test]
    fn tree_click_moves_keyboard_to_viewer_tab(cx: &mut TestAppContext) {
        use gpui_kit::Focusable;

        let (handle, app) = open_app(cx);
        let ws = temp_ws("kbd-viewer");
        std::fs::write(ws.join("stub-top.rs"), "x").unwrap();
        setup_filetree(cx, &app, &ws, None);
        render(handle, cx);

        let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));

        // 点 .rs 行 → open 分发 → viewer 签开且成 active tab
        click_id(handle, &format!("cell-{}/stub-top.rs", ws.display()), cx);
        render(handle, cx);

        let viewer_focus = cx.update(|cx| {
            let app = app.read(cx);
            let panel_id = app
                .open_tabs
                .get(&format!("{}/stub-top.rs", ws.display()))
                .copied()
                .expect("点 .rs 行应开 viewer 签");
            let area = app.dock_area.read(cx);
            area.panel(panel_id)
                .expect("viewer 面板在 dock 中")
                .focus_handle(cx)
        });
        assert_ne!(viewer_focus, term_focus, "viewer 焦点 handle 必须区别于终端");

        // 模拟 dock TabGroup 把焦点交给 viewer (tab 切换内建行为)
        cx.update_window(handle, |_, window, cx| {
            viewer_focus.clone().focus(window, cx);
        })
        .expect("update window");
        cx.run_until_parked();

        // 重复点已开行 (dedup 聚焦不换面板) → 冒泡交回 viewer
        click_id(handle, &format!("cell-{}/stub-top.rs", ws.display()), cx);

        let focused = cx.update_window(handle, |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(
            focused,
            Some(viewer_focus),
            "viewer active 时树行点击后焦点必须交回 viewer 面板"
        );
        assert_ne!(focused, Some(term_focus), "焦点不得落回终端面板");
    }

    /// 5.2 (spec: Right-click context menu): 菜单项全部来自 manager
    /// (v1: 复制路径 + 在 Finder 显示, 库不自造项)。
    #[gpui_kit::test]
    fn right_click_shows_asset_menu(cx: &mut TestAppContext) {
        use gpui_kit::Role;

        let (handle, app) = open_app(cx);
        let ws = temp_ws("menu");
        std::fs::write(ws.join("stub-top.rs"), "x").unwrap();
        setup_filetree(cx, &app, &ws, None);
        render(handle, cx);

        cx.update_window(handle, |_, window, cx| {
            window.right_click(format!("cell-{}/stub-top.rs", ws.display()), cx);
        })
        .expect("update window");
        cx.run_until_parked();
        render(handle, cx);

        cx.update_window(handle, |_, window, _cx| {
            let items = window.find_all_by_role(Role::MenuItem);
            assert_eq!(
                items.len(),
                2,
                "右键菜单项应全部来自 asset_manager (v1 两项)"
            );
        })
        .expect("update window");
    }

    /// 5.2: 菜单项点击 → manager execute → CopyText 效果 → 面板写剪贴板。
    /// (效果映射本身在 asset_manager::manager 测试断言 — 源缝的继任。)
    #[gpui_kit::test]
    fn copy_path_writes_clipboard(cx: &mut TestAppContext) {
        use gpui_kit::Role;

        let (handle, app) = open_app(cx);
        let ws = temp_ws("copy");
        std::fs::write(ws.join("stub-top.rs"), "x").unwrap();
        setup_filetree(cx, &app, &ws, None);
        render(handle, cx);

        cx.update_window(handle, |_, window, cx| {
            window.right_click(format!("cell-{}/stub-top.rs", ws.display()), cx);
        })
        .expect("update window");
        cx.run_until_parked();
        render(handle, cx);

        // 点击第一项 (复制路径) → execute → CopyText 效果 → 面板写剪贴板
        let item_id = cx.update_window(handle, |_, window, _cx| {
            let items = window.find_all_by_role(Role::MenuItem);
            assert!(!items.is_empty(), "菜单项应可寻址");
            items[0].path().last().expect("菜单项路径").clone()
        })
        .expect("update window");
        // 菜单项 id 是索引 (Integer(0)), 与树行撞 — 限定 popup-menu 子树
        cx.update_window(handle, |_, window, cx| {
            let mut menu = window.within("popup-menu");
            menu.click(item_id, cx);
        })
        .expect("update window");
        cx.run_until_parked();

        let text = cx.update(|cx| cx.read_from_clipboard())
            .and_then(|item| item.text());
        assert_eq!(
            text.as_deref(),
            Some(format!("{}/stub-top.rs", ws.display())).as_deref(),
            "复制路径效果应把行的绝对路径写进剪贴板 (manager execute → CopyText → 面板落地)"
        );
    }

    /// 3.2→5.x (spec: Data-change notification is O(1) / Unchanged view
    /// serves from cache): 代数未变的通知不得拉视图 — manager.view_pulls
    /// 计数 spy; 缓存视图照常渲染。
    #[gpui_kit::test]
    fn unchanged_epoch_notification_does_not_rebuild_view(cx: &mut TestAppContext) {
        let (handle, app_entity) = open_app(cx);
        let ws = temp_ws("epoch");
        std::fs::write(ws.join("stub-top.rs"), "x").unwrap();
        setup_filetree(cx, &app_entity, &ws, None);
        render(handle, cx); // 首次同步 (setup 的清场通知 + 落地通知各拉一次)

        let m = manager(cx, &app_entity);
        let (pulls0, gen0) = cx.update(|cx| {
            let m = m.read(cx);
            (m.view_pulls_handle().get(), m.view_generation())
        });
        assert!(pulls0 >= 1, "首次同步后已拉过视图: {pulls0}");

        for _ in 0..5 {
            app_entity.update(cx, |app, cx| app.notify_panels(cx));
            cx.run_until_parked();
        }
        let (pulls1, gen1) = cx.update(|cx| {
            let m = m.read(cx);
            (m.view_pulls_handle().get(), m.view_generation())
        });
        assert_eq!(
            pulls1, pulls0,
            "代数未变的 5 次通知一次都不得拉视图 (spec: O(1) notification)"
        );
        assert_eq!(gen1, gen0, "通知不推进代数");

        // 未重建不等于不渲染: 面板仍从缓存出内容
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("stub-top.rs").is_some(),
                "缓存视图照常渲染"
            );
        })
        .expect("update window");
    }

    /// 5.x (spec: In-flight enumeration is not restarted by notifications):
    /// 注册后枚举在途期间的连续数据通知 — 恰一次枚举、代数不动;
    /// pump 后单次落地上屏。
    #[gpui_kit::test]
    fn notifications_during_inflight_start_exactly_one_enumeration(
        cx: &mut TestAppContext,
    ) {
        let (handle, app) = open_app(cx);
        let ws = temp_ws("slowscan");
        std::fs::write(ws.join("slow-a.txt"), "x").unwrap();

        // 注册 → 枚举启动; 不 pump → 后台枚举保持在途
        register_ws(&app, &ws, cx);
        let m = manager(cx, &app);
        let (starts0, gen_inflight, view) = cx.update(|cx| {
            let m = m.read(cx);
            (
                m.enumerate_starts_handle().get(),
                m.view_generation(),
                m.view(),
            )
        });
        assert_eq!(starts0, 1, "注册恰启动一次枚举");
        assert!(
            matches!(view, AssetView::Empty),
            "枚举在途: 尚未落地 (view = 清场后的 Empty)"
        );

        // 在途期间的连续帧 (打字消费): 5 次数据通知
        let tree = cx.update(|cx| app.read(cx).tree.clone());
        for _ in 0..5 {
            tree.update(cx, |panel, cx| panel.on_data_changed(cx));
        }
        let (starts1, gen_after) = cx.update(|cx| {
            let m = m.read(cx);
            (m.enumerate_starts_handle().get(), m.view_generation())
        });
        assert_eq!(starts1, 1, "在途枚举不得被通知重启 (恰一次)");
        assert_eq!(gen_after, gen_inflight, "通知未触发失效 (代数不动)");

        // pump → 单次落地 → 内容上屏
        cx.run_until_parked();
        let starts2 = cx.update(|cx| m.read(cx).enumerate_starts_handle().get());
        assert_eq!(starts2, 1, "落地不增枚举数 (single completion)");
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("slow-a.txt").is_some(),
                "枚举落地后文件上屏"
            );
        })
        .expect("update window");
    }

    /// spec: Explicit refresh still restarts — 在途期间 ⟳ 必须启动新枚举。
    #[gpui_kit::test]
    fn explicit_refresh_restarts_inflight_enumeration(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let ws = temp_ws("explicit-restart");
        std::fs::write(ws.join("restart-a.txt"), "x").unwrap();

        register_ws(&app, &ws, cx); // 不 park: 在途
        let m = manager(cx, &app);
        assert_eq!(cx.update(|cx| m.read(cx).enumerate_starts_handle().get()), 1);

        // 显式刷新 (⟳ 处理器同路径 app.refresh_assets) → 必须重启
        app.update(cx, |app, cx| app.refresh_assets(cx));
        assert_eq!(
            cx.update(|cx| m.read(cx).enumerate_starts_handle().get()),
            2,
            "显式刷新在在途期间仍启动新枚举 (spec)"
        );

        cx.run_until_parked();
        assert_eq!(cx.update(|cx| m.read(cx).enumerate_starts_handle().get()), 2);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("restart-a.txt").is_some(),
                "重启后的枚举内容上屏"
            );
        })
        .expect("update window");
    }

    /// 5.1 (spec: Hidden panel costs nothing per frame): 右 dock 关闭期间
    /// 的数据通知不得拉视图/触碰枚举; 重开渲染缓存内容且不触发刷新。
    #[gpui_kit::test]
    fn hidden_panel_notifications_do_no_work(cx: &mut TestAppContext) {
        let (handle, app_entity) = open_app(cx);
        let ws = temp_ws("hidden");
        std::fs::write(ws.join("stub-top.rs"), "x").unwrap();
        setup_filetree(cx, &app_entity, &ws, None);
        render(handle, cx); // 可见状态: tree-close 按钮在帧中

        let m = manager(cx, &app_entity);
        // 关闭右 dock (× → close_right_dock); 再渲一帧确认面板未渲染
        cx.update_window(handle, |_, window, cx| {
            window.click("tree-close".to_string(), cx)
        })
        .expect("update window");
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find("tree-root").is_none(),
                "dock 关闭 → 面板未渲染"
            );
        })
        .expect("update window");

        // 隐藏期间 5 次数据通知 → 零面板工作 (闸门先于一切 manager 触碰)
        let (pulls0, starts0) = cx.update(|cx| {
            let m = m.read(cx);
            (m.view_pulls_handle().get(), m.enumerate_starts_handle().get())
        });
        for _ in 0..5 {
            app_entity.update(cx, |app, cx| app.notify_panels(cx));
            cx.run_until_parked();
        }
        let (pulls1, starts1) = cx.update(|cx| {
            let m = m.read(cx);
            (m.view_pulls_handle().get(), m.enumerate_starts_handle().get())
        });
        assert_eq!(pulls1, pulls0, "隐藏期间不得拉视图");
        assert_eq!(starts1, starts0, "隐藏期间不得触碰枚举");

        // 重开 (toggle-dock:Right) → 渲染缓存内容, 不触发新枚举/重拉
        cx.update_window(handle, |_, window, cx| {
            window.click("toggle-dock:Right".to_string(), cx)
        })
        .expect("update window");
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("stub-top.rs").is_some(),
                "重开显示缓存视图"
            );
        })
        .expect("update window");
        let (pulls2, starts2) = cx.update(|cx| {
            let m = m.read(cx);
            (m.view_pulls_handle().get(), m.enumerate_starts_handle().get())
        });
        assert_eq!(pulls2, pulls0, "代数未变: 重开渲染不重拉视图");
        assert_eq!(starts2, starts0, "重开不触发枚举 (无重扫)");
    }

    /// 5.1 (spec: Reopening the dock shows the current tree): 隐藏期间
    /// 落地的枚举推进代数但通知被闸门拦下; 重开渲染的惰性代数检查补
    /// 同步 → 最后失效时的内容上屏, 且不因曾隐藏而重枚举。
    #[gpui_kit::test]
    fn reopen_after_hidden_shows_landed_enumeration_without_rescan(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let ws = temp_ws("reopen");
        std::fs::write(ws.join("reopen-a.txt"), "x").unwrap();

        // 注册, 不 pump → 首次枚举在途; 先渲一帧 (不 park! — 清场空态,
        // 枚举保持在途, 对齐旧测时序)
        register_ws(&app, &ws, cx);
        let m = manager(cx, &app);
        assert_eq!(cx.update(|cx| m.read(cx).enumerate_starts_handle().get()), 1);

        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .expect("update window");
        // 关闭 dock → 确认面板未渲染 (枚举仍在途)
        cx.update_window(handle, |_, window, cx| {
            window.click("tree-close".to_string(), cx)
        })
        .expect("update window");
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .expect("update window");
        cx.update_window(handle, |_, window, _cx| {
            assert!(window.try_find("tree-root").is_none(), "dock 关闭 → 未渲染");
        })
        .expect("update window");

        // 隐藏期间: 新文件落盘 + pump → 枚举落地 (代数推进),
        // 落地通知被闸门拦下 → 面板代数仍停在隐藏前
        std::fs::write(ws.join("reopen-b.txt"), "x").unwrap();
        cx.run_until_parked();
        assert_eq!(
            cx.update(|cx| m.read(cx).enumerate_starts_handle().get()),
            1,
            "落地不启动新枚举"
        );

        // 重开 → 渲染路径惰性代数检查 → 最后失效时的内容上屏
        cx.update_window(handle, |_, window, cx| {
            window.click("toggle-dock:Right".to_string(), cx)
        })
        .expect("update window");
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("reopen-a.txt").is_some(),
                "重开显示最后失效时的树"
            );
            assert!(
                window.try_find_by_label("reopen-b.txt").is_some(),
                "隐藏期间落地的枚举内容也在 (代数补同步)"
            );
        })
        .expect("update window");
        assert_eq!(
            cx.update(|cx| m.read(cx).enumerate_starts_handle().get()),
            1,
            "重开不因曾隐藏而重枚举 (spec: no rescan)"
        );
    }

    /// spec: Refresh picks up new files — ⟳ → manager 重枚举。
    #[gpui_kit::test]
    fn refresh_picks_up_new_files(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let ws = temp_ws("refresh");
        std::fs::write(ws.join("refresh-before.txt"), "x").unwrap();
        setup_filetree(cx, &app, &ws, None);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("refresh-before.txt").is_some(),
                "首枚举后旧文件在树中 (first registration)"
            );
        })
        .expect("update window");

        // 磁盘新文件 → 点 ⟳ (显式重枚举, spec: manual refresh)
        std::fs::write(ws.join("refresh-after.txt"), "x").unwrap();
        click_id(handle, "tree-refresh", cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("refresh-after.txt").is_some(),
                "⟳ 后新文件出现在树中"
            );
        })
        .expect("update window");
    }

    /// spec: Refresh re-reads filter rules — 改 .gitignore → ⟳ 生效。
    #[gpui_kit::test]
    fn refresh_rereads_filter_rules(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let ws = temp_ws("reread");
        std::fs::write(ws.join("reread-a.txt"), "x").unwrap();
        std::fs::write(ws.join("reread-b.rs"), "x").unwrap();
        setup_filetree(cx, &app, &ws, Some(&["*.txt"]));
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("reread-b.rs").is_some(),
                "未匹配规则的文件显示"
            );
            assert!(
                window.try_find_by_label("reread-a.txt").is_none(),
                "首枚举即应用规则 *.txt (a.txt 被藏)"
            );
        })
        .expect("update window");

        // 改配置文件 → ⟳ 重读规则 (spec: refresh re-reads filter rules)
        write_rules(&ws, &["*.rs"]);
        click_id(handle, "tree-refresh", cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("reread-a.txt").is_some(),
                "新规则 *.rs 生效 → a.txt 回来"
            );
            assert!(
                window.try_find_by_label("reread-b.rs").is_none(),
                "新规则 *.rs 生效 → b.rs 被藏"
            );
        })
        .expect("update window");
    }

    /// spec: Refresh preserves expansion — ⟳ 后展开态从面板集合贴回。
    #[gpui_kit::test]
    fn refresh_preserves_expansion(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let ws = temp_ws("keepexp");
        std::fs::create_dir_all(ws.join("keepdir")).unwrap();
        std::fs::write(ws.join("keepdir/keep-child.txt"), "x").unwrap();
        setup_filetree(cx, &app, &ws, None);
        render(handle, cx);

        let dir_cell = format!("cell-{}/keepdir", ws.display());
        click_id(handle, &dir_cell, cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("keep-child.txt").is_some(),
                "展开后子文件可见"
            );
        })
        .expect("update window");

        // ⟳ 全量重枚举 → 展开态从面板集合贴回 (design D4)
        click_id(handle, "tree-refresh", cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("keep-child.txt").is_some(),
                "重枚举后展开态保留 (spec: Refresh preserves expansion)"
            );
        })
        .expect("update window");
        app.update(cx, |app, cx| {
            let ws_path = ws.to_string_lossy().to_string();
            assert!(
                app.tree
                    .read(cx)
                    .expanded
                    .contains(&format!("{ws_path}/keepdir")),
                "expanded 集合跨刷新保留"
            );
        });
    }

    /// 5.3 (spec: Degraded view shows hint): 删根 → ⟳ → 降级提示, 不崩,
    /// 不把旧内容当现状。
    #[gpui_kit::test]
    fn deleted_root_degrades_gracefully(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let ws = temp_ws("degrade");
        std::fs::write(ws.join("degrade-x.txt"), "x").unwrap();
        setup_filetree(cx, &app, &ws, None);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(window.try_find_by_label("degrade-x.txt").is_some());
        })
        .expect("update window");

        // 删根 → ⟳ → 降级提示, 不崩, 不把旧内容当现状
        std::fs::remove_dir_all(&ws).unwrap();
        click_id(handle, "tree-refresh", cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find("tree-degraded").is_some(),
                "根不可读 → 降级提示 (spec: degraded state)"
            );
            assert!(
                window.try_find_by_label("degrade-x.txt").is_none(),
                "降级不得把旧内容当现状"
            );
        })
        .expect("update window");

        // 应用仍存活: 再触发一次刷新不 panic
        click_id(handle, "tree-refresh", cx);
        render(handle, cx);
    }

    /// dock-layout delta (file-filter-prune): header 摘要位已撤除 —
    /// 规则源 = 项目根 .gitignore, 头部只保留 标题 + ⟳ + 关闭。
    #[gpui_kit::test]
    fn header_has_no_rule_summary(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let ws = temp_ws("summary");
        std::fs::write(ws.join("summary-a.txt"), "x").unwrap();
        setup_filetree(cx, &app, &ws, Some(&["*.txt"]));
        render(handle, cx);

        let read_texts = |cx: &mut TestAppContext| -> Vec<String> {
            cx.update_window(handle, |_, window, _cx| {
                gpui_kit::base::test_support::texts(window)
                    .iter()
                    .map(|t| t.text().to_string())
                    .collect()
            })
            .expect("update window")
        };
        // 标题/摘要位按 id 查 (texts() 只收录 test_text 节点 — 原摘要
        // 用 test_text, 标题与结构性检查走 try_find)
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find("tree-title").is_some(),
                "header 标题在帧 (dock-layout: Header chrome present)"
            );
            assert!(
                window.try_find("tree-rules").is_none(),
                "摘要位已撤除 (dock-layout delta): tree-rules 不在帧"
            );
        })
        .expect("update window");
        let texts = read_texts(cx);
        assert!(
            !texts.iter().any(|t| t.contains("过滤:")),
            "摘要位已撤除, 不应出现规则摘要, 实际文本: {texts:?}"
        );

        // 刷新后仍无摘要 (规则变化不再映射为头部文本; ⟳ 钮仍在)
        write_rules(&ws, &["*.log"]);
        click_id(handle, "tree-refresh", cx);
        render(handle, cx);
        let texts = read_texts(cx);
        assert!(
            !texts.iter().any(|t| t.contains("过滤:")),
            "刷新后仍无摘要, 实际文本: {texts:?}"
        );
    }
    /// 7.1 全链路 (app × 面板贯通): 打开注册 → 异步枚举落树 → 过滤 →
    /// ⟳ 保展开 → 加 solution 增量 → 切换清场。
    #[gpui_kit::test]
    fn full_chain_open_filter_refresh_add_switch(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);

        // 1. 工作区 ws1 (项目根 = ws1 自身): 被过滤的 .log + 目录 + .rs
        let ws1 = temp_ws("chain1");
        std::fs::write(ws1.join("top.rs"), "x").unwrap();
        std::fs::write(ws1.join("noise.log"), "x").unwrap();
        std::fs::create_dir_all(ws1.join("keepdir")).unwrap();
        std::fs::write(ws1.join("keepdir/child.txt"), "x").unwrap();
        // 规则在项目根 .gitignore (file-filter-prune D9), 与 marker
        // 无涉 — 先建工作区文件后写规则即可
        crate::project_model::create_workspace_files(&ws1).unwrap();
        crate::project_model::append_simple_solution(&ws1, ws1.clone()).unwrap();
        write_rules(&ws1, &["*.log"]);

        // 隔离历史: commit 会写历史, 不得污染进程共享的默认路径
        let hist = std::env::temp_dir().join(format!(
            "chain-hist-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&hist);
        app.update(cx, |app, _| {
            app.history_path = hist;
        });

        // 打开 → 注册 → 异步落树 → 过滤生效
        app.update(cx, |app, cx| app.begin_open(ws1.clone(), cx));
        cx.run_until_parked();
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("top.rs").is_some(),
                "打开后项目树落屏"
            );
            assert!(
                window.try_find_by_label("noise.log").is_none(),
                "规则 *.log 在 land 时生效"
            );
            assert!(
                window
                    .try_find(format!("cell-{}", crate::asset_manager::project_id(&ws1)))
                    .is_some(),
                "Project 行在场"
            );
        })
        .expect("update window");

        // 展开 keepdir
        click_id(handle, &format!("cell-{}/keepdir", ws1.display()), cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(window.try_find_by_label("child.txt").is_some(), "展开可见子文件");
        })
        .expect("update window");

        // ⟳ → 保展开 + 过滤仍生效 (规则重读同值)
        click_id(handle, "tree-refresh", cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("child.txt").is_some(),
                "⟳ 后展开态保留"
            );
            assert!(
                window.try_find_by_label("noise.log").is_none(),
                "⟳ 后过滤仍生效"
            );
            assert!(window.try_find_by_label("top.rs").is_some());
        })
        .expect("update window");

        // 加 solution (工作区外项目) → 增量注册, 无重启
        let p2 = temp_ws("chain-p2");
        std::fs::write(p2.join("second.rs"), "x").unwrap();
        app.update(cx, |app, cx| {
            let items =
                crate::project_model::append_simple_solution(&ws1, p2.clone()).unwrap();
            app.apply_add_solution(p2.clone(), Ok(items), cx);
        });
        cx.run_until_parked();
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window
                    .try_find(format!("cell-{}", crate::asset_manager::project_id(&p2)))
                    .is_some(),
                "新注册项目行出现 (增量)"
            );
            assert!(
                window.try_find_by_label("second.rs").is_some(),
                "新项目文件落屏"
            );
        })
        .expect("update window");

        // 切换到 ws2 → 注册集整体替换 (旧项目行无残留)
        let ws2 = temp_ws("chain2");
        std::fs::write(ws2.join("only2.txt"), "x").unwrap();
        crate::project_model::create_workspace_files(&ws2).unwrap();
        crate::project_model::append_simple_solution(&ws2, ws2.clone()).unwrap();
        app.update(cx, |app, cx| app.begin_open(ws2.clone(), cx));
        cx.run_until_parked();
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("only2.txt").is_some(),
                "切换后新工作区树在场"
            );
            assert!(
                window
                    .try_find(format!("cell-{}", crate::asset_manager::project_id(&ws2)))
                    .is_some(),
                "ws2 项目行在场"
            );
            assert!(
                window
                    .try_find(format!("cell-{}", crate::asset_manager::project_id(&ws1)))
                    .is_none(),
                "ws1 项目行无残留"
            );
            assert!(
                window
                    .try_find(format!("cell-{}", crate::asset_manager::project_id(&p2)))
                    .is_none(),
                "ws1 注册的外部项目行也无残留"
            );
            assert!(window.try_find_by_label("top.rs").is_none(), "旧树内容清空");
        })
        .expect("update window");

        // manager 侧终态: 视图只含 ws2 的一个注册
        let m = manager(cx, &app);
        cx.update(|cx| {
            let view = m.read(cx).view();
            let AssetView::Forest { nodes, .. } = view else {
                panic!("切换后应为 Forest, got {view:?}")
            };
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0].id, crate::asset_manager::project_id(&ws2));
        });
    }
}
