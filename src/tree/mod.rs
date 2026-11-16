//! 右 dock 文件树面板（proposal：占位面板 → 文件树面板）。
//!
//! part2（design D7）：触发刷新、拉 `source.view()`、把中立 DTO 适配成
//! gpui 元素、渲染与分发。本模块**零文件语义**——过滤解释、扫描、菜单
//! 描述全在 `workspace_property_source`（design D1/i 定案）。
//!
//! chrome（design D9 / dock-layout spec 修订）：header = 标题（源名）+
//! ⟳ 刷新 + 只读规则摘要 + `×`；body = 空态 / 降级态 / gpui Tree。
//!
//! **焦点纪律（spec: 键盘转发）**：v1 面板内无可聚焦文本输入——根 div
//! 的 `on_click` 冒泡覆盖全部点击（行/按钮），把键盘/IME 焦点交回终端
//! 面板，键盘唯一归宿仍是 active session（占位面板同款路径）。
//!
//! **展开态面板自持（design D4）**：`expanded` 集合由面板维护（订阅
//! `TreeEvent`），构造 `TreeItem` 时贴回——刷新（重扫=新 `Rc`）与过滤
//! 重建都不丢展开；同步时按新树裁剪陈旧 id。

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::base::TestTextExt;
use gpui_kit::component::dock::{BasePanel, DockPlacement, Panel, PanelEvent};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::tree::{TreeEvent, TreeItem, TreeState, tree};
use gpui_kit::*;

use crate::app::TerminalApp;
use crate::workspace::store;
use crate::workspace_property_source::{
    create_source, SourceEffect, SourceNode, SourceNodeKind, SourceView, WorkspacePropertySource,
    DEFAULT_SOURCE_ID,
};

pub struct TreePanel {
    app: WeakEntity<TerminalApp>,
    /// `Focusable` 契约要求；dock 交互时会聚焦此 handle（spike E2 实测）。
    focus: FocusHandle,
    /// 插件边界：面板从第一天持 `Box<dyn _>`（design D1）。
    source: Box<dyn WorkspacePropertySource>,
    /// gpui 树状态（虚拟滚动 + 展开/选中由库管；数据由本面板推入）。
    tree_state: Entity<TreeState>,
    /// 展开态集合（design D4：UI 状态不跨数据源边界）。
    expanded: HashSet<String>,
    /// 全部目录 id（`is_folder()` 对空目录为 false → 图标/chevron 兜底）。
    /// Rc 包裹: render 每帧只需引用计数 +1 — 不做整集合克隆
    /// (spec: Keystrokes do not pay tree cost)。
    dir_ids: Rc<HashSet<String>>,
    /// 最近一次同步进 `tree_state` 的视图（render 读缓存，避免每帧克隆）。
    last_synced: Option<SourceView>,
    /// 最近同步的视图代数 (design D5): epoch 相等比对替代整树深比较 —
    /// 代数未变的通知零工作 (spec: Data-change notification is O(1))。
    last_generation: Option<u64>,
    /// `TreeEvent` 订阅（drop 即退订 → 必须存住）。
    _tree_events: Subscription,
}

impl TreePanel {
    pub fn new(app: WeakEntity<TerminalApp>, cx: &mut Context<Self>) -> Self {
        let source = create_source(DEFAULT_SOURCE_ID, app.clone(), store::filepath())
            .expect("默认数据源必须在注册表中注册");
        let tree_state = cx.new(|cx| TreeState::new(cx));
        let tree_events = cx.subscribe(&tree_state, |this, _state, event: &TreeEvent, cx| {
            match event {
                TreeEvent::Expanded(id) => {
                    this.expanded.insert(id.to_string());
                }
                TreeEvent::Collapsed(id) => {
                    this.expanded.remove(id.as_str());
                }
            }
            let _ = cx;
        });
        Self {
            app,
            focus: cx.focus_handle(),
            source,
            tree_state,
            expanded: HashSet::new(),
            dir_ids: Rc::new(HashSet::new()),
            last_synced: None,
            last_generation: None,
            _tree_events: tree_events,
        }
    }

    /// 测试注入缝（task 2.2）：整体替换数据源并触发同步。
    #[cfg(test)]
    pub(crate) fn set_source(
        &mut self,
        source: Box<dyn WorkspacePropertySource>,
        cx: &mut Context<Self>,
    ) {
        self.source = source;
        self.last_synced = None;
        self.last_generation = None; // 换源 → epoch 重新对齐 (防代数巧合命中)
        self.on_data_changed(cx);
    }

    /// 数据变化入口（app 的 `notify_panels` 转发名单对树面板调用）：
    /// 0. **可见性闸门**（design D7; spec: Tree panel skips work while not
    ///    rendered）：右 dock 关闭 → 面板未渲染 → 触碰源之前早退；
    /// 1. **重扫粒度 = 根路径变化**（spec: 同 workspace 切 agent 不重扫;
    ///    D6: 在途扫描的根也算当前根 → 通知不重启扫描，比对收敛在源侧
    ///    `refresh_if_needed`）；
    /// 2. **epoch 比对**（design D5）：代数未变 → 零工作（无视图克隆、
    ///    无整树深比较）；变化才 `view()` → 推入 `TreeState` → 重渲。
    pub(crate) fn on_data_changed(&mut self, cx: &mut Context<Self>) {
        if !self.right_dock_open(cx) {
            return; // 隐藏期间的通知零源工作 (spec: Hidden panel costs nothing)
        }
        self.source.refresh_if_needed(cx);
        self.sync_if_generation_changed(cx);
        cx.notify();
    }

    /// 右 dock 是否开着（6.1 闸门依据）：dock 关闭 = 面板未渲染；
    /// app 实体已释放也按未渲染处理。
    fn right_dock_open(&self, cx: &App) -> bool {
        self.app.upgrade().is_some_and(|app| {
            app.read(cx)
                .dock_area
                .read(cx)
                .is_dock_open(DockPlacement::Right)
        })
    }

    /// epoch 同步（design D5/D7，`on_data_changed` 与 render 惰性检查
    /// 共用）：代数变化才拉视图 → 推入 `TreeState`。
    fn sync_if_generation_changed(&mut self, cx: &mut Context<Self>) {
        let generation = self.source.view_generation();
        if self.last_generation != Some(generation) {
            let view = self.source.view();
            self.sync_tree(&view, cx);
            self.last_synced = Some(view);
            self.last_generation = Some(generation);
        }
    }

    /// DTO → `TreeItem` 推入（适配器核心，design D3）：
    /// - 顶层条目作根节点（跳过合成根，spec: structure as root nodes）；
    /// - 展开态从 `self.expanded` 贴回，随后按新树裁剪陈旧 id；
    /// - `dir_ids` 收集供图标兜底。
    fn sync_tree(&mut self, view: &SourceView, cx: &mut Context<Self>) {
        let (items, dirs, all_ids) = match view {
            SourceView::Tree { node, .. } => {
                let (dirs, all_ids) = collect_ids(node);
                let items: Vec<TreeItem> = node
                    .children
                    .iter()
                    .map(|child| to_tree_item(child, &self.expanded))
                    .collect();
                (items, dirs, all_ids)
            }
            _ => (Vec::new(), HashSet::new(), HashSet::new()),
        };
        self.expanded.retain(|id| all_ids.contains(id)); // 刷新后裁剪
        self.dir_ids = Rc::new(dirs);
        self.tree_state
            .update(cx, |state, cx| state.set_items(items, cx));
    }
}

/// 效果落地（design D2）：源返回**意图**，面板执行——剪贴板要窗口上下文、
/// reveal 要系统命令，源保持可单测、可跨插件边界。
fn apply_effects(effects: Vec<SourceEffect>, _window: &mut Window, cx: &mut App) {
    for effect in effects {
        match effect {
            SourceEffect::CopyText(text) => {
                cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text));
            }
            SourceEffect::RevealPath(path) => reveal_in_finder(&path),
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

/// 递归构造 `TreeItem`（children 全量给出，可见性由 expanded 控制——
/// 展开时库直接从已有 children 拉平，无需重新推数）。
fn to_tree_item(node: &SourceNode, expanded: &HashSet<String>) -> TreeItem {
    let mut item = TreeItem::new(node.id.clone(), node.label.clone())
        .expanded(expanded.contains(&node.id));
    if node.kind == SourceNodeKind::Dir && !node.children.is_empty() {
        item = item.children(node.children.iter().map(|c| to_tree_item(c, expanded)));
    }
    item
}

/// 收集 (目录 id 集合, 全部 id 集合)。
fn collect_ids(node: &SourceNode) -> (HashSet<String>, HashSet<String>) {
    let mut dirs = HashSet::new();
    let mut all = HashSet::new();
    fn walk(node: &SourceNode, dirs: &mut HashSet<String>, all: &mut HashSet<String>) {
        all.insert(node.id.clone());
        if node.kind == SourceNodeKind::Dir {
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
        // 惰性 epoch 检查 (design D7; spec: Reopening the dock shows the
        // current tree): 隐藏期间错过的失效在渲染路径补同步 — 重开 dock
        // 即显示最后失效时的内容, 不因曾隐藏而重扫。
        self.sync_if_generation_changed(cx);
        let app_refocus = self.app.clone();
        let title = self.source.name().to_string();
        // 只读规则摘要 (spec: header chrome; 数据来自源当前生效规则)
        let rules_summary = match &self.last_synced {
            Some(SourceView::Tree { rules, .. }) if !rules.is_empty() => {
                format!("过滤: {}", rules.join("  "))
            }
            Some(SourceView::Tree { .. }) => "无过滤规则".to_string(),
            _ => "—".to_string(),
        };
        // render 读同步缓存（on_data_changed 已保证先于 notify 同步）
        let body = match &self.last_synced {
            None | Some(SourceView::Empty) => div()
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
                .text_color(rgb(0x969696))
                .child("no active session")
                .child("选择会话后此处显示其文件树")
                .into_any_element(),
            Some(SourceView::Degraded { root, message }) => div()
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
                .text_color(rgb(0xcd3131))
                .child(format!("无法读取 {}", root.display()))
                .child(message.clone())
                .into_any_element(),
            Some(SourceView::Tree { .. }) => {
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
                        // (库只给 depth, 行高/虚拟化由 uniform_list 管)
                        // 左键: 内容 on_click 分发 open 钩子 — 库保证内容
                        // handler **先于**其内置"选中+切换"执行 (共存各恰一次),
                        // 焦点交回由根级 on_click 冒泡统一处理 (spec: 键盘转发)
                        ListItem::new(format!("row-{id}"))
                            .on_click({
                                let panel_weak = panel_weak.clone();
                                let open_id = id.clone();
                                move |_, window, cx| {
                                    panel_weak
                                        .update(cx, |panel, cx| {
                                            panel.source.open(&open_id, window, cx)
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
                        // 右键: 数据源描述功能 → PopupMenu; 项点击 → 数据源
                        // 执行 → 面板落地效果 (spec: Right-click context menu)
                        let panel_weak = menu_weak.clone();
                        move |_ix, entry, menu, _window, cx| {
                            let item_id = entry.item().id.clone();
                            let descs = match panel_weak
                                .update(cx, |panel, _cx| panel.source.menu(&item_id))
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
                                                .update(cx, |panel, _cx| {
                                                    panel.source.execute(&item_id, &func_id)
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
            // 面板** (design D6-3: console active 时即终端, 与旧行为逐字
            // 同效; viewer active 时归 viewer; × 自身的 close_right_dock
            // 也会聚焦, 幂等)
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
                    .border_color(rgb(0xd9d9d9))
                    .child(div().id("tree-title").test_support().text_sm().child(title))
                    .child(
                        div()
                            .id("tree-actions")
                            .flex()
                            .items_center()
                            .gap_2()
                            // 只读规则摘要 (5.5 填充内容; 先给锚点)
                            .child(
                                div()
                                    .id("tree-rules")
                                    .text_xs()
                                    .text_color(rgb(0x969696))
                                    // texts() 只收录 test_text/ObservedText
                                    // (pane 同款; 观测子文本拿不到)
                                    .child(rules_summary.clone())
                                    .test_text(rules_summary),
                            )
                            .child(
                                div()
                                    .id("tree-refresh")
                                    .test_support()
                                    .cursor_pointer()
                                    .px_1()
                                    .text_sm()
                                    .hover(|style| style.text_color(rgb(0x005fb8)))
                                    .child("⟳")
                                    .on_click(cx.listener(|this, _event: &ClickEvent, _window, cx| {
                                        this.source.refresh(cx);
                                        this.on_data_changed(cx); // 同步空态立即生效; 异步落树经 app 转发
                                    })),
                            )
                            .child(
                                div()
                                    .id("tree-close")
                                    .test_support()
                                    .cursor_pointer()
                                    .px_1()
                                    .text_sm()
                                    .hover(|style| style.text_color(rgb(0xcd3131)))
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
    //! task 2.2/3.1: 源注入缝 + DTO→TreeItem 适配（渲染 / 展开集合一致）。

    // 显式导入, 不用 glob — 防 `gpui_kit::*` 把 `#[test]` 宏遮蔽掉
    // (app.rs tests 同款教训: 属性宏展开的内层 #[test] 会解析到自身 → 递归爆)
    use crate::app::TerminalApp;
    use crate::workspace_property_source::{
        MenuDesc, SourceEffect, SourceNode, SourceNodeKind, SourceView, WorkspacePropertySource,
    };
    use gpui_kit::test::{TestQueryExt, TestWindowExt};
    use gpui_kit::{
        App, AppContext, Bounds, Point, TestAppContext, Window, WindowBounds, WindowOptions, px,
        size,
    };
    use crate::agent_entities::{AgentEntity, AgentSession, SessionStatus};
    use crate::workspace::Workspace;
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    /// 返回嵌套固定树的 stub 源 — 面板只经 trait 边界消费数据。
    /// `opens` 记录 open 钩子调用次数 (task 4.1 断言); `views` 记录
    /// `view()` 调用次数 (task 4.2 spy: epoch 未变的通知不得拉视图);
    /// `app` 供与真实源同款的 `open_object` 分发 (stub 也走真实链路)。
    struct StubSource {
        opens: Rc<Cell<usize>>,
        views: Rc<Cell<usize>>,
        /// `refresh`/`refresh_if_needed` 调用计数 (task 6.1 spy:
        /// 隐藏期间的通知不得触碰源)。
        refreshes: Rc<Cell<usize>>,
        app: gpui_kit::WeakEntity<TerminalApp>,
    }

    fn stub_tree() -> SourceView {
        SourceView::Tree {
            root: PathBuf::from("/stub"),
            rules: Vec::new(),
            node: SourceNode {
                id: "/stub".to_string(),
                label: "/stub".to_string(),
                kind: SourceNodeKind::Dir,
                children: vec![
                    SourceNode {
                        id: "/stub/stub-dir".to_string(),
                        label: "stub-dir".to_string(),
                        kind: SourceNodeKind::Dir,
                        children: vec![SourceNode {
                            id: "/stub/stub-dir/stub-app.rs".to_string(),
                            label: "stub-app.rs".to_string(),
                            kind: SourceNodeKind::File,
                            children: Vec::new(),
                        }],
                    },
                    SourceNode {
                        id: "/stub/stub-top.rs".to_string(),
                        label: "stub-top.rs".to_string(),
                        kind: SourceNodeKind::File,
                        children: Vec::new(),
                    },
                    // 无 handler 认领的文件行 (4.2: console active 场景
                    // 用它表达“行点击不持焦”现状保持)
                    SourceNode {
                        id: "/stub/stub-top.md".to_string(),
                        label: "stub-top.md".to_string(),
                        kind: SourceNodeKind::File,
                        children: Vec::new(),
                    },
                ],
            },
        }
    }

    impl WorkspacePropertySource for StubSource {
        fn name(&self) -> &str {
            "stub-tree"
        }
        fn refresh(&mut self, _cx: &mut App) {
            self.refreshes.set(self.refreshes.get() + 1);
        }
        fn refresh_if_needed(&mut self, _cx: &mut App) {
            // 无内部状态: 通知无需重扫 (与真实源的同根早退同效);
            // 计数仅为 6.1 闸门 spy (调用本身即"源工作")
            self.refreshes.set(self.refreshes.get() + 1);
        }
        fn view_generation(&self) -> u64 {
            0 // 视图恒定 → 代数恒定 (面板只同步首次)
        }
        fn view(&self) -> SourceView {
            self.views.set(self.views.get() + 1);
            stub_tree()
        }
        fn menu(&self, _item_id: &str) -> Vec<MenuDesc> {
            // 3 项 = 源驱动的证明 (库不会自造项); 与真实源的 2 项不同
            vec![
                MenuDesc { id: "stub-copy".into(), label: "stub-copy".into() },
                MenuDesc { id: "stub-reveal".into(), label: "stub-reveal".into() },
                MenuDesc { id: "stub-third".into(), label: "stub-third".into() },
            ]
        }
        fn execute(&mut self, item_id: &str, func_id: &str) -> Vec<SourceEffect> {
            match func_id {
                "stub-copy" => vec![SourceEffect::CopyText(item_id.to_string())],
                "stub-reveal" => vec![SourceEffect::RevealPath(PathBuf::from(item_id))],
                _ => Vec::new(),
            }
        }
        fn open(&mut self, item_id: &str, window: &mut Window, cx: &mut App) {
            self.opens.set(self.opens.get() + 1);
            // 与真实源同款分发 (tasks 4.1): 查 stub 权威树得 kind →
            // open_object (证明钩子端到端落到默认打开系统)
            let kind = {
                let view = stub_tree();
                match &view {
                    SourceView::Tree { node, .. } => {
                        crate::workspace_property_source::find_node_kind(node, item_id)
                    }
                    _ => None,
                }
            };
            let Some(kind) = kind else {
                return;
            };
            let obj = crate::property_detail_manager::OpenObject {
                path: PathBuf::from(item_id),
                kind,
            };
            self.app
                .update(cx, |app, cx| app.open_object(obj, window, cx))
                .ok();
        }
    }

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
                |window, cx| cx.new(|cx| TerminalApp::new(window, cx).expect(".workspaces.json 加载")),
            )
            .expect("open test window")
        });
        // 隔离外部配置: TerminalApp::new 会加载 CWD 的 `.workspaces.json`
        // (gitignored, 跑过一次 app 就可能出现), 按位置索引的测试会被
        // 多出来的条目错位 — 起完先清空, 各测试只注入自己的内存 workspace。
        app.update(cx, |app, _| app.workspaces.clear());
        (handle, app)
    }

    fn inject_stub(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
    ) -> Rc<Cell<usize>> {
        // 先在非更新态读出 tree 实体, 再单独 update — 注入缝内部会
        // resolve_active_root 读 app, 包在 app.update 里会重入 panic
        // ("cannot read TerminalApp while it is already being updated")。
        let opens = Rc::new(Cell::new(0));
        let weak_app = cx.update(|_| app.downgrade());
        let source = StubSource {
            opens: opens.clone(),
            views: Rc::new(Cell::new(0)),
            refreshes: Rc::new(Cell::new(0)),
            app: weak_app,
        };
        let tree = cx.update(|cx| app.read(cx).tree.clone());
        tree.update(cx, |panel, cx| panel.set_source(Box::new(source), cx));
        cx.run_until_parked();
        opens
    }

    /// 免进程假会话：只为占住 `ws.agents` 让 `resolve_active_root` 命中。
    struct FakeAgent {
        name: String,
        ws: PathBuf,
        parser: vt100::Parser,
    }

    impl FakeAgent {
        fn new(name: &str, ws: &Path) -> Self {
            Self {
                name: name.to_string(),
                ws: ws.to_path_buf(),
                parser: vt100::Parser::new(
                    std::num::NonZeroU16::new(24).unwrap(),
                    std::num::NonZeroU16::new(80).unwrap(),
                    0,
                ),
            }
        }
    }

    impl AgentEntity for FakeAgent {
        fn name(&self) -> &str {
            &self.name
        }
        fn ws_path(&self) -> &Path {
            &self.ws
        }
        fn screen(&self) -> &vt100::Screen {
            self.parser.screen()
        }
        fn poll(&mut self) -> bool {
            false
        }
        fn send_key(&mut self, _key: crossterm::event::KeyEvent) {}
        fn send_text(&mut self, _text: &str) {}
        fn send_wheel(&mut self, _lines: i16) {}
        fn send_click(&mut self, _x: u16, _y: u16) {}
        fn shutdown(&mut self) {}
        fn status(&self) -> SessionStatus {
            SessionStatus::Running
        }
        fn resize(&mut self, _cols: u16, _rows: u16) -> bool {
            false
        }
    }

    /// hermetic 临时 workspace 目录（进程 id 隔离，先清后建）。
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

    /// 写 hermetic 规则文件 (tag 隔离并行测试)。
    fn write_rules_file(tag: &str, ws: &Path, rules: &[&str]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("tree-rules-{tag}-{}.json", std::process::id()));
        // 形状必须完整: Persisted.workspaces 无 serde default (spec: 缺键 =
        // 损坏文件), 所以测试配置也要带 workspaces 键
        let json = serde_json::json!({
            "workspaces": [],
            "file_tree": { ws.to_string_lossy().as_ref(): rules }
        });
        std::fs::write(&path, json.to_string()).unwrap();
        path
    }

    /// 注入真实 `FileTreeSource`（`rules_path` 测试缝）+ 活动 workspace +
    /// 首次激活通知（根变化 → 触发 part1 扫描，spec: first load）。
    fn setup_filetree(
        cx: &mut TestAppContext,
        app: &gpui_kit::Entity<TerminalApp>,
        ws: &Path,
        rules_file: Option<&Path>,
    ) {
        let weak = cx.update(|_| app.downgrade());
        let rules_path = rules_file
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/nonexistent-filetree-rules"));
        let source =
            crate::workspace_property_source::filetree::FileTreeSource::new(weak, rules_path);
        let tree = cx.update(|cx| app.read(cx).tree.clone());
        tree.update(cx, |panel, cx| panel.set_source(Box::new(source), cx));

        app.update(cx, |app, cx| {
            app.workspaces.push(Workspace::new(ws.to_path_buf()));
            let agent = cx.new(|_| AgentSession(Box::new(FakeAgent::new("pi-1", ws))));
            let id = agent.entity_id();
            app.workspaces
                .last_mut()
                .expect("注入的 workspace")
                .agents
                .push(agent);
            app.active = Some(id);
            app.notify_panels(cx);
        });
        cx.run_until_parked();
    }

    fn click_id(handle: gpui_kit::AnyWindowHandle, id: &str, cx: &mut TestAppContext) {
        cx.update_window(handle, |_, window, cx| {
            window.click(id.to_string(), cx);
        })
        .expect("update window");
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn panel_renders_stub_source(cx: &mut TestAppContext) {
        let (handle, app_entity) = open_app(cx);
        inject_stub(cx, &app_entity);

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
        })
        .expect("update window");
        cx.run_until_parked();

        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("stub-top.rs").is_some(),
                "stub 源的顶层文件应渲染进右槽"
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

    #[gpui_kit::test]
    fn tree_adapter_roundtrip(cx: &mut TestAppContext) {
        let (handle, app_entity) = open_app(cx);
        inject_stub(cx, &app_entity);

        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
        })
        .expect("update window");
        cx.run_until_parked();

        // 顶层根节点渲染; 子节点因默认折叠不可见
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
        cx.update_window(handle, |_, window, cx| {
            window.click("cell-/stub/stub-dir", cx);
        })
        .expect("update window");
        cx.run_until_parked();

        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("stub-app.rs").is_some(),
                "展开后子节点渲染"
            );
        })
        .expect("update window");
        app_entity.update(cx, |app, cx| {
            let panel = app.tree.read(cx);
            assert!(
                panel.expanded.contains("/stub/stub-dir"),
                "展开态集合应记录该目录"
            );
            assert!(panel.dir_ids.contains("/stub/stub-dir"), "目录 id 集合应含该目录");
        });

        // 折叠: 视图与集合同时收回
        cx.update_window(handle, |_, window, cx| {
            window.click("cell-/stub/stub-dir", cx);
        })
        .expect("update window");
        cx.run_until_parked();
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("stub-app.rs").is_none(),
                "折叠后子节点收回"
            );
        })
        .expect("update window");
        app_entity.update(cx, |app, cx| {
            assert!(
                !app.tree.read(cx).expanded.contains("/stub/stub-dir"),
                "折叠后集合移除该目录"
            );
        });
    }

    #[gpui_kit::test]
    fn tree_scope_follows_active_workspace(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let ws1 = temp_ws("scope1");
        let ws2 = temp_ws("scope2");
        std::fs::write(ws1.join("scope-a.txt"), "a").unwrap();
        std::fs::write(ws2.join("scope-c.txt"), "c").unwrap();

        // 注入两 ws；ws1 两个 agent（同 ws 切换用），ws2 一个
        let (i2, i3) = app.update(cx, |app, cx| {
            app.workspaces.push(Workspace::new(ws1.clone()));
            app.workspaces.push(Workspace::new(ws2.clone()));
            let a1 = cx.new(|_| AgentSession(Box::new(FakeAgent::new("pi-1", &ws1))));
            let a2 = cx.new(|_| AgentSession(Box::new(FakeAgent::new("pi-2", &ws1))));
            let a3 = cx.new(|_| AgentSession(Box::new(FakeAgent::new("pi-1", &ws2))));
            let (i1, i2, i3) = (a1.entity_id(), a2.entity_id(), a3.entity_id());
            app.workspaces[0].agents.push(a1);
            app.workspaces[0].agents.push(a2);
            app.workspaces[1].agents.push(a3);
            app.active = Some(i1);
            app.notify_panels(cx); // 首次激活 → 根变化 → 扫描
            (i2, i3)
        });
        cx.run_until_parked();
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("scope-a.txt").is_some(),
                "活动 workspace 的文件应出现在树中"
            );
        })
        .expect("update window");

        // 同 ws 切 agent + 磁盘新文件 → 不重扫（spec: keeps tree）
        std::fs::write(ws1.join("scope-b.txt"), "b").unwrap();
        app.update(cx, |app, cx| {
            app.active = Some(i2);
            app.notify_panels(cx);
        });
        cx.run_until_parked();
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("scope-a.txt").is_some(),
                "同 workspace 切 agent 不换树"
            );
            assert!(
                window.try_find_by_label("scope-b.txt").is_none(),
                "同 workspace 切 agent 不重扫 (磁盘新文件不出现)"
            );
        })
        .expect("update window");

        // 切到另一 workspace → 换树（根变化 → 重扫）
        app.update(cx, |app, cx| {
            app.active = Some(i3);
            app.notify_panels(cx);
        });
        cx.run_until_parked();
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("scope-c.txt").is_some(),
                "切 workspace 后显示新 ws 的树"
            );
            assert!(
                window.try_find_by_label("scope-a.txt").is_none(),
                "旧 ws 的树被替换"
            );
        })
        .expect("update window");
    }

    #[gpui_kit::test]
    fn no_active_agent_shows_empty_state(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);

        // 初始: 无活跃 agent → 空态提示
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find("tree-empty").is_some(),
                "无活跃 agent → 空态提示"
            );
        })
        .expect("update window");

        // 有活跃 agent → 树出现、空态消失
        let ws = temp_ws("empty");
        std::fs::write(ws.join("empty-scope.txt"), "x").unwrap();
        app.update(cx, |app, cx| {
            app.workspaces.push(Workspace::new(ws.clone()));
            let agent = cx.new(|_| AgentSession(Box::new(FakeAgent::new("pi-1", &ws))));
            let id = agent.entity_id();
            app.workspaces[0].agents.push(agent);
            app.active = Some(id);
            app.notify_panels(cx);
        });
        cx.run_until_parked();
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("empty-scope.txt").is_some(),
                "有活跃 agent → 树渲染"
            );
            assert!(
                window.try_find("tree-empty").is_none(),
                "空态提示应消失"
            );
        })
        .expect("update window");

        // active 收回 → 空态回来
        app.update(cx, |app, cx| {
            app.active = None;
            app.notify_panels(cx);
        });
        cx.run_until_parked();
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find("tree-empty").is_some(),
                "active 归 None → 空态回来"
            );
            assert!(
                window.try_find_by_label("empty-scope.txt").is_none(),
                "树内容清空"
            );
        })
        .expect("update window");
    }

    #[gpui_kit::test]
    fn file_click_invokes_open_once(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let opens = inject_stub(cx, &app);
        render(handle, cx);

        // 目录行: open 钩子 + 库内置"选中+展开切换"共存, 各恰一次
        cx.update_window(handle, |_, window, cx| {
            window.click("cell-/stub/stub-dir", cx);
        })
        .expect("update window");
        cx.run_until_parked();
        assert_eq!(opens.get(), 1, "目录行点击 → open 钩子恰一次");
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("stub-app.rs").is_some(),
                "库内置切换同时生效 (共存证明: 内容 handler 与选中/切换各跑一次)"
            );
        })
        .expect("update window");

        // 文件行: 再来一次 → 除钩子计数外, 真实分发必须落到默认打开
        // 系统: open_object 认领 .rs → 中槽出现 viewer 签 (tasks 4.1)
        cx.update_window(handle, |_, window, cx| {
            window.click("cell-/stub/stub-top.rs", cx);
        })
        .expect("update window");
        cx.run_until_parked();
        assert_eq!(opens.get(), 2, "文件行点击 → open 钩子恰一次");
        let opened = cx.update(|cx| {
            let app = app.read(cx);
            app.open_tabs.contains_key("/stub/stub-top.rs")
        });
        assert!(
            opened,
            "open 钩子必须端到端分发到 open_object (.rs 行 → viewer 签登记)"
        );
    }

    #[gpui_kit::test]
    fn tree_click_keeps_keyboard_with_session(cx: &mut TestAppContext) {
        use gpui_kit::Focusable;

        let (handle, app) = open_app(cx);
        inject_stub(cx, &app);
        render(handle, cx);

        let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));
        let tree_focus = cx.update(|cx| app.read(cx).tree.read(cx).focus_handle(cx));

        // 模拟 dock TabGroup 把焦点交给树面板 (spike E2: dock 交互聚焦面板)
        cx.update_window(handle, |_, window, cx| {
            tree_focus.clone().focus(window, cx);
        })
        .expect("update window");
        cx.run_until_parked();

        // 点无 handler 认领的文件行 → 不开签 (console 仍 active) → 根级
        // on_click 冒泡把键盘/IME 交回 active tab 面板 = 终端面板
        // (design D6-3: console active 场景与旧行为逐字同效)
        cx.update_window(handle, |_, window, cx| {
            window.click("cell-/stub/stub-dir", cx);
        })
        .expect("update window");
        cx.run_until_parked();

        let focused = cx.update_window(handle, |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(
            focused,
            Some(term_focus),
            "console active 时树行点击(未认领对象)后键盘焦点必须回到终端面板 (spec: 键盘转发 — 树不持焦)"
        );
    }

    #[gpui_kit::test]
    fn tree_click_moves_keyboard_to_viewer_tab(cx: &mut TestAppContext) {
        use gpui_kit::Focusable;

        let (handle, app) = open_app(cx);
        inject_stub(cx, &app);
        render(handle, cx);

        let term_focus = cx.update(|cx| app.read(cx).terminal.read(cx).focus_handle(cx));

        // 点 .rs 行 → open 分发 → viewer 签开且成 active tab
        cx.update_window(handle, |_, window, cx| {
            window.click("cell-/stub/stub-top.rs", cx);
        })
        .expect("update window");
        cx.run_until_parked();
        render(handle, cx);

        // active tab 面板 = viewer (dedup 表里该对象的面板)
        let viewer_focus = cx.update(|cx| {
            let app = app.read(cx);
            let panel_id = app
                .open_tabs
                .get("/stub/stub-top.rs")
                .copied()
                .expect("点 .rs 行应开 viewer 签");
            let area = app.dock_area.read(cx);
            area.panel(panel_id)
                .expect("viewer 面板在 dock 中")
                .focus_handle(cx)
        });
        assert_ne!(
            viewer_focus, term_focus,
            "viewer 焦点 handle 必须区别于终端"
        );

        // 模拟 dock TabGroup 把焦点交给 viewer (tab 切换的内建行为)
        cx.update_window(handle, |_, window, cx| {
            viewer_focus.clone().focus(window, cx);
        })
        .expect("update window");
        cx.run_until_parked();

        // 树行点击冒泡 → 焦点交回 active tab 面板 = viewer (非终端);
        // 重复点已开的 .rs 行 (dedup 聚焦不换面板, 焦点语义与原 md 行同效)
        cx.update_window(handle, |_, window, cx| {
            window.click("cell-/stub/stub-top.rs", cx);
        })
        .expect("update window");
        cx.run_until_parked();

        let focused = cx.update_window(handle, |_, window, cx| window.focused(cx))
            .expect("update window");
        assert_eq!(
            focused,
            Some(viewer_focus),
            "viewer active 时树行点击后焦点必须交回 viewer 面板 (design D6-3)"
        );
        assert_ne!(
            focused,
            Some(term_focus),
            "焦点不得落回终端面板"
        );
    }

    #[gpui_kit::test]
    fn right_click_shows_source_menu(cx: &mut TestAppContext) {
        use gpui_kit::Role;

        let (handle, app) = open_app(cx);
        inject_stub(cx, &app);
        render(handle, cx);

        // 右键树行 → 菜单出现, 项数由数据源描述决定 (stub=3, 库不自造项)
        cx.update_window(handle, |_, window, cx| {
            window.right_click("cell-/stub/stub-top.rs", cx);
        })
        .expect("update window");
        cx.run_until_parked();
        render(handle, cx);

        cx.update_window(handle, |_, window, _cx| {
            let items = window.find_all_by_role(Role::MenuItem);
            assert_eq!(
                items.len(),
                3,
                "右键菜单项应全部来自数据源描述 (stub 返回 3 项)"
            );
        })
        .expect("update window");
    }

    #[gpui_kit::test]
    fn copy_path_writes_clipboard(cx: &mut TestAppContext) {
        use gpui_kit::Role;

        let (handle, app) = open_app(cx);
        inject_stub(cx, &app);
        render(handle, cx);

        cx.update_window(handle, |_, window, cx| {
            window.right_click("cell-/stub/stub-top.rs", cx);
        })
        .expect("update window");
        cx.run_until_parked();
        render(handle, cx);

        // 点击第一项 (stub-copy) → execute → CopyText 效果 → 面板写剪贴板
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
            Some("/stub/stub-top.rs"),
            "复制路径效果应把行的绝对路径写进剪贴板 (execute → CopyText → 面板落地)"
        );

        // reveal 效果产出断言 (真实 FileTreeSource 级): execute 必须把
        // 两个功能映射成对应效果意图 (面板负责落地, 源保持可单测)
        use crate::workspace_property_source::filetree::{
            FUNC_COPY_PATH, FUNC_REVEAL, FileTreeSource,
        };
        let weak = cx.update(|_| app.downgrade());
        let mut source = FileTreeSource::new(weak, PathBuf::from("/nonexistent-rules"));
        assert_eq!(
            source.execute("/a/proj/main.rs", FUNC_REVEAL),
            vec![SourceEffect::RevealPath(PathBuf::from("/a/proj/main.rs"))],
            "在 Finder 显示 → RevealPath 效果"
        );
        assert_eq!(
            source.execute("/a/proj/main.rs", FUNC_COPY_PATH),
            vec![SourceEffect::CopyText("/a/proj/main.rs".to_string())],
            "复制路径 → CopyText 效果"
        );
        assert!(
            source.execute("/a/proj/main.rs", "unknown-func").is_empty(),
            "未知功能 → 无效果"
        );
    }

    /// task 4.2 (spec: Data-change notification is O(1) when nothing
    /// changed / Unchanged view serves from cache): epoch 未变的通知
    /// 不得拉视图 — `StubSource.view` 计数 spy。set_source 同步恰一次;
    /// 之后 5 次 `notify_panels` 计数不动 (旧实现每次通知都 view() +
    /// 整树深比较 → 计数随通知增长)。
    #[gpui_kit::test]
    fn unchanged_epoch_notification_does_not_rebuild_view(cx: &mut TestAppContext) {
        let (handle, app_entity) = open_app(cx);
        let opens = Rc::new(Cell::new(0));
        let views = Rc::new(Cell::new(0));
        let refreshes = Rc::new(Cell::new(0));
        let weak_app = cx.update(|_| app_entity.downgrade());
        let source = StubSource {
            opens: opens.clone(),
            views: views.clone(),
            refreshes: refreshes.clone(),
            app: weak_app,
        };
        let tree = cx.update(|cx| app_entity.read(cx).tree.clone());
        tree.update(cx, |panel, cx| panel.set_source(Box::new(source), cx));
        cx.run_until_parked();

        assert_eq!(views.get(), 1, "set_source 首次同步恰拉一次视图");
        for _ in 0..5 {
            app_entity.update(cx, |app, cx| app.notify_panels(cx));
            cx.run_until_parked();
        }
        assert_eq!(
            views.get(),
            1,
            "epoch 未变的 5 次通知一次都不得重建视图 (spec: O(1) notification)"
        );

        // 未重建不等于不渲染: 面板仍从缓存出内容 (spec: cached view
        // still renders the same content)
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("stub-top.rs").is_some(),
                "缓存视图照常渲染"
            );
        })
        .expect("update window");
    }

    /// task 5.1+5.2 (spec: In-flight scan is not restarted by
    /// notifications): 扫描在途期间的连续数据通知 — 恰一次扫描、代数
    /// 不动; 随后 pump 扫描单次落地并上屏 (Typing during initial scan
    /// lets it finish — 原实现每帧重启扫描 → 本测红)。
    #[gpui_kit::test]
    fn notifications_during_inflight_scan_start_exactly_one_scan(cx: &mut TestAppContext) {
        use crate::workspace_property_source::filetree::FileTreeSource;
        let (handle, app) = open_app(cx);
        let ws = temp_ws("slowscan");
        std::fs::write(ws.join("slow-a.txt"), "x").unwrap();

        let weak = cx.update(|_| app.downgrade());
        let source = FileTreeSource::new(weak, PathBuf::from("/nonexistent-filetree-rules"));
        let scans = source.scan_starts_handle();
        let tree = cx.update(|cx| app.read(cx).tree.clone());
        tree.update(cx, |panel, cx| panel.set_source(Box::new(source), cx));

        // 激活 → 首次通知 (defer 于 update finish flush) → refresh 启动
        // 扫描; 不 pump → 后台扫描保持在途 (TestDispatcher 显式 tick)
        app.update(cx, |app, cx| {
            app.workspaces.push(Workspace::new(ws.clone()));
            let agent = cx.new(|_| AgentSession(Box::new(FakeAgent::new("pi-1", &ws))));
            let id = agent.entity_id();
            app.workspaces
                .last_mut()
                .expect("注入的 workspace")
                .agents
                .push(agent);
            app.active = Some(id);
            app.notify_panels(cx);
        });
        assert_eq!(scans.get(), 1, "首载恰启动一次扫描");
        let (gen_inflight, view) = cx.update(|cx| {
            let panel = app.read(cx).tree.read(cx);
            (panel.source.view_generation(), panel.source.view())
        });
        assert!(
            matches!(view, SourceView::Empty),
            "扫描在途: 尚未落地 (view = Empty)"
        );

        // 在途期间的连续帧 (打字消费): 5 次数据通知
        for _ in 0..5 {
            tree.update(cx, |panel, cx| panel.on_data_changed(cx));
        }
        assert_eq!(scans.get(), 1, "在途扫描不得被通知重启 (恰一次, 5.1)");
        let gen_after = cx.update(|cx| app.read(cx).tree.read(cx).source.view_generation());
        assert_eq!(gen_after, gen_inflight, "通知未触发 refresh (代数不动)");

        // pump → 扫描单次落地 (5.2) → 内容上屏
        cx.run_until_parked();
        assert_eq!(scans.get(), 1, "落地不增扫描数 (single completion)");
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("slow-a.txt").is_some(),
                "扫描落地后文件上屏"
            );
        })
        .expect("update window");
    }

    /// spec scenario Explicit refresh still restarts: 在途扫描期间的
    /// 显式刷新 (⟳ 处理器同路径 `source.refresh`) 必须启动新扫描 —
    /// 通知幂等 (refresh_if_needed) 不影响显式路径。
    #[gpui_kit::test]
    fn explicit_refresh_restarts_inflight_scan(cx: &mut TestAppContext) {
        use crate::workspace_property_source::filetree::FileTreeSource;
        let (handle, app) = open_app(cx);
        let ws = temp_ws("explicit-restart");
        std::fs::write(ws.join("restart-a.txt"), "x").unwrap();

        let weak = cx.update(|_| app.downgrade());
        let source = FileTreeSource::new(weak, PathBuf::from("/nonexistent-filetree-rules"));
        let scans = source.scan_starts_handle();
        let tree = cx.update(|cx| app.read(cx).tree.clone());
        tree.update(cx, |panel, cx| panel.set_source(Box::new(source), cx));
        app.update(cx, |app, cx| {
            app.workspaces.push(Workspace::new(ws.clone()));
            let agent = cx.new(|_| AgentSession(Box::new(FakeAgent::new("pi-1", &ws))));
            let id = agent.entity_id();
            app.workspaces
                .last_mut()
                .expect("注入的 workspace")
                .agents
                .push(agent);
            app.active = Some(id);
            app.notify_panels(cx);
        });
        assert_eq!(scans.get(), 1, "首载扫描在途");

        // 显式刷新 → 必须重启 (新扫描取代在途旧扫描)
        tree.update(cx, |panel, cx| panel.source.refresh(cx));
        assert_eq!(scans.get(), 2, "显式刷新在在途期间仍启动新扫描 (spec)");

        cx.run_until_parked();
        assert_eq!(scans.get(), 2, "新扫描单次落地");
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("restart-a.txt").is_some(),
                "重启后的扫描内容上屏"
            );
        })
        .expect("update window");
    }

    /// task 6.1 (spec: Hidden panel costs nothing per frame): 右 dock
    /// 关闭期间的数据通知不得触碰源 — refresh/view spy 计数均不动;
    /// 重开后缓存视图照常渲染且不触发刷新 (spec: Rendering again shows
    /// the current cached view without requiring a rescan)。
    #[gpui_kit::test]
    fn hidden_panel_notifications_do_no_source_work(cx: &mut TestAppContext) {
        let (handle, app_entity) = open_app(cx);
        let opens = Rc::new(Cell::new(0));
        let views = Rc::new(Cell::new(0));
        let refreshes = Rc::new(Cell::new(0));
        let weak_app = cx.update(|_| app_entity.downgrade());
        let source = StubSource {
            opens: opens.clone(),
            views: views.clone(),
            refreshes: refreshes.clone(),
            app: weak_app,
        };
        let tree = cx.update(|cx| app_entity.read(cx).tree.clone());
        tree.update(cx, |panel, cx| panel.set_source(Box::new(source), cx));
        render(handle, cx); // 可见状态: tree-close 按钮在帧中

        // 关闭右 dock (× → close_right_dock); 再渲一帧确认面板未渲染
        cx.update_window(handle, |_, window, cx| {
            window.click("tree-close".to_string(), cx)
        })
        .expect("update window");
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .expect("update window");
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find("tree-root").is_none(),
                "dock 关闭 → 面板未渲染"
            );
        })
        .expect("update window");

        // 隐藏期间 5 次数据通知 → 零源工作 (闸门先于一切源调用)
        let (v0, r0) = (views.get(), refreshes.get());
        for _ in 0..5 {
            app_entity.update(cx, |app, cx| app.notify_panels(cx));
            cx.run_until_parked();
        }
        assert_eq!(views.get(), v0, "隐藏期间不得拉视图");
        assert_eq!(refreshes.get(), r0, "隐藏期间不得触碰 refresh 路径");

        // 重开 (toggle-dock:Right) → 渲染缓存内容, 不触发刷新/重扫
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
        assert_eq!(refreshes.get(), r0, "重开不触发源刷新 (无重扫)");
        assert_eq!(views.get(), v0, "epoch 未变: 重开渲染不重拉视图");
    }

    /// task 6.2 (spec: Reopening the dock shows the current tree): 隐藏
    /// 期间落地的扫描推进代数但通知被闸门拦下; 重开渲染的惰性 epoch
    /// 检查补同步 → 最后失效时的内容上屏, 且不因曾隐藏而重扫。
    #[gpui_kit::test]
    fn reopen_after_hidden_shows_landed_scan_without_rescan(cx: &mut TestAppContext) {
        use crate::workspace_property_source::filetree::FileTreeSource;
        let (handle, app) = open_app(cx);
        let ws = temp_ws("reopen");
        std::fs::write(ws.join("reopen-a.txt"), "x").unwrap();

        // 注入 + 激活, 不 pump → 首扫在途 (TestDispatcher 显式 tick)
        let weak = cx.update(|_| app.downgrade());
        let source = FileTreeSource::new(weak, PathBuf::from("/nonexistent-filetree-rules"));
        let scans = source.scan_starts_handle();
        let tree = cx.update(|cx| app.read(cx).tree.clone());
        tree.update(cx, |panel, cx| panel.set_source(Box::new(source), cx));
        app.update(cx, |app, cx| {
            app.workspaces.push(Workspace::new(ws.clone()));
            let agent = cx.new(|_| AgentSession(Box::new(FakeAgent::new("pi-1", &ws))));
            let id = agent.entity_id();
            app.workspaces
                .last_mut()
                .expect("注入的 workspace")
                .agents
                .push(agent);
            app.active = Some(id);
            app.notify_panels(cx);
        });
        assert_eq!(scans.get(), 1, "首扫在途");

        // 渲染一帧 (空态) → 关闭 dock → 确认面板未渲染 (扫描仍在途)
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .expect("update window");
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

        // 隐藏期间: 新文件落盘 + pump → 扫描落地 (代数推进),
        // 落地通知被闸门拦下 → 面板代数仍停在隐藏前
        std::fs::write(ws.join("reopen-b.txt"), "x").unwrap();
        cx.run_until_parked();
        assert_eq!(scans.get(), 1, "落地不启动新扫描");

        // 重开 → 渲染路径惰性 epoch 检查 → 最后失效时的内容上屏
        cx.update_window(handle, |_, window, cx| {
            window.click("toggle-dock:Right".to_string(), cx)
        })
        .expect("update window");
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .expect("update window");
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("reopen-a.txt").is_some(),
                "重开显示最后失效时的树"
            );
            assert!(
                window.try_find_by_label("reopen-b.txt").is_some(),
                "隐藏期间落地的扫描内容也在 (epoch 补同步)"
            );
        })
        .expect("update window");
        assert_eq!(scans.get(), 1, "重开不因曾隐藏而重扫 (spec: no rescan)");
    }

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
                "首扫后旧文件在树中 (first load)"
            );
        })
        .expect("update window");

        // 磁盘新文件 → 点 ⟳ (同根强制重扫, spec: manual refresh)
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

    #[gpui_kit::test]
    fn refresh_rereads_filter_rules(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let ws = temp_ws("reread");
        std::fs::write(ws.join("reread-a.txt"), "x").unwrap();
        std::fs::write(ws.join("reread-b.rs"), "x").unwrap();
        let rules = write_rules_file("reread", &ws, &["*.txt"]);
        setup_filetree(cx, &app, &ws, Some(&rules));
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("reread-b.rs").is_some(),
                "未匹配规则的文件显示"
            );
            assert!(
                window.try_find_by_label("reread-a.txt").is_none(),
                "首扫即应用规则 *.txt (a.txt 被藏)"
            );
        })
        .expect("update window");

        // 改配置文件 → ⟳ 重读规则 (spec: refresh re-reads filter rules)
        write_rules_file("reread", &ws, &["*.rs"]);
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

        // ⟳ 全量重扫 → 展开态从面板集合贴回 (design D4)
        click_id(handle, "tree-refresh", cx);
        render(handle, cx);
        cx.update_window(handle, |_, window, _cx| {
            assert!(
                window.try_find_by_label("keep-child.txt").is_some(),
                "重扫后展开态保留 (spec: Refresh preserves expansion)"
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

    #[gpui_kit::test]
    fn header_shows_rule_summary(cx: &mut TestAppContext) {
        let (handle, app) = open_app(cx);
        let ws = temp_ws("summary");
        std::fs::write(ws.join("summary-a.txt"), "x").unwrap();
        let rules = write_rules_file("summary", &ws, &["*.txt"]);
        setup_filetree(cx, &app, &ws, Some(&rules));
        render(handle, cx);

        let read_summary = |cx: &mut TestAppContext| -> Vec<String> {
            cx.update_window(handle, |_, window, _cx| {
                gpui_kit::base::test_support::texts(window)
                    .iter()
                    .map(|t| t.text().to_string())
                    .collect()
            })
            .expect("update window")
        };
        let texts = read_summary(cx);
        assert!(
            texts.iter().any(|t| t.contains("过滤: *.txt")),
            "header 摘要应显示当前生效规则, 实际文本: {texts:?}"
        );

        // 改规则 → ⟳ → 摘要随规则更新 (spec: header chrome)
        write_rules_file("summary", &ws, &["*.log"]);
        click_id(handle, "tree-refresh", cx);
        render(handle, cx);
        let texts = read_summary(cx);
        assert!(
            texts.iter().any(|t| t.contains("过滤: *.log")),
            "刷新后摘要应更新为新规则, 实际文本: {texts:?}"
        );
    }
}
