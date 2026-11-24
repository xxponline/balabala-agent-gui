//! 消息契约（design D1/D5）：kind 常量与载荷类型——GUI 与 `SolutionService`
//! 之间唯一的共享词汇。形态对齐 `agent_service::contract`：全部类型宿主同
//! 二进制（非 dylib 边界），wire 载荷经 `Payload::Typed(Arc<..>)` 零拷贝透传；
//! 仅 **solutions 文件契约**（`balabala.solutions.json`）走 serde（design D8）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use service_core::Payload;

// 节点动作描述符归中立命名空间 (change asset-service design D6) ——
// solution 与 asset 两侧共用同一类型, 服务之间不互相依赖。
pub use contracts::shared::{ActionDesc, ExecutedBy};

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

    /// 类型标记（serde `type` 字段同值）—— 视图用它区分 solution 行与
    /// 下方同名 project 行（change solution-overview D2: v1 两者 path
    /// 相同, label 会一模一样, 类型标记是唯一视觉区分）。
    pub fn type_name(&self) -> &'static str {
        match self {
            SolutionItem::Simple { .. } => "simple",
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

/// 按 action_id + 当前值构造 invoke 的 `args`（change solution-overview
/// design D4 修订 — 甲）。
///
/// 视图在**点击时**由描述符的 `checked` 取反得到目标值：不预构建
/// `Payload`（D3：预构建会把快照期的值冻结），也不让视图逐字段拼
/// `ConfigureReq`（形状知识留在契约）。
///
/// 返回 `None` = 非开关动作或当前值未知 → invoke 不带 `args`。
pub fn configure_action_args(action_id: &str, checked: Option<bool>) -> Option<ConfigureReq> {
    let current = checked?;
    match action_id {
        ACTION_CONFIGURE_REGISTER => Some(ConfigureReq {
            register_to_asset_manager: Some(!current),
            git_enabled: None,
        }),
        ACTION_CONFIGURE_GIT => Some(ConfigureReq {
            register_to_asset_manager: None,
            git_enabled: Some(!current),
        }),
        _ => None,
    }
}

/// 动作 id 常量 — 描述符声明与服务端 `match` 分派共用同一词汇
/// （change solution-overview design D6）。
pub const ACTION_CREATE: &str = "create";
pub const ACTION_CONFIGURE_REGISTER: &str = "configure.register_to_asset_manager";
pub const ACTION_CONFIGURE_GIT: &str = "configure.git_enabled";

/// solution 节点声明的可用操作（design D3: 描述符随快照下传）。
///
/// 两个开关的 `checked` 取自该 item 的当前配置值 — **当前值只有服务
/// 知道**, 视图只负责渲染勾选态。
pub fn solution_actions(item: &SolutionItem) -> Arc<[ActionDesc]> {
    Arc::from(vec![
        ActionDesc {
            id: ACTION_CONFIGURE_REGISTER.to_string(),
            label: "注册到资产树".to_string(),
            checked: Some(item.register_to_asset_manager()),
            execution: ExecutedBy::Service,
        },
        ActionDesc {
            id: ACTION_CONFIGURE_GIT.to_string(),
            label: "git 管理".to_string(),
            checked: Some(item.git_enabled()),
            execution: ExecutedBy::Service,
        },
    ])
}

/// solution 节点 id 前缀 — 与 asset 树 `project_id()` 同构的命名空间方案
/// （change solution-overview D2）：v1 的 solution path 与 project root
/// **是同一目录**, 裸路径会让两行撞 id（展开集/右键查找/观测锚点全坏）。
pub const SOLUTION_NODE_PREFIX: &str = "solution://";

/// 构造 solution 节点 id（视图渲染与服务回指同一函数, 保证往返一致）。
pub fn solution_node_id(path: &Path) -> String {
    format!("{SOLUTION_NODE_PREFIX}{}", path.display())
}

/// 解析 solution 节点 id → 目标 path。
///
/// `None` = id 缺失或前缀不对 → **接收方契约 bug**, 调用方 `reject` 落
/// 死信（design D4; 与 `Payload::downcast` 失败的 `TypeMismatch` 同类）。
/// 前缀对但 solution 不存在, 由调用方转 `OpError::NoSuch`（用户可见红条）。
pub fn parse_solution_node(node: Option<&str>) -> Option<PathBuf> {
    node?.strip_prefix(SOLUTION_NODE_PREFIX).map(PathBuf::from)
}

/// project 层节点（hierarchy 第二层）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectNode {
    pub desc: ProjectDesc,
    /// 服务为该节点声明的操作（design D3）。
    pub actions: Arc<[ActionDesc]>,
}

/// solution 层节点（hierarchy 根层）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolutionNode {
    /// 该 solution 的配置项（文件 DTO 原样 — 视图读 solution 状态）。
    pub item: SolutionItem,
    /// 该 solution 解析出的 project 节点。
    pub projects: Arc<[ProjectNode]>,
    /// 服务为该节点声明的操作（design D3）。
    pub actions: Arc<[ActionDesc]>,
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

/// 加载工作区的 solutions（打开工作区的唯一入口；同时**绑定**该工作区的
/// 目录 — 之后的 create/configure 落盘都用它，design D5）。
pub struct LoadReq {
    pub workspace_dir: PathBuf,
}

/// 取当前项目树快照（镜像补拉；无状态变更）。
pub struct GetTreeReq;

/// 创建 solution（append 一个 simple item 并落盘；幂等——同 path 已存在
/// 则原样返回，spec: Create and configure operations are idempotent）。
///
/// 不带 `workspace_dir` — 落盘目标由服务持有的绑定决定（design D5），
/// 而非每个请求重申一遍。
pub struct CreateReq {
    pub path: PathBuf,
}

/// 配置 solution（`None` 字段 = 不改；同值重放 = no-op，spec: idempotent）。
///
/// 不带 `workspace_dir` — 同 [`CreateReq`]，目标目录单源在服务侧（D5）。
pub struct ConfigureReq {
    pub register_to_asset_manager: Option<bool>,
    pub git_enabled: Option<bool>,
}

/// 通用动作调用 — **写入口单点**（change solution-overview design D6）。
///
/// `KIND_CREATE` / `KIND_CONFIGURE` 退场后, 所有用户动作都经此进服务,
/// 由服务按 `action_id` 内部分派。代价是边界上放弃 per-action 类型安全 —
/// 类型安全移到服务内部 `match`; 未知 id 走 `reject` 落死信, 与
/// `Payload::downcast` 失败的 `TypeMismatch` 同语义。
#[derive(Debug, Clone)]
pub struct ActionInvoke {
    /// 目标节点 id（`solution://<path>`）; `None` = workspace 级动作
    /// （如 create — 无目标节点, 且服务已持有绑定目录, design D4）。
    pub node: Option<String>,
    /// 动作 id — 服务端 `match` 的分派键, 同时是回执路由键（design D7）。
    pub action_id: String,
    /// 动作参数; 无参动作为 `None`。
    ///
    /// **不进描述符** — `Payload` 无 `PartialEq`/`Default`, 挂进
    /// `TreeSnapshot` 会连带断掉三个 derive（design D3）; 描述符只携带
    /// 渲染所需的 `id`/`label`/`checked`。
    pub args: Option<Payload>,
}

// ---------- 回执（服务 → GUI） ----------

/// 树快照（ask 回执与 tell 事件的统一载荷）—— **`hierarchy` 是唯一
/// 展开数据（view model）**，`tree` / `handoff` 由它派生（change
/// solution-overview design D1）：
/// - `solutions`: solution 配置全量（文件 DTO 原样；视图经合同读 solution
///   状态，spec: Solution and project state lives in the service）
/// - `hierarchy`: solution → project 的嵌套展开，每个节点携带服务声明的
///   `actions`
///
/// 派生方法 [`TreeSnapshot::tree`]（展平全量 project）与
/// [`TreeSnapshot::handoff`]（过滤注册开关）取代原先并列的两个字段 —
/// 并列字段意味着同一批数据有多个来源, 正是本 change 要拆的双源。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeSnapshot {
    pub solutions: Arc<[SolutionItem]>,
    pub hierarchy: Arc<[SolutionNode]>,
}

impl TreeSnapshot {
    /// 派生: 展平全部 project（含注册开关关闭的）— 视图读源。
    pub fn tree(&self) -> ProjectTree {
        ProjectTree {
            projects: self
                .hierarchy
                .iter()
                .flat_map(|s| s.projects.iter())
                .map(|p| p.desc.clone())
                .collect::<Vec<_>>()
                .into(),
        }
    }

    /// 派生: 交树（只含 `register_to_asset_manager` 开启的 solution 的
    /// project）— 资产注册的唯一来源（spec: Asset registration flag gates
    /// project handoff）。
    pub fn handoff(&self) -> ProjectTree {
        ProjectTree {
            projects: self
                .hierarchy
                .iter()
                .filter(|s| s.item.register_to_asset_manager())
                .flat_map(|s| s.projects.iter())
                .map(|p| p.desc.clone())
                .collect::<Vec<_>>()
                .into(),
        }
    }
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
    /// 调用方发了一个服务**未声明**的动作 id — 消息看得懂、只是操作不
    /// 支持。**立即回执**而非 `reject`（实施期定案 C, design D6）：
    /// reject 不回信封, 调用方要等 `DEFAULT_CALL_TIMEOUT = 5s` 才拿到
    /// `RouteError::Timeout`, notice 只写 "ask timeout" — 既慢又不点名。
    /// 与 node 格式坏的 `reject` 分流（那条是真的无法解释目标）。
    UnknownAction(String),
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpError::Load(e) => write!(f, "{e}"),
            OpError::NoSuch(p) => write!(f, "no such solution: {}", p.display()),
            OpError::Persist(m) => write!(f, "{m}"),
            OpError::UnknownAction(a) => write!(f, "unknown action: {a}"),
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
/// **写入口单点**（change solution-overview design D6）：所有用户动作
/// （create / configure…）都经此进服务, 服务按 `action_id` 内部分派。
/// `solution.create` / `solution.configure` 两个 kind 已退场。
pub const KIND_ACTION: &str = "solution.action";
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

    /// 1.1: `tree()` 派生 = 原平表（展平全部 project，含注册关闭的）。
    ///
    /// 原 `snapshot()` 手工维护 `tree`/`handoff` 两份投影；改 hierarchy
    /// 单源后这两份必须由派生复现同一结果（change solution-overview D1）。
    #[test]
    fn tree_derives_the_old_flat_table() {
        let on = SolutionItem::Simple {
            path: PathBuf::from("/a/on"),
            register_to_asset_manager: true,
            git_enabled: true,
        };
        let off = SolutionItem::Simple {
            path: PathBuf::from("/a/off"),
            register_to_asset_manager: false,
            git_enabled: true,
        };
        let snap = snapshot_of(vec![on.clone(), off.clone()]);

        let derived: Vec<PathBuf> = snap
            .tree()
            .projects
            .iter()
            .map(|p| p.root.clone())
            .collect();
        // 原平表语义: 每个 item 一个 project, 顺序 = items 顺序, 不论注册开关
        let old_flat = vec![on.path().to_path_buf(), off.path().to_path_buf()];
        assert_eq!(derived, old_flat, "tree() 与原平表等价");
    }

    /// 1.1: `handoff()` 派生 = 注册门控后的交树（spec: Asset registration
    /// flag gates project handoff）；全量集仍保留注册关闭的 project。
    #[test]
    fn handoff_derives_the_gated_projection() {
        let snap = snapshot_of(vec![
            SolutionItem::Simple {
                path: PathBuf::from("/a/on"),
                register_to_asset_manager: true,
                git_enabled: true,
            },
            SolutionItem::Simple {
                path: PathBuf::from("/a/off"),
                register_to_asset_manager: false,
                git_enabled: true,
            },
        ]);

        let gated: Vec<PathBuf> = snap
            .handoff()
            .projects
            .iter()
            .map(|p| p.root.clone())
            .collect();
        assert_eq!(gated, vec![PathBuf::from("/a/on")], "交树只含注册开启的");
        assert_eq!(
            snap.tree().projects.len(),
            2,
            "全量集保留注册关闭的 project (spec)"
        );
    }

    /// 2.2: `solution_actions()` 为 solution 节点声明两个开关，
    /// `checked` 取 item 当前值（design D3: 当前值只有服务知道）。
    #[test]
    fn solution_actions_describe_the_two_config_toggles() {
        let on = SolutionItem::Simple {
            path: PathBuf::from("/a/foo"),
            register_to_asset_manager: true,
            git_enabled: false,
        };
        let acts = solution_actions(&on);
        assert_eq!(acts.len(), 2, "两个配置开关");
        assert_eq!(acts[0].id, ACTION_CONFIGURE_REGISTER);
        assert_eq!(acts[0].label, "注册到资产树", "label 归服务所有");
        assert_eq!(acts[0].checked, Some(true), "checked 取当前值");
        assert_eq!(acts[1].id, ACTION_CONFIGURE_GIT);
        assert_eq!(acts[1].label, "git 管理", "label 归服务所有");
        assert_eq!(acts[1].checked, Some(false), "checked 取当前值");
    }

    /// change asset-service 1.3: solution 动作必须显式标 `Service`（
    /// design D6 —— 配置写入口在服务, 服务执行 ⇒ 视图发 invoke）。
    /// 缺省值不存在, 漏标是编译错（见 `contracts::shared::ActionDesc`
    /// 的 `compile_fail` doctest）。
    #[test]
    fn solution_actions_declare_service_execution() {
        let item = SolutionItem::Simple {
            path: PathBuf::from("/a/foo"),
            register_to_asset_manager: true,
            git_enabled: true,
        };
        let acts = solution_actions(&item);
        assert_eq!(acts.len(), 2);
        for a in acts.iter() {
            assert_eq!(
                a.execution,
                ExecutedBy::Service,
                "{} 必须标为服务执行 (无缺省值)",
                a.id
            );
        }
        assert_ne!(acts[0].execution, ExecutedBy::View, "反例自检");
    }

    /// 2.2: 快照节点携带描述符（含 label）— 右键菜单零往返的来源。
    #[test]
    fn snapshot_nodes_carry_action_descriptors() {
        let snap = snapshot_of(vec![SolutionItem::Simple {
            path: PathBuf::from("/a/foo"),
            register_to_asset_manager: true,
            git_enabled: true,
        }]);
        let root = &snap.hierarchy[0];
        let ids: Vec<&str> = root.actions.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![ACTION_CONFIGURE_REGISTER, ACTION_CONFIGURE_GIT],
            "solution 节点声明两个动作"
        );
        let labels: Vec<&str> = root.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, vec!["注册到资产树", "git 管理"], "描述符携带 label");
        assert!(
            root.projects[0].actions.is_empty(),
            "project 节点暂无声明 (create worktree 属后续)"
        );
        assert_eq!(
            root.item.path(),
            std::path::Path::new("/a/foo"),
            "item 原样保留在节点上"
        );
    }

    /// 由 item 列表搭一份 hierarchy（与 `snapshot()` 同构）。
    fn snapshot_of(items: Vec<SolutionItem>) -> TreeSnapshot {
        let hierarchy: Arc<[SolutionNode]> = items
            .iter()
            .map(|item| SolutionNode {
                item: item.clone(),
                projects: Arc::from(vec![ProjectNode {
                    desc: ProjectDesc {
                        name: item
                            .path()
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        root: item.path().to_path_buf(),
                        is_git: false,
                    },
                    actions: Arc::from([]),
                }]),
                actions: solution_actions(item),
            })
            .collect::<Vec<_>>()
            .into();
        TreeSnapshot {
            solutions: items.into(),
            hierarchy,
        }
    }
}
