//! 消息契约（design D1/D3）：kind 常量、注册 DTO 与视图快照——GUI 与
//! `AssetService` 之间唯一的共享词汇。形态对齐 `solution_service::contract`。
//!
//! **DTO 是 `Project`/`Module` 与服务之间的边界**（design D3 修订）：
//! `src/project_model/` 被 launcher / workspace / solution_overview /
//! service_layer / app 广泛使用，**不能整体搬进本 crate**；而枚举与过滤
//! 是纯逻辑，应随服务走。
//!
//! **分工**：GUI 只出**纯结构**（字段映射，不读文件、不构造过滤集）；
//! 服务拿到根与 `is_git` 后自己读 `.gitignore`、组过滤集、扫盘出树。
//! 本 crate **不认识 `Project`**。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use contracts::shared::{ActionDesc, ExecutedBy};

// ---------- 视图状态（自 `src/assets/state.rs` 迁入 — task 3.1） ----------

/// 服务暴露的三态视图 (design D5: 三态 + 原子落地)。
///
/// **入契约的理由**：服务要把视图推给 GUI 镜像（`asset.tree_changed`），
/// 载荷必须是服务与 GUI 共享的词汇 — design D8 的「并入契约」分支。
#[derive(Clone, Debug, PartialEq)]
pub enum AssetState {
    /// 无工作区 / 零注册 — 面板显示空态。
    Empty,
    /// 模块根枚举失败 — 面板显示降级提示 (命名失败原因)。
    Degraded { message: String },
    /// 已过滤的完整森林 (一次落地; 过滤规则源 = 项目根 `.gitignore`,
    /// 不再随视图携带 — design D9)。
    Forest { nodes: Vec<AssetNode> },
}

// ---------- 资产树 DTO（自 `src/assets/state.rs` 迁入，task 2.4） ----------

/// 资产种类。
///
/// `Dir`/`File` 是普通树行; `Project`/`Module` 是模型层行
/// (spec: 右树以已注册 Project 为根节点, 其下为 Module)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetKind {
    /// Project 行 (分组头; id 带前缀, 见 [`project_id`])。
    Project,
    /// Module 行 (id = 模块根的绝对路径 — 兼容目录行的菜单/复制语义)。
    Module,
    Dir,
    File,
}

impl AssetKind {
    /// 可展开行 (folder 图标 + 折叠集合兜底): Project/Module/Dir。
    pub fn is_folder(self) -> bool {
        matches!(self, AssetKind::Project | AssetKind::Module | AssetKind::Dir)
    }
}

/// 树节点 DTO — 中立, 不含任何 gpui 类型, 可跨面板/服务边界。
///
/// `id` = 绝对路径字符串 (目录/文件, 树内唯一; 右键/左键载荷与复制路径
/// 共用它) — 唯一例外是 [`AssetKind::Project`] 行, 见 [`project_id`]。
/// **身份体系不变**: `AssetNode::id` 对目录/文件仍是绝对路径字符串 —
/// center-tab dedup、复制路径、`OpenObject` key 全部免改。
#[derive(Clone, Debug, PartialEq)]
pub struct AssetNode {
    pub id: String,
    pub label: String,
    pub kind: AssetKind,
    pub children: Vec<AssetNode>,
    /// 服务为该节点声明的动作（`node-actions` spec: 描述符随快照下传 ⇒
    /// 菜单构建零往返）。Project 行恒为空 ⇒ 右键无菜单。
    ///
    /// **全部标 `ExecutedBy::View`**（design D5：执行归属跟着状态走 ——
    /// 剪贴板与 OS 集成本就是 view 层能力，服务只声明"有这个功能"与
    /// 标签，不参与执行、也不产生任何 invoke 消息）。
    pub actions: Arc<[ActionDesc]>,
}

/// Project 行的 id: `project://<绝对路径>`。
///
/// v1 的 Project 根与 root Module 根是同一目录 — 若两行都用裸路径会撞
/// id (树内唯一性 + 观测断言歧义)。Project 行是纯分组头 (无菜单/打开
/// 语义), 命名空间前缀只存在于行身份上; 一切路径语义仍由 Module/文件行
/// 的裸绝对路径承载 (design D3 的 id 稳定面)。
pub fn project_id(root: &Path) -> String {
    format!("project://{}", root.display())
}

// ---------- 契约类标记 ----------

/// asset 契约类标记（对齐 `AgentContract`/`SolutionContract`）：
/// "asset 这一类服务"的身份判据，宿主以 `ContractId::of::<AssetContract>()` 铸造。
pub struct AssetContract;

// ---------- 请求载荷 ----------

/// 全量注册（workspace 打开 / 切换 / 注册集变化；design D2/D3）。
///
/// **带 `workspace_dir`** —— 资产服务没有 solution 侧那个 `load` 边界
/// （它不读 workspace 文件），规则读取点（`root/.gitignore`）由项目根
/// 自身携带，所以这里只需知道工作区目录以判空态与去重。
#[derive(Clone)]
pub struct AssetRegistration {
    pub workspace_dir: PathBuf,
    pub projects: Vec<ProjectRegistration>,
}

/// 显式刷新（⟳）— supersede 在途、重读 filter 源。
pub struct RefreshReq;

/// 视图变更订阅握手（单订阅者，对齐 solution/agent 的 watch 形态）。
pub struct WatchReq;

/// 节点动作调用 — **写入口单点**（对齐 `solution.action`）。
///
/// asset 侧两个动作均标 `ExecutedBy::View`（design D5），所以本消息
/// 目前只用于 **服务侧动作**；携带 `args` 位以备将来（`node-actions`:
/// "arguments SHALL be absent for actions that need none"）。
pub struct ActionInvoke {
    /// `solution://`/`project://`/裸路径 皆可 — 服务按树解析。
    pub node: Option<String>,
    /// 回执路由键（design D7：按 id 不按文案）。
    pub action_id: String,
}

/// 服务为**可菜单行**声明的动作（task 6.1）。
///
/// 共享一份 `OnceLock`：资产树可达数十万行，逐节点分配 2 个 `ActionDesc`
/// 不可接受 —— `Arc` 克隆只 bump 引用计数。
///
/// 两条都标 [`ExecutedBy::View`]：复制路径要 `cx.write_to_clipboard`
/// （GPUI `App` 方法、仅主线程）、reveal 要发系统命令 —— 这两个**能力**
/// 本就在 view 层，服务声明它们只是"注册有这么个功能"（design D5）。
/// 视图执行它们 ⇒ 不产生 invoke 消息。
pub fn view_row_actions() -> Arc<[ActionDesc]> {
    static ACTIONS: std::sync::OnceLock<Arc<[ActionDesc]>> = std::sync::OnceLock::new();
    ACTIONS
        .get_or_init(|| {
            Arc::from(vec![
                ActionDesc {
                    id: ACTION_COPY_PATH.to_string(),
                    label: "复制路径".to_string(),
                    checked: None,
                    execution: ExecutedBy::View,
                    children: std::sync::Arc::from([]),
                },
                ActionDesc {
                    id: ACTION_REVEAL.to_string(),
                    label: "在 Finder 显示".to_string(),
                    checked: None,
                    execution: ExecutedBy::View,
                    children: std::sync::Arc::from([]),
                },
            ])
        })
        .clone()
}

// ---------- 视图快照与事件 ----------

/// 视图快照（服务 → GUI 的读取出口与事件载荷）。
#[derive(Debug, Clone, PartialEq)]
pub struct TreeSnapshot {
    /// 当前三态视图。
    pub state: AssetState,
    /// 视图代数：每次失效与每次落地各 +1 — 面板以相等比对做 O(1) 失效
    /// 检测（spec: O(1) when nothing changed）。
    pub generation: u64,
}

/// 视图变更事件（tell 自带快照；载荷粒度 = 整视图）。
#[derive(Debug, Clone, PartialEq)]
pub struct TreeChanged(pub TreeSnapshot);

// ---------- 回执 ----------

/// 结构化回执错误（对齐 `solution_service::OpError` 的三路归因思路）。
#[derive(Debug, Clone, PartialEq)]
pub enum OpError {
    /// 注册从未到达就收到 refresh/action — 调用顺序 bug，以红条暴露
    /// （不 panic、不静默丢）。
    NoWorkspace,
    /// 调用方发了服务**未声明**的动作 id — **立即回执**而非 `reject`：
    /// reject 不回信封，调用方要等 `DEFAULT_CALL_TIMEOUT = 5s` 才拿到
    /// Timeout，notice 只写 “ask timeout” — 既慢又不点名（对齐
    /// solution 侧实施期定案 C）。
    UnknownAction(String),
    /// 枚举失败的结构化原因（降级态文案来源）。
    EnumFailed(String),
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpError::NoWorkspace => write!(f, "no workspace registered"),
            OpError::UnknownAction(id) => write!(f, "unknown action: {id}"),
            OpError::EnumFailed(m) => write!(f, "enumeration failed: {m}"),
        }
    }
}

impl std::error::Error for OpError {}

/// 控制回执：`Result<TreeSnapshot, OpError>`（先回再推事件，形态对齐
/// solution 的 `OpResult`）。
pub type OpResult = Result<TreeSnapshot, OpError>;

// ---------- 服务内部回投 ----------

/// 枚举完成 — 服务**向自己**发的 tell（task 3.3：枚举跑在
/// `spawn_blocking`，结果必须回到 handler 才能改状态）。
///
/// `seq` 是代数令牌：被 supersede 的在途枚举（刷新/重注册）回投时
/// `seq != self.enum_seq` ⇒ 直接丢弃，旧结果不得覆盖新视图。
#[derive(Debug, Clone, PartialEq)]
pub struct EnumerationDone {
    pub seq: u64,
    /// `Ok` = 完整森林；`Err` = 模块根不可读等原因（落降级态）。
    pub result: Result<Vec<AssetNode>, String>,
    /// **观测面**：执行本次枚举的线程（含 `.gitignore` 读取与 `read_dir`
    /// 的同步 fs）。正常路径非空；`spawn_blocking` join 失败时为 `None`。
    ///
    /// 用途：`offload_tests` 断言它 ≠ 驱动后台任务的单 worker ——
    /// 这是 spec *Enumeration does not delay other services* 的直接判据
    /// （design D4）。`ThreadId` 是进程内不透明值，不参与序列化。
    pub fs_thread: Option<std::thread::ThreadId>,
}

// ---------- kind 常量（Envelope.kind） ----------

/// 全量注册（ask）。
pub const KIND_REGISTER: &str = "asset.register";
/// 显式刷新（ask）。
pub const KIND_REFRESH: &str = "asset.refresh";
/// 节点动作写入口单点（ask）。
pub const KIND_ACTION: &str = "asset.action";
/// 读当前视图（ask）。
pub const KIND_GET_STATE: &str = "asset.get_state";
/// 视图订阅握手（tell）。
pub const KIND_WATCH: &str = "asset.watch";
/// 视图变更推送（**tell** — `corr = None`，形如 `solution.tree_changed`）。
pub const KIND_TREE_CHANGED: &str = "asset.tree_changed";
/// ask 回执。
pub const KIND_OP_REPLY: &str = "asset.op_reply";
/// 枚举完成回投（服务 → 服务自身，tell）—— 不是对外契约，但需一个
/// kind 才能进 `dispatch`。
pub const KIND_ENUM_DONE: &str = "asset.enum_done";

// ---------- 动作 id 常量 ----------

/// 复制绝对路径。
pub const ACTION_COPY_PATH: &str = "copy-path";
/// 在 Finder 中显示。
pub const ACTION_REVEAL: &str = "reveal";



/// 一个待注册项目的描述（change asset-service design D3）。
///
/// 字段**全 `pub`**：由 `src/project_model` 侧构造，本 crate 只读。
/// `is_git` 携带已解析的 git 管理标志（`project-model` spec: Filter
/// provisioning —— 运行期不再探测 `root/.git`）。
#[derive(Clone)]
pub struct ProjectRegistration {
    /// 项目名 = root 目录末段。
    pub name: String,
    /// 项目根目录（绝对路径，枚举与 reveal 的身份根）。
    pub root: PathBuf,
    /// 已解析的 git-managed 标志 — 只读，服务不再自行探测。
    pub is_git: bool,
    /// 项目下的模块（root module 在首，name == project name）。
    pub modules: Vec<ModuleRegistration>,
}

/// 一个模块的描述（递归结构，与 `Module` 同形）。
///
/// **纯结构 —— 不携带过滤集**（design D3 修订）。过滤集由服务在**每次
/// 枚举**时按 `ProjectRegistration::{root, is_git}` 现场供给（spec:
/// Filter provisioning from the project root / Refresh re-reads filter
/// rules），若在转换期固化，⟳ 就会拿陈旧规则去扫。
#[derive(Clone)]
pub struct ModuleRegistration {
    /// 模块名（root module 与项目同名）。
    pub name: String,
    /// 模块根目录 — 界定资产枚举范围。
    pub root: PathBuf,
    /// 子模块（树形）。
    pub children: Vec<ModuleRegistration>,
}
