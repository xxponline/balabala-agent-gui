//! 消息契约（design D1/D5）：kind 常量与载荷类型——GUI 与 `SolutionService`
//! 之间唯一的共享词汇。形态对齐 `agent_service::contract`：全部类型宿主同
//! 二进制（非 dylib 边界），wire 载荷经 `Payload::Typed(Arc<..>)` 零拷贝透传；
//! 仅 **solutions 文件契约**（`balabala.solutions.json`）走 serde（design D8）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

// ---------- 契约类标记 ----------

/// solution 契约类标记（对齐 `AgentContract`）：骨架侧"Solution 这一类服务"
/// 的身份判据，宿主以 `ContractId::of::<SolutionContract>()` 铸造。
pub struct SolutionContract;

// ---------- 配置与数据出口 ----------

/// 配置项缺省 = enabled（spec: legacy item without flags loads with
/// unchanged behavior — 旧文件缺字段时行为与迁移前一致）。
fn enabled_by_default() -> bool {
    true
}

/// 配置中的一个 solution item（文件契约，serde；spec: Solution
/// configuration file）。
///
/// 未知 `type` 在解析阶段被拒（spec: Solution load failures —— 归
/// [`LoadError::UnknownType`]）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SolutionItem {
    /// v1 唯一类型: 一个目录路径 → 一个 Project。
    Simple {
        path: PathBuf,
        /// 是否把该 solution 的 project 注册到资产侧（缺失 = true）。
        #[serde(default = "enabled_by_default")]
        register_to_asset_manager: bool,
        /// 是否启用 git 管理（口子；参与 `is_git` 解析，缺失 = true）。
        #[serde(default = "enabled_by_default")]
        git_enabled: bool,
    },
}

impl SolutionItem {
    /// item 的解决方案根目录（v1 simple 的 path；多类型后是身份键）。
    pub fn path(&self) -> &Path {
        match self {
            SolutionItem::Simple { path, .. } => path,
        }
    }

    /// 资产注册开关（spec: Asset registration flag gates project handoff）。
    pub fn register_to_asset_manager(&self) -> bool {
        match self {
            SolutionItem::Simple {
                register_to_asset_manager,
                ..
            } => *register_to_asset_manager,
        }
    }

    /// git 管理开关（spec: Git-managed resolution combines root and solution）。
    pub fn git_enabled(&self) -> bool {
        match self {
            SolutionItem::Simple { git_enabled, .. } => *git_enabled,
        }
    }

    /// 写入资产注册开关（configure 链路）。
    pub fn set_register_to_asset_manager(&mut self, v: bool) {
        match self {
            SolutionItem::Simple {
                register_to_asset_manager,
                ..
            } => *register_to_asset_manager = v,
        }
    }

    /// 写入 git 管理开关（configure 链路）。
    pub fn set_git_enabled(&mut self, v: bool) {
        match self {
            SolutionItem::Simple { git_enabled, .. } => *git_enabled = v,
        }
    }
}

/// Project 描述——**交树粒度 = Project 层**（design D2）：无 module/资产概念，
/// `is_git` 为解析产出（root 是 git 目录 AND solution.git_enabled，spec:
/// Git-managed resolution）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDesc {
    pub name: String,
    pub root: PathBuf,
    pub is_git: bool,
}

/// 项目树快照（交树/镜像载荷；事件自带快照，对齐 `SessionView` 纪律）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectTree {
    pub projects: Arc<[ProjectDesc]>,
}

/// 加载失败的结构化 reason（spec: Solution load failures surface as
/// structured errors——失败绝不携带部分树）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// solutions 文件读/解析失败（结构错误）。
    Parse(String),
    /// 未知 SolutionType（registry 中无此类型）。
    UnknownType(String),
    /// item 的 path 在磁盘上不存在。
    MissingPath(PathBuf),
    /// 项目依赖 / 模块引用成环（spec: DAG constraint；v1 展开无边，骨架在位）。
    Cycle(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Parse(m) => write!(f, "solution configuration invalid: {m}"),
            LoadError::UnknownType(t) => write!(f, "unknown solution type: {t}"),
            LoadError::MissingPath(p) => {
                write!(f, "solution path does not exist: {}", p.display())
            }
            LoadError::Cycle(m) => write!(f, "dependency cycle: {m}"),
        }
    }
}

impl std::error::Error for LoadError {}

// ---------- 控制（GUI → 服务，一律 ask） ----------

/// 加载工作区的 solutions（三个 load 边界共用此入口：启动 restore /
/// 手动切换 / 显式 reload）。
pub struct LoadReq {
    pub workspace_dir: PathBuf,
}

/// 取当前项目树快照（镜像补拉；无状态变更）。
pub struct GetTreeReq;

/// 创建 solution（append 一个 simple item 并落盘；幂等——同 path 已存在
/// 则原样返回，spec: Create and configure operations are idempotent）。
pub struct CreateReq {
    pub workspace_dir: PathBuf,
    pub path: PathBuf,
}

/// 配置 solution（`None` 字段 = 不改；同值重放 = no-op，spec: idempotent）。
pub struct ConfigureReq {
    pub workspace_dir: PathBuf,
    /// 目标 solution 身份（v1 = simple item 的 path）。
    pub solution_path: PathBuf,
    pub register_to_asset_manager: Option<bool>,
    pub git_enabled: Option<bool>,
}

// ---------- 回执（服务 → GUI） ----------

/// 树快照（ask 回执与 tell 事件的统一载荷）：
/// - `solutions`: solution 配置全量（视图经合同读 solution 状态，spec:
///   Solution and project state lives in the service）
/// - `tree`: project 全量集（视图读源——"solution 自己的 project 集"，
///   含注册门控关闭的项目，spec: Asset registration flag）
/// - `handoff`: 交树（`register_to_asset_manager` 门控后；资产注册的
///   唯一来源，spec: Asset registration flag gates project handoff）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeSnapshot {
    pub solutions: Arc<[SolutionItem]>,
    pub tree: ProjectTree,
    pub handoff: ProjectTree,
}

/// 操作失败（统一回执的错误侧）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpError {
    /// 加载失败的结构化 reason（spec: Solution load failures——失败绝不
    /// 携带/缓存部分树）。
    Load(LoadError),
    /// 目标 solution 不存在（configure 的幂等前置检失败）。
    NoSuch(PathBuf),
    /// 落盘失败（红条文案来源；内存变更已保留——workspace-management:
    /// Persistence write failure surfacing）。
    Persist(String),
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpError::Load(e) => write!(f, "{e}"),
            OpError::NoSuch(p) => write!(f, "no such solution: {}", p.display()),
            OpError::Persist(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for OpError {}

/// 控制回执：`Result<TreeSnapshot, OpError>`（Load 失败只有 reason，spec:
/// 绝不返回部分树）。
pub type OpResult = Result<TreeSnapshot, OpError>;

// ---------- 观察（服务 → GUI，tell） ----------

/// watch 握手载荷（订阅身份取信封 `from`，对齐 `agent_service::WatchReq`；
/// 首次订阅即推全量快照）。
pub struct WatchReq;

/// 项目树变更事件（事件自带快照；载荷粒度 = 整树——Open Questions 留档）。
#[derive(Debug, Clone)]
pub struct TreeChanged(pub TreeSnapshot);

// ---------- kind 常量（Envelope.kind） ----------

pub const KIND_LOAD: &str = "solution.load";
pub const KIND_GET_TREE: &str = "solution.get_tree";
pub const KIND_CREATE: &str = "solution.create";
pub const KIND_CONFIGURE: &str = "solution.configure";
pub const KIND_WATCH: &str = "solution.watch";
/// 控制 ask 的统一回执 kind（载荷 = `OpResult`；对齐 `KIND_CONTROL_REPLY`）。
pub const KIND_OP_REPLY: &str = "solution.reply";
pub const KIND_TREE_CHANGED: &str = "solution.tree_changed";

#[cfg(test)]
mod tests {
    use super::*;

    /// DTO serde 往返（task 1.1）：带配置字段的 item 编解码不丢字段。
    #[test]
    fn solution_item_serde_round_trip() {
        let item = SolutionItem::Simple {
            path: PathBuf::from("/a/foo"),
            register_to_asset_manager: false,
            git_enabled: true,
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: SolutionItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back, item, "serde 往返等值");
        assert_eq!(back.path(), Path::new("/a/foo"));
        assert!(!back.register_to_asset_manager());
        assert!(back.git_enabled());
    }

    /// ProjectDesc/ProjectTree 快照克隆等值（交树载荷形态）。
    #[test]
    fn project_tree_snapshot_eq() {
        let tree = ProjectTree {
            projects: Arc::from(vec![ProjectDesc {
                name: "foo".into(),
                root: PathBuf::from("/a/foo"),
                is_git: true,
            }]),
        };
        assert_eq!(tree.clone(), tree);
        assert_eq!(tree.projects.len(), 1);
    }

    /// LoadError 文案（错误对话框文案来源）。
    #[test]
    fn load_error_display() {
        assert_eq!(
            LoadError::UnknownType("fancy".into()).to_string(),
            "unknown solution type: fancy"
        );
        assert_eq!(
            LoadError::MissingPath(PathBuf::from("/gone")).to_string(),
            "solution path does not exist: /gone"
        );
    }
}
