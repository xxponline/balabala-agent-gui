//! Workspace entity: 一个 workspace 持有一个绝对路径, 以及该路径下的 agents。
//!
//! 这个模块只声明类型和 trait, 具体的持久化 IO 在 `store` 子模块。

pub mod store;

use std::path::PathBuf;

use crate::agent_entities::AgentEntity;

/// 一个工作区: 对应磁盘上的一个目录, 下面挂着若干 agent 会话。
pub struct Workspace {
    /// 绝对路径, 作为去重 key
    pub path: PathBuf,
    /// 折叠状态 (易失, 不持久化)
    pub collapsed: bool,
    /// 该 workspace 下的所有 agent 会话
    pub agents: Vec<Box<dyn AgentEntity>>,
    /// 下一个 agent 编号: 每个 workspace 独立从 1 开始
    pub next_agent_idx: u32,
}

impl Workspace {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            collapsed: false,
            agents: Vec::new(),
            next_agent_idx: 1,
        }
    }

    /// sidebar 行内显示的短名字: 路径末段 (e.g. "foo" for "/Users/me/code/foo")
    pub fn display_name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
    }

    /// tooltip 显示的完整绝对路径
    pub fn full_path_str(&self) -> String {
        self.path.to_string_lossy().to_string()
    }

    /// 生成下一个 agent 的显示名: "<dirname>/pi-<N>" 并自增计数器
    pub fn new_agent_name(&mut self) -> String {
        let name = format!("{}/pi-{}", self.display_name(), self.next_agent_idx);
        self.next_agent_idx += 1;
        name
    }
}
