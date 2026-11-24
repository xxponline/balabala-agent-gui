//! Workspace 实体: 单实例工作区 (change: single-workspace-project-model)。
//!
//! 一个应用实例同时只打开一个 workspace (design D1): 目录本身即工作区,
//! Solution/Project 域状态归 solution 服务 (change solution-service 5.4:
//! 视图经服务镜像读), 会话按 Project 挂载。持久化分工: workspace 内文件
//! 见 `crate::project_model`, 全局历史见 `history`, legacy 迁移见 `store`。

pub mod history;
pub mod store;

use std::collections::HashSet;
use std::path::PathBuf;

/// 当前打开的工作区: 目录 + UI 折叠态。
///
/// 会话与 active 不在此存 — 一律读 AgentService 投影
/// (`sessions_view()`, change solution-overview); 所属 Project 由
/// `AgentEntity::project_root()` 即 cwd 决定 (spec: Session working
/// directory)。
pub struct Workspace {
    /// 工作区目录绝对路径 (去重身份; 右树扫描根)。
    pub path: PathBuf,
    /// 折叠的 Project 行 (易失, 不持久化 — 对齐"折叠态不持久化"立场;
    /// 纯 UI 层状态, 留本地 — 定案 2026-11-23)。
    pub collapsed_projects: HashSet<PathBuf>,
    /// 折叠的 Solution 行 (同上: 易失、不持久化)。
    ///
    /// 键 = `solution://<path>` 节点 id (change solution-overview D2 —
    /// 与 project 行的裸路径键不同命名空间, 不会撞)。
    pub collapsed_solutions: HashSet<String>,
}

impl Workspace {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            collapsed_projects: HashSet::new(),
            collapsed_solutions: HashSet::new(),
        }
    }
}
