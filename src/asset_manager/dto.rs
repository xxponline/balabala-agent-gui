//! asset_manager 的中立 DTO — 不含任何 gpui 类型, 可跨面板/服务边界。
//!
//! 由旧 `workspace_property_source` 的 DTO 家族改名迁入 (design D3)。
//! **身份体系不变**: `AssetNode::id` 对目录/文件仍是绝对路径字符串 —
//! center-tab dedup、复制路径、`OpenObject` key 全部免改。

use std::path::{Path, PathBuf};

/// 资产种类。
///
/// `Dir`/`File` 沿用旧语义; `Project`/`Module` 是本次新增的模型层行
/// (spec: 右树以已注册 Project 为根节点, 其下为 Module)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    /// Project 行 (分组头; id 带前缀, 见 [`project_id`])。
    Project,
    /// Module 行 (id = 模块根的绝对路径 — 兼容目录行的菜单/复制语义)。
    Module,
    Dir,
    File,
}

impl AssetKind {
    /// 可展开行 (folder 图标 + 折叠集合兜底): Project/Module/Dir。
    pub fn is_folder(self) -> bool {
        matches!(self, AssetKind::Project | AssetKind::Module | AssetKind::Dir)
    }

    /// 参与右键菜单/默认打开的行 (spec: file or directory row) —
    /// Project 是分组头, 不在其列。
    pub fn is_menu_row(self) -> bool {
        matches!(self, AssetKind::Module | AssetKind::Dir | AssetKind::File)
    }
}

/// 树节点 DTO。
///
/// `id` = 绝对路径字符串 (目录/文件, 树内唯一; 右键/左键载荷与复制路径
/// 共用它) — 唯一例外是 [`AssetKind::Project`] 行, 见 [`project_id`]。
#[derive(Clone, Debug, PartialEq)]
pub struct AssetNode {
    pub id: String,
    pub label: String,
    pub kind: AssetKind,
    pub children: Vec<AssetNode>,
}

/// Project 行的 id: `project://<绝对路径>`。
///
/// v1 的 Project 根与 root Module 根是同一目录 — 若两行都用裸路径会撞
/// id (树内唯一性 + 观测断言歧义)。Project 行是纯分组头 (无菜单/打开
/// 语义), 命名空间前缀只存在于行身份上; 一切路径语义仍由 Module/文件行
/// 的裸绝对路径承载 (design D3 的 id 稳定面)。
pub fn project_id(root: &Path) -> String {
    format!("project://{}", root.display())
}

/// 右键菜单项描述 — 功能由 asset_manager 提供 (proposal 定案)。
#[derive(Clone, Debug)]
pub struct AssetMenu {
    pub id: String,
    pub label: String,
}

/// manager 执行菜单功能后返回的**效果意图**, 由面板落地 (design D2):
/// 剪贴板要 `&mut Window`、reveal 要系统命令 — manager 保持可单测。
#[derive(Clone, PartialEq, Debug)]
pub enum AssetEffect {
    CopyText(String),
    RevealPath(PathBuf),
}

/// manager 暴露给面板的视图状态 (design D5: 三态 + 原子落地)。
#[derive(Clone, Debug, PartialEq)]
pub enum AssetView {
    /// 无工作区 / 零注册 — 面板显示空态。
    Empty,
    /// 模块根枚举失败 — 面板显示降级提示 (命名失败原因)。
    Degraded { message: String },
    /// 已过滤的完整森林 (一次落地; 过滤规则源 = 项目根 `.gitignore`,
    /// 不再随视图携带 — design D9, 头部摘要位一并撤除)。
    Forest { nodes: Vec<AssetNode> },
}

/// 在视图中按 id 找资产种类 (打开钩子的 kind 依据; 纯遍历)。
pub fn find_kind(view: &AssetView, id: &str) -> Option<AssetKind> {
    fn walk(node: &AssetNode, id: &str) -> Option<AssetKind> {
        if node.id == id {
            return Some(node.kind);
        }
        node.children.iter().find_map(|c| walk(c, id))
    }
    match view {
        AssetView::Forest { nodes, .. } => nodes.iter().find_map(|n| walk(n, id)),
        _ => None,
    }
}
