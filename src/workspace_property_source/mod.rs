//! `workspace_property_source`: workspace 属性数据源模块（插件化的根）。
//!
//! 架构主轴（proposal/design D1-D3）：右 dock 的数据分两部分——
//! - **part1（本模块）**：从 workspace 定位根路径、读过滤配置、扫描文件
//!   系统、产出权威树并按规则过滤；右键功能的描述与执行也在此；
//! - **part2（`src/tree/` 面板）**：触发刷新、拉取 `view()`、把中立 DTO
//!   适配成 gpui 元素、渲染与分发。
//!
//! **插件化纪律**（design D3）：边界类型全部中立（不含 gpui 类型），
//! trait 为 dyn-safe，面板持 `Box<dyn WorkspacePropertySource>`；
//! 注册表 v1 静态（仅 filetree 一个条目），未来源切换器按 id 取用。

pub mod filetree;
pub mod filter;

pub use filetree::resolve_active_root;

use std::path::{Path, PathBuf};

use gpui_kit::{App, WeakEntity, Window};

use crate::app::TerminalApp;

/// 树节点种类。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceNodeKind {
    Dir,
    File,
}

/// 中立树节点 DTO——不含任何 gpui 类型，可跨插件边界（design D3）。
///
/// `id` = 绝对路径字符串（树内唯一，右键/左键载荷与复制路径共用它）。
#[derive(Clone, Debug, PartialEq)]
pub struct SourceNode {
    pub id: String,
    pub label: String,
    pub kind: SourceNodeKind,
    pub children: Vec<SourceNode>,
}

/// 右键菜单项描述——功能由数据源提供（proposal 定案）。
#[derive(Clone, Debug)]
pub struct MenuDesc {
    pub id: String,
    pub label: String,
}

/// 数据源执行菜单功能后返回的**效果意图**，由面板落地（design D2）：
/// 剪贴板要 `&mut Window`、reveal 要系统命令——源保持可单测、可跨边界。
#[derive(Clone, PartialEq, Debug)]
pub enum SourceEffect {
    CopyText(String),
    RevealPath(PathBuf),
}

/// `view()` 拉取的视图状态；`root` 携带当前根路径，面板用它做
/// "根路径变化才重扫" 的比对（file-tree spec: Tree scope）。
#[derive(Clone, Debug, PartialEq)]
pub enum SourceView {
    /// 无活跃 workspace（或尚未初始化）——面板显示空态。
    Empty,
    /// 根已定位但不可读——降级态（spec: Empty and degraded states）。
    Degraded { root: PathBuf, message: String },
    /// 过滤后的树；`node` 为合成根（其 children 即 workspace 顶层条目）；
    /// `rules` = 源当前生效的过滤规则（header 只读摘要用, spec: header chrome）。
    Tree {
        root: PathBuf,
        node: SourceNode,
        rules: Vec<String>,
    },
}

impl SourceView {
    /// 当前根路径（无则 None）——面板据此判断是否需要触发重扫。
    pub fn root(&self) -> Option<&Path> {
        match self {
            SourceView::Empty => None,
            SourceView::Degraded { root, .. } | SourceView::Tree { root, .. } => Some(root),
        }
    }
}

/// 数据源契约（dyn-safe，design D2）。
///
/// 六个方法即插件边界的全部；v1 面板持 `Box<dyn _>`，未来动态加载
/// 只需换注册表的构造方式，不改此契约。
pub trait WorkspacePropertySource {
    /// 数据源显示名（面板标题与注册表语义依据，如 "文件树"）。
    fn name(&self) -> &str;

    /// part1 入口：经 `WeakEntity<TerminalApp>` 定位活动 workspace 根 +
    /// 重读配置规则，然后**后台扫描**落权威树；完成经 app 的
    /// `notify_panels` 送达面板（file-tree spec: Refresh triggers）。
    /// 无活跃 workspace 时同步落入空态。
    fn refresh(&mut self, cx: &mut App);

    /// part2：面板拉取的当前视图（含过滤求值结果）。
    fn view(&self) -> SourceView;

    /// 右键菜单：功能描述由数据源提供。
    fn menu(&self, item_id: &str) -> Vec<MenuDesc>;

    /// 执行菜单功能，返回效果意图（面板落地）。
    fn execute(&mut self, item_id: &str, func_id: &str) -> Vec<SourceEffect>;

    /// 左键默认打开钩子 (center-slot-multi-tab: 接默认打开系统)——
    /// 数据源查权威树得 kind → 构造 `OpenObject` → `TerminalApp::`
    /// `open_object` 分发 (签名收 `window` 是 EditorState 构造所需;
    /// 效果仍由 app 层落地, 与 `SourceEffect` 分工一致)。
    fn open(&mut self, item_id: &str, window: &mut Window, cx: &mut App);
}

/// 在权威树中按绝对路径 id 查找节点种类 (open 钩子的 kind 依据)。
pub fn find_node_kind(node: &SourceNode, id: &str) -> Option<SourceNodeKind> {
    if node.id == id {
        return Some(node.kind);
    }
    node.children.iter().find_map(|child| find_node_kind(child, id))
}

/// 源工厂签名。
type SourceFactory = fn(WeakEntity<TerminalApp>, PathBuf) -> Box<dyn WorkspacePropertySource>;

/// 静态注册表（design: v1 仅 filetree；切换器立项后按 id 扩行）。
const REGISTRY: &[(&str, SourceFactory)] = &[(
    "filetree",
    |app, rules_path| Box::new(filetree::FileTreeSource::new(app, rules_path)),
)];

/// 注册表 id：默认数据源。
pub const DEFAULT_SOURCE_ID: &str = "filetree";

/// 按 id 取源；未知 id 返回 None。
pub fn create_source(
    id: &str,
    app: WeakEntity<TerminalApp>,
    rules_path: PathBuf,
) -> Option<Box<dyn WorkspacePropertySource>> {
    REGISTRY
        .iter()
        .find(|(registry_id, _)| *registry_id == id)
        .map(|(_, factory)| factory(app, rules_path))
}
