//! 领域模型层: workspace 的 solution 配置 → Project/Module 结构。
//!
//! 单工作区模型 (change: single-workspace-project-model, design D1/D2):
//! - **纯数据层** — 不依赖 gpui, 可无头单测; 应用层只消费 [`WorkspaceModel`]。
//! - 组织逻辑归 Solution 实现: `balabala.solutions.json` 的 item 只带
//!   `SolutionType`, 展开成 Project 的规则在各 Solution 内部 (D1)。
//! - v1 只有 `simple` (path → 1 Project + 同名 root Module); 注册表 =
//!   `SolutionItem` 枚举 + `expand_item` match — 加新 type 就是加变体+臂。
//!
//! 行为契约: openspec/specs/project-model/spec.md。

pub mod files;
pub mod filter;
pub mod project;
pub mod solution;
pub mod validate;

pub use files::{
    MarkerStatus, append_simple_solution, create_workspace_files, marker_status,
};
pub use project::{Module, Project};
pub use solution::SolutionItem;
pub use validate::load;

use std::path::Path;

/// 一个已打开工作区的领域模型: solution 配置 + 展开后的 Project 列表。
///
/// 由 [`load`] 在加载边界 (启动恢复/切换/显式重载) 构造; 应用层
/// (`TerminalApp`) 持有其所有 Project 级运行时状态 (会话等)。
#[derive(Debug, Default, Clone)]
pub struct WorkspaceModel {
    /// 配置里的 solution items (原样保留 — append 写回时保序)。
    pub solutions: Vec<SolutionItem>,
    /// 展开出的 Project (v1: 每个 simple item 一个; DAG 校验已在 load 时通过)。
    pub projects: Vec<Project>,
}

impl WorkspaceModel {
    /// 按 root 路径找 Project (可变; 会话计数器领取 — `create_agent` 用,
    /// design D2: 身份 = root)。
    pub fn project_by_root_mut(&mut self, root: &Path) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| p.root == root)
    }
}
