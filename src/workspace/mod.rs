//! Workspace 实体: 单实例工作区 (change: single-workspace-project-model)。
//!
//! 一个应用实例同时只打开一个 workspace (design D1): 目录本身即工作区,
//! 领域模型 (solutions → Projects) 持在其上, 会话按 Project 挂载。
//! 持久化分工: workspace 内文件见 `crate::project_model`, 全局历史见
//! `history`, legacy 迁移见 `store`。

pub mod history;
pub mod store;

use std::collections::HashSet;
use std::path::PathBuf;

use agent_service::contract::SessionId;

use crate::project_model::WorkspaceModel;

/// 当前打开的工作区: 目录 + 领域模型 + 会话。
///
/// 会话平铺在 workspace 级 (poll/主题/resize 遍历零分组开销); 所属
/// Project 由 `AgentEntity::project_root()` 即 cwd 决定 (spec: Session
/// working directory — agent 在哪个项目下 = cwd 指向哪个项目根)。
pub struct Workspace {
    /// 工作区目录绝对路径 (去重身份; 右树扫描根)。
    pub path: PathBuf,
    /// 领域模型 (solutions + Projects; 纯数据, design D2)。
    pub model: WorkspaceModel,
    /// 折叠的 Project 行 (易失, 不持久化 — 对齐"折叠态不持久化"立场)。
    pub collapsed_projects: HashSet<PathBuf>,
    /// 该工作区下的全部会话 id (平铺; 会话本体归 AgentService 所有,
    /// 此处仅 id 镜像 — change agent-service task 3.2, D4)。
    pub agents: Vec<SessionId>,
}

impl Workspace {
    pub fn new(path: PathBuf, model: WorkspaceModel) -> Self {
        Self {
            path,
            model,
            collapsed_projects: HashSet::new(),
            agents: Vec::new(),
        }
    }

    /// 行内短名: 路径末段 (e.g. "foo" for "/Users/me/code/foo")。
    pub fn display_name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
    }

    /// tooltip 完整路径。
    pub fn full_path_str(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}
