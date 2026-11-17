//! Project / Module 结构 (spec: Project and module structure shape)。

use std::path::PathBuf;

/// 模块树节点。引用 (跨 module/project 的 DAG 边) 当前不产生
/// (v1 simple 单节点输出), 结构位留给后续 solution 类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    /// 模块名 (root module 与 Project 同名, spec)。
    pub name: String,
    /// 模块根目录 — 界定资产枚举范围
    /// (spec: Module root mirrors the project root)。
    pub root: PathBuf,
    /// 子模块 (树形; DAG 校验针对引用边, 见 `validate::check_dag`)。
    pub children: Vec<Module>,
}

impl Module {
    pub fn new(name: impl Into<String>, root: PathBuf) -> Self {
        Self {
            name: name.into(),
            root,
            children: Vec::new(),
        }
    }
}

/// 一个项目: 名字 + 根目录 + 模块树 + 运行时计数器。
///
/// 身份 = `root` 路径 (design D2: sidebar 事件载荷/会话挂载键)。
/// `next_agent_idx` 是会话命名计数 (design D8: per-project, 不持久化 —
/// spec: session naming starts at 1 per project and is never reused
/// *within* the open lifetime)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// 项目名 = root 目录末段 (display `<project>/pi-N`)。
    pub name: String,
    /// 项目根目录 (可在 workspace 目录之外, proposal)。
    pub root: PathBuf,
    /// root module 在首, name == project name (spec)。
    pub modules: Vec<Module>,
    /// 下一个会话编号 (从 1 起)。
    pub next_agent_idx: u32,
}

impl Project {
    /// 由根目录路径构造 (SimpleSolution 的展开路径): 名字取目录末段。
    pub fn from_root(root: PathBuf) -> Self {
        let name = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/".to_string());
        Self {
            modules: vec![Module::new(name.clone(), root.clone())],
            name,
            root,
            next_agent_idx: 1,
        }
    }

    /// 领取下一个会话显示名 `<project>/pi-N` 并自增 (spec: Project
    /// session creation — N 起 1, 同项目内不复用)。
    pub fn next_agent_name(&mut self) -> String {
        let name = format!("{}/pi-{}", self.name, self.next_agent_idx);
        self.next_agent_idx += 1;
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_module_shares_project_name() {
        let p = Project::from_root("/Users/me/code/foo".into());
        assert_eq!(p.name, "foo");
        assert_eq!(p.modules.len(), 1);
        assert_eq!(p.modules[0].name, p.name);
    }

    /// spec: Module root mirrors the project root.
    #[test]
    fn root_module_mirrors_project_root() {
        let p = Project::from_root("/a/bar".into());
        assert_eq!(p.modules[0].root, PathBuf::from("/a/bar"));
    }

    #[test]
    fn agent_names_are_per_project_and_never_reused() {
        let mut p = Project::from_root("/a/bar".into());
        assert_eq!(p.next_agent_name(), "bar/pi-1");
        assert_eq!(p.next_agent_name(), "bar/pi-2");
        assert_eq!(p.next_agent_idx, 3);
    }
}
