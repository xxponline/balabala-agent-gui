//! 消息契约（design D1/D5）：kind 常量与载荷类型——GUI 与 `SolutionService`
//! 之间唯一的共享词汇。形态对齐 `agent_service::contract`：全部类型宿主同
//! 二进制（非 dylib 边界），wire 载荷经 `Payload::Typed(Arc<..>)` 零拷贝透传；
//! 仅 **solutions 文件契约**（`balabala.solutions.json`）走 serde（design D8）。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use service_core::Payload;

// 节点动作描述符归中立命名空间 (change asset-service design D6) ——
// solution 与 asset 两侧共用同一类型, 服务之间不互相依赖。
pub use contracts::shared::{ActionChild, ActionDesc, ExecutedBy};

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

/// 视图点击时按 `action_id` 构造 invoke 的 typed args（change
/// solution-git-worktree design D6: 形状知识留在契约, 视图只管一行调用）：
/// - 创建 worktree → 默认名 `<project>-<epoch毫秒>`（change
///   solution-git-worktree OQ2: 表单未就绪的过渡, 见
///   [`default_create_worktree_args`]）；
/// - configure 两开关 → 目标态反值（[`configure_action_args`]）；
/// - 其余（拉取/推送/删除 worktree）无 args。
pub fn action_args(action_id: &str, node: &str, checked: Option<bool>) -> Option<Payload> {
    match action_id {
        ACTION_CREATE_WORKTREE => default_create_worktree_args(node).map(Payload::new),
        _ => configure_action_args(action_id, checked).map(Payload::new),
    }
}

/// 创建 worktree 的默认 args（change solution-git-worktree OQ2 过渡）：
/// `<project目录名>-<epoch毫秒>`，project 段过滤成**分支名安全**字符集。
///
/// `git worktree add <path>` 按目录 basename 自建分支 —— 空格、`..`、
/// 前导 `-`/`.` 等「目录名合法但分支名非法」的字符会让 git 直接 fatal
/// （6.2 冒烟前置实测；服务端 `sanitize_name` 只管目录安全, 这里补分支
/// 名面）。project 段取 `project://` 节点的 `file_name()`（路径可能无
/// file_name → 兜底 `worktree`）。节点非 `project://` → `None`
/// （服务端 parse 同样会拒, 名字用不上）。
pub fn default_create_worktree_args(node: &str) -> Option<CreateWorktreeReq> {
    let root = parse_project_node(Some(node))?;
    let project = root
        .file_name()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    Some(CreateWorktreeReq {
        name: format!("{}-{ts}", branch_safe_segment(&project)),
    })
}

/// project 段 → 分支名安全片段：保留字母数字（含中文等 Unicode 项目名）
/// 与 `_`、`-`、`.`；其余（空格、`:*?"<>|`、控制符…）折 `-`；折叠连续
/// `-` 与 `..`；不以 `-`/`.` 开头；修剪尾部 `-`/`.`；全被滤掉 → `worktree`。
/// 合法性以真 `git check-ref-format` 为准（契约测试裁决, 不自写规则）。
fn branch_safe_segment(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    let mut last_dot = false;
    for c in raw.chars() {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
            last_dash = false;
            last_dot = false;
        } else if c == '-' {
            if last_dash || out.is_empty() {
                continue;
            }
            out.push('-');
            last_dash = true;
            last_dot = false;
        } else if c == '.' {
            if last_dot || out.is_empty() {
                continue;
            }
            out.push('.');
            last_dot = true;
            last_dash = false;
        } else {
            // 非法字符 → 折 `-`（首字符非法时直接丢, 不产出前导 `-`）
            if last_dash || out.is_empty() {
                continue;
            }
            out.push('-');
            last_dash = true;
            last_dot = false;
        }
    }
    let trimmed = out.trim_end_matches(|c| c == '-' || c == '.');
    if trimmed.is_empty() {
        "worktree".into()
    } else {
        trimmed.into()
    }
}

/// 动作 id 常量 — 描述符声明与服务端 `match` 分派共用同一词汇
/// （change solution-overview design D6）。
pub const ACTION_CREATE: &str = "create";
pub const ACTION_CONFIGURE_REGISTER: &str = "configure.register_to_asset_manager";
pub const ACTION_CONFIGURE_GIT: &str = "configure.git_enabled";
/// git 操作四个动作 id（change solution-git-worktree, design D5）：
/// 描述符声明与服务端 `match` 分派共用同一词汇。
pub const ACTION_PULL: &str = "git.pull";
pub const ACTION_PUSH: &str = "git.push";
pub const ACTION_CREATE_WORKTREE: &str = "worktree.create";
pub const ACTION_DELETE_WORKTREE: &str = "worktree.delete";
/// 双向同步两个动作 id（change solution-git-sync design D1）：前者挂
/// worktree 行（project HEAD 合入 worktree）, 后者挂 project 行子菜单
/// （worktree HEAD 快进 project）。
pub const ACTION_SYNC_WORKTREE_FROM_PROJECT: &str = "worktree.sync_from_project";
pub const ACTION_SYNC_PROJECT_FROM_WORKTREE: &str = "project.sync_from_worktree";

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
            children: Arc::from([]),
        },
        ActionDesc {
            id: ACTION_CONFIGURE_GIT.to_string(),
            label: "git 管理".to_string(),
            checked: Some(item.git_enabled()),
            execution: ExecutedBy::Service,
            children: Arc::from([]),
        },
    ])
}

/// project 节点声明的可用操作（恒声明, design D5 方案 b）：
/// **不**按 is_git/干净态过滤 —— 快照现算拦不住 invoke 前的状态变化
/// （陈旧），门控在服务端 invoke 时做, 不过 → `OpError` → 既有 notice。
///
/// change solution-git-sync (design D3): worktree 层**非空**时追加
/// "从 worktree 同步" 父描述符, children = 每 worktree 一条 —— 与
/// 下方 worktree 行同源同涨落（层空 = 无目标, 不声明）。
pub fn project_actions(worktrees: &[WorktreeNode]) -> Arc<[ActionDesc]> {
    const OPS: [(&str, &str); 3] = [
        (ACTION_PULL, "拉取"),
        (ACTION_PUSH, "推送"),
        (ACTION_CREATE_WORKTREE, "创建 worktree"),
    ];
    let mut acts: Vec<ActionDesc> = OPS
        .into_iter()
        .map(|(id, label)| ActionDesc {
            id: id.to_string(),
            label: label.to_string(),
            checked: None, // 非开关
            execution: ExecutedBy::Service,
            children: Arc::from([]),
        })
        .collect();
    if !worktrees.is_empty() {
        acts.push(ActionDesc {
            id: ACTION_SYNC_PROJECT_FROM_WORKTREE.to_string(),
            label: "从 worktree 同步".to_string(),
            checked: None,
            execution: ExecutedBy::Service,
            children: worktrees
                .iter()
                .map(|w| ActionChild {
                    label: worktree_display_name(w),
                    node: w.path.to_string_lossy().into_owned(),
                })
                .collect::<Vec<_>>()
                .into(),
        });
    }
    acts.into()
}

/// worktree 显示名：分支名, detached → 目录末段（与视图行推导同规则 ——
/// 子项 label 必须与行一致, change solution-git-sync design D3）。
pub fn worktree_display_name(w: &WorktreeNode) -> String {
    w.branch.clone().unwrap_or_else(|| {
        w.path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    })
}

/// worktree 节点声明的可用操作（恒声明, 同 `project_actions` 立场）。
pub fn worktree_actions() -> Arc<[ActionDesc]> {
    Arc::from(vec![
        ActionDesc {
            id: ACTION_DELETE_WORKTREE.to_string(),
            label: "删除 worktree".to_string(),
            checked: None,
            execution: ExecutedBy::Service,
            children: Arc::from([]),
        },
        ActionDesc {
            id: ACTION_SYNC_WORKTREE_FROM_PROJECT.to_string(),
            label: "同步 Project 代码".to_string(),
            checked: None,
            execution: ExecutedBy::Service,
            children: Arc::from([]),
        },
    ])
}

/// 徽标键归一（change solution-git-sync design D4）：Windows 上 create
/// 的 target 是 `parent.join` 拼的反斜杠, 快照行路径来自 git porcelain
/// 的正斜杠 —— 同一仓两种写法必须落同一徽标键。
pub fn badge_key(path: &str) -> String {
    path.replace('\\', "/")
}

/// 动作 id → 显示文案的静态表（change solution-git-sync design D5）：
/// `KIND_GIT_OP_FINISHED` 的 notice 用它把裸 id 换成人话, 未知 id
/// 返回 `None` → 调用方回退裸 id。文案与各声明处同源, 测试钉住一致。
pub fn action_label(action_id: &str) -> Option<&'static str> {
    match action_id {
        ACTION_CREATE => Some("添加 solution"), // 文案源头在视图提交处
        ACTION_CONFIGURE_REGISTER => Some("注册到资产树"),
        ACTION_CONFIGURE_GIT => Some("git 管理"),
        ACTION_PULL => Some("拉取"),
        ACTION_PUSH => Some("推送"),
        ACTION_CREATE_WORKTREE => Some("创建 worktree"),
        ACTION_DELETE_WORKTREE => Some("删除 worktree"),
        ACTION_SYNC_WORKTREE_FROM_PROJECT => Some("同步 Project 代码"),
        ACTION_SYNC_PROJECT_FROM_WORKTREE => Some("从 worktree 同步"),
        _ => None,
    }
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

/// project 层节点 id 前缀 — 与 asset 树 `project_id()` 同构
/// （change solution-overview D2: 复用 `project://<path>`）。
pub const PROJECT_NODE_PREFIX: &str = "project://";

/// 构造 project 节点 id（视图渲染与服务回指同一函数）。
pub fn project_node_id(root: &Path) -> String {
    format!("{PROJECT_NODE_PREFIX}{}", root.display())
}

/// 解析 project 节点 id → 目标 root；语义同 [`parse_solution_node`]
/// （None = 契约 bug → reject；前缀对但无此 project → `OpError::NoSuch`）。
pub fn parse_project_node(node: Option<&str>) -> Option<PathBuf> {
    node?.strip_prefix(PROJECT_NODE_PREFIX).map(PathBuf::from)
}

/// project 层节点（hierarchy 第二层）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectNode {
    pub desc: ProjectDesc,
    /// 该 project 注册的 worktree 层（hierarchy 第三层；无 → 空层，
    /// change solution-git-worktree design D4：真源 = git 元数据）。
    pub worktrees: Arc<[WorktreeNode]>,
    /// 服务为该节点声明的操作（design D3）。
    pub actions: Arc<[ActionDesc]>,
}

/// worktree 层节点（hierarchy 第三层，change solution-git-worktree）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeNode {
    /// working copy 绝对路径（身份锚点；视图右键回指它）。
    pub path: PathBuf,
    /// 当前分支（detached → `None`）。
    pub branch: Option<String>,
    /// 服务为该节点声明的操作（恒声明 delete，design D5）。
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

/// 创建 worktree 的参数（change solution-git-worktree 4.1）：`name` =
/// 期望的分支/目录名（同名 —— `git worktree add` 按 basename 自建分支），
/// 经消毒后落盘；表单 UI 属后续（design OQ2）。
pub struct CreateWorktreeReq {
    pub name: String,
}

/// worktree 物理目录（workspace 下, design D4 —— 点前缀让资产枚举默认
/// 隐藏；与 tracked-clean 门（D2）互为双解药, 缺一不可）。
pub const WORKTREES_DIR: &str = ".balabala.worktrees";

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
    /// git 域拒绝/失败的结构化原因（change solution-git-worktree,
    /// design D5）：非 git 项目、tracked 不净、git 子进程失败（stderr
    /// 尾部）都归这里 → 既有 notice 链路（按 action_id 命名）。
    Git(String),
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpError::Load(e) => write!(f, "{e}"),
            OpError::NoSuch(p) => write!(f, "no such solution: {}", p.display()),
            OpError::Persist(m) => write!(f, "{m}"),
            OpError::UnknownAction(a) => write!(f, "unknown action: {a}"),
            OpError::Git(m) => write!(f, "{m}"),
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
/// git 操作完成事件（change solution-git-worktree, design D1）：**无 corr**
/// 的 tell —— 原 corr 已在 accepted 回执时销账, 带 corr 迟到会进死信
/// （"late or unknown reply"）。载荷 = 本结构。
pub const KIND_GIT_OP_FINISHED: &str = "solution.git_op_finished";

/// 仓库状态（change solution-git-sync design D4）——行内徽标语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoState {
    /// 干净（门禁通过或执行成功后的态）。
    Clean,
    /// 有未提交的 tracked 改动（tracked-clean 门禁判出）。
    Dirty,
    /// 存在未完成合并（merge 冲突后）——与 Dirty 刻意区分。
    Conflict,
}

/// 单个仓库的状态观测（随完成事件回传，path = project root 或
/// worktree 裸路径，与行身份锚点同键）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoStatus {
    /// 仓库路径（绝对）。
    pub path: String,
    /// 观测到的状态。
    pub state: RepoState,
}

/// `KIND_GIT_OP_FINISHED` 载荷：完成结果按 `action_id` 回到 notice 链路
/// （node-actions: 按 id 路由不按 label）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOpFinished {
    /// 发起时的动作 id（git.pull / git.push / …）—— notice 路由键。
    pub action_id: String,
    /// 操作目标（project root 或 worktree path, 展示用）。
    pub target: String,
    /// 成败。
    pub ok: bool,
    /// 失败原因（git stderr 尾部摘要）；成功 → 空串。
    pub detail: String,
    /// 本次运行检视过的每个仓库的状态（门禁判出/执行后, design D4）；
    /// 空 = 未检视任何仓库 → 视图保持原徽标。
    pub statuses: Vec<RepoStatus>,
}

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
        // change solution-git-worktree 2.1: project 节点从空 → 恒声明三个 git 动作
        let pids: Vec<&str> = root.projects[0]
            .actions
            .iter()
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(
            pids,
            vec![ACTION_PULL, ACTION_PUSH, ACTION_CREATE_WORKTREE],
            "project 节点恒声明 pull/push/create-worktree (design D5 恒声明)"
        );
        assert_eq!(
            root.item.path(),
            std::path::Path::new("/a/foo"),
            "item 原样保留在节点上"
        );
    }

    /// change solution-git-sync (D5): 完成事件 notice 的 id → label 静态表 ——
    /// 7 个已声明 id 全命中、与声明处 label 逐字一致（防漂移）, 未知 id
    /// 返回 None（调用方回退裸 id）。
    #[test]
    fn action_label_covers_declared_ids() {
        let expected: [(&str, &str); 9] = [
            (ACTION_CREATE, "添加 solution"), // 文案源头在视图提交处
            (ACTION_CONFIGURE_REGISTER, "注册到资产树"),
            (ACTION_CONFIGURE_GIT, "git 管理"),
            (ACTION_PULL, "拉取"),
            (ACTION_PUSH, "推送"),
            (ACTION_CREATE_WORKTREE, "创建 worktree"),
            (ACTION_DELETE_WORKTREE, "删除 worktree"),
            (ACTION_SYNC_WORKTREE_FROM_PROJECT, "同步 Project 代码"),
            (ACTION_SYNC_PROJECT_FROM_WORKTREE, "从 worktree 同步"),
        ];
        for (id, label) in expected {
            assert_eq!(action_label(id), Some(label), "{id}");
        }
        // 与参数化的声明处逐字一致（防两处文案漂移）—— 带一个 worktree
        // 让 project 侧也产出 sync 父描述符
        let wt = WorktreeNode {
            path: std::path::PathBuf::from("/ws/.balabala.worktrees/feat"),
            branch: Some("feat".into()),
            actions: worktree_actions(),
        };
        for desc in project_actions(&[wt.clone()])
            .iter()
            .chain(worktree_actions().iter())
        {
            assert_eq!(
                action_label(&desc.id),
                Some(desc.label.as_str()),
                "{} 声明与表漂移",
                desc.id
            );
        }
        assert_eq!(action_label("git.nonexistent"), None, "未知 id 回退");
    }

    /// change solution-git-sync (D4): 徽标键归一 —— 反斜杠/正斜杠同键
    /// （create 的 target 与快照行路径在 Windows 上写法不同）。
    #[test]
    fn badge_key_normalizes_separators() {
        assert_eq!(badge_key(r"D:\ws\proj"), "D:/ws/proj");
        assert_eq!(badge_key("D:/ws/proj"), "D:/ws/proj");
        assert_eq!(badge_key(&badge_key(r"D:\ws")), badge_key("D:/ws"));
    }

    /// 任务 2.1 验证: 两类节点携带的 descriptor 集合与 label（对齐
    /// `solution_actions` 既有测试形态）；solution-git-sync D3: 层非空
    /// 追加 sync 父 + children, 层空不声明。
    #[test]
    fn project_and_worktree_nodes_declare_git_actions() {
        // worktree 层空 → project 三动作, 无 sync 父
        let p0 = project_actions(&[]);
        let p0ids: Vec<&str> = p0.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            p0ids,
            vec![ACTION_PULL, ACTION_PUSH, ACTION_CREATE_WORKTREE]
        );

        // 有 worktree → 第四个 sync 父描述符, children = 每 worktree 一条
        let wt = WorktreeNode {
            path: std::path::PathBuf::from("/ws/.balabala.worktrees/feat"),
            branch: Some("feat".into()),
            actions: worktree_actions(),
        };
        let p = project_actions(&[wt.clone()]);
        let pids: Vec<&str> = p.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            pids,
            vec![
                ACTION_PULL,
                ACTION_PUSH,
                ACTION_CREATE_WORKTREE,
                ACTION_SYNC_PROJECT_FROM_WORKTREE
            ],
            "层非空 → 追加 sync 父描述符"
        );
        let plabels: Vec<&str> = p.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(
            plabels,
            vec!["拉取", "推送", "创建 worktree", "从 worktree 同步"],
            "label 归服务所有"
        );
        for a in p.iter() {
            assert_eq!(a.execution, ExecutedBy::Service, "{} 必须标 Service", a.id);
            assert_eq!(a.checked, None, "{} 非开关", a.id);
        }
        // children: label = 行同源显示名, node = 裸路径
        let sync = p.last().expect("sync 父在场");
        assert_eq!(sync.children.len(), 1, "每 worktree 一条 child");
        assert_eq!(sync.children[0].label, "feat");
        assert_eq!(
            sync.children[0].node, "/ws/.balabala.worktrees/feat",
            "child node = 裸路径"
        );

        // 显示名规则: 分支优先, detached → 目录末段（与视图行同规则）
        assert_eq!(worktree_display_name(&wt), "feat", "分支名优先");
        let detached = WorktreeNode {
            path: std::path::PathBuf::from("/ws/.balabala.worktrees/det"),
            branch: None,
            actions: worktree_actions(),
        };
        assert_eq!(
            worktree_display_name(&detached),
            "det",
            "detached → 目录末段"
        );

        // worktree 节点: delete + sync 两动作
        let w = worktree_actions();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].id, ACTION_DELETE_WORKTREE);
        assert_eq!(w[0].label, "删除 worktree");
        assert_eq!(w[1].id, ACTION_SYNC_WORKTREE_FROM_PROJECT);
        assert_eq!(w[1].label, "同步 Project 代码");
        for a in w.iter() {
            assert_eq!(a.execution, ExecutedBy::Service);
            assert_eq!(a.checked, None);
        }
    }

    /// 任务 2.2 验证: `WorktreeNode` 构造往返不丢字段（typed Payload
    /// 下转型, 非 serde —— 往返 = 构造后逐字段读回）。
    #[test]
    fn worktree_node_round_trips_fields() {
        let node = WorktreeNode {
            path: std::path::PathBuf::from("/ws/.balabala.worktrees/feature-x"),
            branch: Some("feature-x".into()),
            actions: worktree_actions(),
        };
        // 读回
        assert_eq!(node.path, std::path::PathBuf::from("/ws/.balabala.worktrees/feature-x"));
        assert_eq!(node.branch.as_deref(), Some("feature-x"));
        assert_eq!(node.actions[0].id, ACTION_DELETE_WORKTREE);

        // detached 分支 → None 不丢语义
        let detached = WorktreeNode {
            path: std::path::PathBuf::from("/ws/.balabala.worktrees/detached"),
            branch: None,
            actions: worktree_actions(),
        };
        assert_eq!(detached.branch, None);

        // 挂进 ProjectNode 层级后字段仍在 (第三层不掉)
        let proj = ProjectNode {
            desc: ProjectDesc {
                name: "foo".into(),
                root: std::path::PathBuf::from("/a/foo"),
                is_git: true,
            },
            worktrees: Arc::from(vec![node.clone()]),
            actions: project_actions(&[node.clone()]),
        };
        assert_eq!(proj.worktrees.len(), 1);
        assert_eq!(proj.worktrees[0], node, "层级挂载后逐字相等");
        assert_eq!(proj.worktrees[0].branch.as_deref(), Some("feature-x"));
    }

    /// 任务 2.3 验证: 完成事件载荷 typed-Payload 往返不丢字段, 且信封
    /// 形态为**无 corr 的 tell**（design D1: 原 corr 已销账, 带 corr
    /// 迟到 = 死信 "late or unknown reply"）。
    #[test]
    fn git_op_finished_payload_round_trips_and_envelope_has_no_corr() {
        let done = GitOpFinished {
            action_id: ACTION_PUSH.to_string(),
            target: "/a/foo".into(),
            ok: false,
            detail: "remote rejected: non-fast-forward".into(),
            statuses: Vec::new(),
        };
        // 载荷往返 (typed Payload, 同 TreeSnapshot 传输形态)
        let payload = Payload::new(done.clone());
        let back: &GitOpFinished = payload.downcast().expect("类型对上应取回");
        assert_eq!(*back, done, "载荷往返逐字相等");

        // 信封形态: Envelope::new 即 tell (corr = None), 不挂 with_corr
        let env = service_core::Envelope::new(
            service_core::Route::Instance(service_core::InstanceId(1)),
            service_core::Address::instance(service_core::InstanceId(2)),
            KIND_GIT_OP_FINISHED,
            Payload::new(done.clone()),
        );
        assert_eq!(env.kind, KIND_GIT_OP_FINISHED);
        assert_eq!(env.corr, None, "完成事件必须无 corr (design D1)");
        let back = env.downcast::<GitOpFinished>().expect("downcast 应成功");
        assert_eq!(back.action_id, ACTION_PUSH);
        assert_eq!(back.ok, false);
        assert!(back.detail.contains("non-fast-forward"), "失败原因随载荷");
    }

    /// 默认名（6.2 冒烟前置, OQ2 表单未就绪的过渡）：`<project>-<毫秒>`，
    /// project 段过滤成分支名安全字符集 —— 用真 git `check-ref-format`
    /// 裁决合法性, 不自写规则。
    #[test]
    fn default_create_worktree_args_are_valid_branch_names() {
        for node in [
            "project:///ws/my proj",  // 空格 → 折 `-`
            "project:///ws/a..b",     // `..` → 折单点
            "project:///ws/.hidden",  // 前导 `.` 丢弃
            "project:///ws/-lead",    // 前导 `-` 丢弃
            "project:///ws/我的项目", // 中文保留 (Unicode 项目名)
            "project:///ws/+++",      // 全非法 → 兜底 `worktree`
            "project:///,",           // 逗号名 → 全非法 → 兜底 `worktree`
            "project:///",            // 无 file_name → 兜底 `worktree`
        ] {
            let req = default_create_worktree_args(node).expect("project 节点");
            // 形状: <段>-<纯数字毫秒>
            let (seg, ts) = req.name.rsplit_once('-').expect("段-时间戳两段");
            assert!(!seg.is_empty() && !ts.is_empty(), "{node} → {}", req.name);
            assert!(
                ts.chars().all(|c| c.is_ascii_digit()),
                "{node} → {}",
                req.name
            );
            // 真 git 裁决: 合法分支名 (git worktree add 按 basename 自建分支)
            let out = crate::testkit::block_on(crate::git::run_async(
                &["check-ref-format", "--branch", &req.name],
                Path::new("."),
            ))
            .unwrap_or_else(|e| panic!("git 不可用: {e}"));
            assert_eq!(
                out.code, 0,
                "{node} → {:?} 不是合法分支名: {}",
                req.name, out.stderr
            );
        }
        // 非 project 节点 → None (契约 bug 信号: 服务端 parse 同样会拒)
        assert!(default_create_worktree_args("solution:///x").is_none());
    }

    /// typed args 分派表 (design D6)：create → 默认名；configure → 开关
    /// 反值；其余动作（pull/push/delete）无 args。
    #[test]
    fn action_args_dispatches_per_action() {
        let node = "project:///ws/proj";
        let a = action_args(ACTION_CREATE_WORKTREE, node, None).expect("create 有 args");
        let req = a
            .downcast::<CreateWorktreeReq>()
            .expect("CreateWorktreeReq");
        assert!(req.name.starts_with("proj-"), "{}", req.name);

        let a =
            action_args(ACTION_CONFIGURE_REGISTER, node, Some(false)).expect("configure 有 args");
        let cfg = a.downcast::<ConfigureReq>().expect("ConfigureReq");
        assert_eq!(cfg.register_to_asset_manager, Some(true), "checked 取反");

        assert!(
            action_args(ACTION_PULL, node, None).is_none(),
            "pull 无 args"
        );
        assert!(
            action_args(ACTION_PUSH, node, None).is_none(),
            "push 无 args"
        );
        assert!(
            action_args(ACTION_DELETE_WORKTREE, node, None).is_none(),
            "delete 无 args"
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
                    worktrees: Arc::from([]),
                    actions: project_actions(&[]),
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
