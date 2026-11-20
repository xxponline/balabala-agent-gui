//! Project / Module 结构 (spec: Project and module structure shape)。
//! filter 供给归属在这里 (design D2/D8): Project 判定 git、读自身根
//! `.gitignore`、构建 filter 集并注入枚举时现建的 Module。

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use super::filter::{DotFilter, FileFilter, GitIgnoreFilter};

/// 模块树节点。引用 (跨 module/project 的 DAG 边) 当前不产生
/// (v1 simple 单节点输出), 结构位留给后续 solution 类型。
///
/// `Clone` 经 `Arc` 保住 (manager 把 Project 克隆进 bg spawn —
/// design D2 的承重墙); `Debug` 手写、`PartialEq/Eq` 已摘 (filter
/// 是 trait object, 全量验证无使用点)。
#[derive(Clone)]
pub struct Module {
    /// 模块名 (root module 与 Project 同名, spec)。
    pub name: String,
    /// 模块根目录 — 界定资产枚举范围
    /// (spec: Module root mirrors the project root)。
    pub root: PathBuf,
    /// 子模块 (树形; DAG 校验针对引用边, 见 `validate::check_dag`)。
    pub children: Vec<Module>,
    /// 过滤集 (design D2 注入点): Project 枚举时构建 — 默认
    /// [`DotFilter`] + 项目根 git 时的 [`GitIgnoreFilter`] (D8)。
    /// `pub(super)`: project_model 子树内可见 (enumerate 消费),
    /// 资产侧 (`crate::assets`) 不可见 — 注入只发生在构建期。
    pub(super) filters: Vec<Arc<dyn FileFilter>>,
}

impl Module {
    pub fn new(name: impl Into<String>, root: PathBuf) -> Self {
        Self {
            name: name.into(),
            root,
            children: Vec::new(),
            filters: Vec::new(),
        }
    }
}

impl fmt::Debug for Module {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Module")
            .field("name", &self.name)
            .field("root", &self.root)
            .field("children", &self.children)
            .finish_non_exhaustive()
    }
}

/// 一个项目: 名字 + 根目录 + 模块树 + 运行时计数器。
///
/// 身份 = `root` 路径 (design D2: sidebar 事件载荷/会话挂载键)。
/// `next_agent_idx` 是会话命名计数 (design D8: per-project, 不持久化 —
/// spec: session naming starts at 1 per project and is never reused
/// *within* the open lifetime)。
#[derive(Clone)]
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
    ///
    /// change agent-service 3.2: 命名真值已移入 AgentService (服务侧
    /// counters); 本方法保留为 model 层遗留单测锚 (生产不再调用)。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn next_agent_name(&mut self) -> String {
        let name = format!("{}/pi-{}", self.name, self.next_agent_idx);
        self.next_agent_idx += 1;
        name
    }

    /// 供给 filter 集 (design D8; spec: Filter provisioning from the
    /// project root): 恒含默认 [`DotFilter`];仅当 `root/.git` 存在时
    /// 读 `root/.gitignore` 追加 [`GitIgnoreFilter`] — 非 git 根不
    /// 探测任何 ignore 文件, 读失败 best-effort 跳过。
    fn provision_filters(&self) -> Vec<Arc<dyn FileFilter>> {
        let mut filters: Vec<Arc<dyn FileFilter>> = vec![Arc::new(DotFilter)];
        if self.root.join(".git").exists() {
            if let Ok(text) = std::fs::read_to_string(self.root.join(".gitignore")) {
                filters.push(Arc::new(GitIgnoreFilter::from_text(&text)));
            }
        }
        filters
    }

    /// 枚举时调用 (design D2): 判定 git、读 `.gitignore`、构建 filter
    /// 集并**注入现建的 Module 副本**。IO 在调用方的 bg task 内;
    /// 每次枚举重建 → 刷新天然重读 (spec: Refresh re-reads filter rules)。
    pub(super) fn modules_for_enumeration(&self) -> Vec<Module> {
        let filters = self.provision_filters();
        self.modules
            .iter()
            .cloned()
            .map(|mut module| {
                module.filters = filters.clone();
                module
            })
            .collect()
    }
}

impl fmt::Debug for Project {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Project")
            .field("name", &self.name)
            .field("root", &self.root)
            .field("modules", &self.modules)
            .field("next_agent_idx", &self.next_agent_idx)
            .finish_non_exhaustive()
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

    // ===== filter 供给 (design D8; spec: Filter provisioning) =====

    /// 每测试独立 temp 根 (进程 id + 名字, 平行安全)。
    fn temp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "provision-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// spec: Git-managed project applies its root gitignore —
    /// `[Dot, GitIgnore]` 供给并注入枚举时现建的 Module。
    #[test]
    fn git_project_provisions_dot_and_gitignore() {
        let root = temp_root("git");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "*.tmp\n").unwrap();
        std::fs::write(root.join("cache.tmp"), "x").unwrap();

        let p = Project::from_root(root.clone());
        let filters = p.provision_filters();
        // 行为对 (替代计数断言): Dot 命中 .env + gitignore 命中 cache.tmp
        // → 两种规则都在且都生效。
        assert!(
            filters.iter().any(|f| f.is_ignored(".env", false)),
            "默认 Dot 在列"
        );
        assert!(
            filters.iter().any(|f| f.is_ignored("cache.tmp", false)),
            "根 .gitignore 生效 (spec 场景: cache.tmp 不出现)"
        );

        let modules = p.modules_for_enumeration();
        assert_eq!(modules.len(), 1, "现建 Module");
        assert!(
            modules[0].filters.iter().any(|f| f.is_ignored("cache.tmp", false)),
            "注入的 filter 对条目生效"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// spec: Non-git project does not read its gitignore — 非 git 根不
    /// 探测, 只供默认 Dot。
    #[test]
    fn non_git_project_reads_nothing_but_dot() {
        let root = temp_root("non-git");
        std::fs::write(root.join(".gitignore"), "*.tmp\n").unwrap();
        std::fs::write(root.join("cache.tmp"), "x").unwrap();

        let p = Project::from_root(root.clone());
        let filters = p.provision_filters();
        // 行为对 (替代计数断言): Dot 命中 + gitignore 规则不命中 → 仅 Dot。
        assert!(
            filters.iter().any(|f| f.is_ignored(".env", false)),
            "默认 Dot 在列"
        );
        assert!(
            !filters.iter().any(|f| f.is_ignored("cache.tmp", false)),
            "非 git 根的 .gitignore 不生效 (spec 场景: cache.tmp 仍出现)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// git 项目但无 `.gitignore` → 只挂 Dot (读取 best-effort)。
    #[test]
    fn git_project_without_gitignore_only_dot() {
        let root = temp_root("git-empty");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("notes.log"), "x").unwrap();

        let p = Project::from_root(root.clone());
        let filters = p.provision_filters();
        // 行为对 (替代计数断言): Dot 命中 .env + 无 gitignore 规则命中。
        assert!(filters.iter().any(|f| f.is_ignored(".env", false)));
        assert!(
            !filters.iter().any(|f| f.is_ignored("notes.log", false)),
            "无 .gitignore → 无 gitignore 规则命中 (仅默认 Dot)"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Clone 链 (manager spawn 路径): Module 克隆后 filter 同源 (Arc)。
    #[test]
    fn module_clone_keeps_filters_via_arc() {
        let root = temp_root("clone");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();

        let p = Project::from_root(root.clone());
        let modules = p.modules_for_enumeration();
        let cloned = modules[0].clone();
        assert_eq!(cloned.filters.len(), modules[0].filters.len());
        assert!(
            std::sync::Arc::ptr_eq(&cloned.filters[0], &modules[0].filters[0]),
            "克隆共享同一 filter 实例 (Arc 引用计数)"
        );
        let _ = p.clone(); // Project: Clone 不回归
        let _ = std::fs::remove_dir_all(&root);
    }
}
