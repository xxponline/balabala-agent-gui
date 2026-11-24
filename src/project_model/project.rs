//! Project / Module 结构 (spec: Project and module structure shape)。
//! filter 供给归属在这里 (design D2/D8): Project 判定 git、读自身根
//! `.gitignore`、构建 filter 集并注入枚举时现建的 Module。

use std::fmt;
use std::path::{Path, PathBuf};

use asset_service::contract::{ModuleRegistration, ProjectRegistration};
use asset_service::enumerate::{Enumerator, EnumeratorOwner};

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

impl fmt::Debug for Module {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Module")
            .field("name", &self.name)
            .field("root", &self.root)
            .field("children", &self.children)
            .finish_non_exhaustive()
    }
}

/// 一个项目: 名字 + 根目录 + 模块树。
///
/// 身份 = `root` 路径 (design D2: solution-overview 事件载荷/会话挂载键)。
/// 会话命名 (`pi-N`) 真值在 AgentService (change agent-service 3.2,
/// 需求归属 agent-shell spec) — model 层不持计数 (change solution-service
/// 5.3 退役遗留锚点)。
#[derive(Clone)]
pub struct Project {
    /// 项目名 = root 目录末段 (display `<project>/pi-N`)。
    pub name: String,
    /// 项目根目录 (可在 workspace 目录之外, proposal)。
    pub root: PathBuf,
    /// root module 在首, name == project name (spec)。
    pub modules: Vec<Module>,
    /// 解析产出的 git-managed 标志 (change solution-service 4.2, spec:
    /// Filter provisioning — root 是 git 目录 AND solution.git_enabled;
    /// 运行时不再探测 `root/.git`)。
    pub is_git: bool,
}

impl Project {
    /// 由根目录路径构造 (默认 **非 git** — git 判定由调用方解析注入,
    /// 见 [`Self::from_desc`] / [`Self::from_root_with_git`])。
    /// 测试便捷构造 (生产走 `from_desc`)。
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_root(root: PathBuf) -> Self {
        Self::from_root_with_git(root, false)
    }

    /// 由根目录 + 解析后的 git 标志构造。
    pub fn from_root_with_git(root: PathBuf, is_git: bool) -> Self {
        let name = root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/".to_string());
        Self {
            modules: vec![Module::new(name.clone(), root.clone())],
            name,
            root,
            is_git,
        }
    }

    /// 交树描述 → Project (change solution-service: `is_git` 为解析产出,
    /// 随描述下发 — design D3)。
    pub fn from_desc(desc: &solution_service::ProjectDesc) -> Self {
        Self::from_root_with_git(desc.root.clone(), desc.is_git)
    }

    /// `Project` → 注册 DTO（change asset-service design D3 修订, task 2.6）。
    ///
    /// **纯字段映射 —— 不读文件、不构造过滤集**。过滤集由服务在每次
    /// 枚举时按 `is_git` + `root` 现场供给（`ProjectRegistration::
    /// provision_filters`），因此 ⟳ 天然重读规则；DTO 里也就没有
    /// `filters` 字段了（原「转换必须留本子树读 `pub(super)`」的约束
    /// 随之整条消失）。
    pub(crate) fn to_registration(&self) -> ProjectRegistration {
        ProjectRegistration {
            name: self.name.clone(),
            root: self.root.clone(),
            is_git: self.is_git,
            modules: self.modules.iter().cloned().map(module_to_reg).collect(),
        }
    }
}

/// `Module` → DTO（递归；纯结构）。
fn module_to_reg(m: Module) -> ModuleRegistration {
    ModuleRegistration {
        name: m.name,
        root: m.root,
        children: m.children.into_iter().map(module_to_reg).collect(),
    }
}

/// GUI 侧的 `Project` 仍可直接当枚举源装箱（`app.rs` 的 owner 装配点）——
/// 内部**先转 DTO 再枚举**，枚举实现只认 DTO（task 2.5/2.6）。
impl EnumeratorOwner for Project {
    fn root(&self) -> &Path {
        &self.root
    }

    fn enumerator(&self) -> Box<dyn Enumerator> {
        // 现算 DTO ⇒ 每次 `enumerator()` 都重读 .gitignore（刷新重供给）
        self.to_registration().enumerator()
    }
}

impl fmt::Debug for Project {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Project")
            .field("name", &self.name)
            .field("root", &self.root)
            .field("modules", &self.modules)
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

    /// task 2.6: DTO 不得丢字段 —— `Project`/`Module` 每个字段都要落到
    /// 注册 DTO，否则服务侧枚举出的树会与 GUI 侧认知不一致。
    #[test]
    fn registration_carries_every_project_and_module_field() {
        let mut p = Project::from_root("/Users/me/code/foo".into());
        p.is_git = true; // 单独验 is_git 不被掉（它只影响过滤供给）
        let reg = p.to_registration();

        // ---- Project 四字段 ----
        assert_eq!(reg.name, p.name, "name 进 DTO");
        assert_eq!(reg.root, p.root, "root 进 DTO");
        assert_eq!(reg.is_git, p.is_git, "is_git 进 DTO");
        assert_eq!(reg.modules.len(), p.modules.len(), "modules 进 DTO");

        // ---- Module 四字段 ----
        let m = &p.modules[0];
        let r = &reg.modules[0];
        assert_eq!(r.name, m.name, "Module.name 进 DTO");
        assert_eq!(r.root, m.root, "Module.root 进 DTO");
        assert_eq!(r.children.len(), m.children.len(), "Module.children 进 DTO");

        // ---- 过滤集**不进 DTO**（design D3 修订: 服务现场供给）----
        // 本测试只钉「结构字段无一遗漏」；过滤供给由
        // `provision_filters` 的四个 spec 场景测试覆盖。
    }

    /// spec: Module root mirrors the project root.
    #[test]
    fn root_module_mirrors_project_root() {
        let p = Project::from_root("/a/bar".into());
        assert_eq!(p.modules[0].root, PathBuf::from("/a/bar"));
    }

}
