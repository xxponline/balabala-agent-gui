//! 双向共享消息类型的中立命名空间（design D2）。
//!
//! 归属规则：
//! - **单向消息归调用方**：调用方拥有自己发的消息与期待的回复类型，放自己
//!   服务的命名空间（如 `contracts::agent`、`contracts::asset`），被调方
//!   import 调用方契约；
//! - **双向共享的消息类型归本命名空间**：裂分独立契约 crate 时它 = 独立
//!   中立 crate（零依赖），保证 `contract-a` / `contract-b` 永不形成编译期
//!   互依。
//!
//! 本 crate **零依赖**（`cargo build -p contracts` 不引入任何外部 crate）。

/// 节点动作由哪一层执行（change asset-service design D6）。
///
/// **必须显式标注，不提供缺省值** —— 缺省会让「忘标」静默降级：
/// 若缺省为 [`ExecutedBy::Service`]，asset 侧忘标会把剪贴板动作送进总线，
/// 而服务在后台线程够不着 GPUI 剪贴板，只能静默失败；若缺省为
/// [`ExecutedBy::View`]，solution 侧忘标会让配置动作绕过服务、
/// **绕过写入口单点**（design D6：这是最坏失败模式）。显式标注 ⇒ 编译期漏标即报错。
///
/// 判据：**执行归属跟着状态走** —— 能力在视图（剪贴板 / OS 集成）就归视图，
/// 状态在服务（solution 配置）就归服务（design D5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutedBy {
    /// 服务执行 —— 视图发 invoke，服务处理并回执。
    Service,
    /// 视图执行 —— 视图直接执行，**不发任何消息**给声明服务。
    View,
}

/// 节点上声明的可用操作（change solution-overview design D3；
/// change asset-service D6 迁入本中立命名空间）。
///
/// **描述符随快照下传** — 右键菜单闭包是同步的, 等不了 ask; 而视图同步读
/// 镜像, 描述符搭快照的车 ⇒ 菜单构建零往返。**label 归服务所有**（契约
/// 从此携带 UI 文案 — 否则视图必须认识每个 id 才能显示, 又耦合回服务）。
///
/// **无缺省值**：漏标 `execution` 的构造无法通过编译 —— 这是本字段
/// 存在的核心保障（design D6）。下列编译失败样例由 doctest 执行，一旦
/// 将来给 `ActionDesc` 加了 `Default` 或缺省分支，它会变绿并报警：
///
/// ```compile_fail
/// use contracts::shared::ActionDesc;
/// use std::sync::Arc;
/// let _ = ActionDesc {
///     id: "x".into(),
///     label: "y".into(),
///     checked: None,
///     children: Arc::from([]),
/// };
/// ```
///
/// **为何在本 crate**：多个服务都会发布带动作的节点（solution 行、
/// asset 行），共享类型必须落在服务之间，否则 asset 服务会形成对
/// solution 服务的编译期依赖 — 违反本模块文档的「双向共享归本命名空间」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionDesc {
    /// 稳定动作 id — 回执路由键（design D7: 路由按 id 不按文案）。
    pub id: String,
    /// 显示文案 — 服务拥有（design D3）。
    pub label: String,
    /// 开关项当前值; `None` = 非开关。当前值只有服务知道。
    pub checked: Option<bool>,
    /// 由哪一层执行 — **无缺省值**，构造点必须显式给出（design D6）。
    pub execution: ExecutedBy,
    /// 子菜单子项（change solution-git-sync design D3）：非空 ⇒ 视图把该
    /// 动作渲染为子菜单，每项 `node` = 点击时回传的目标节点 id；空 ⇒
    /// 既有平铺项。子项由服务随快照声明（同 label 归服务）。
    pub children: std::sync::Arc<[ActionChild]>,
}

/// [`ActionDesc`] 的子菜单子项 — 点击发 `(node = child.node,
/// action_id = 父动作 id)`，目标寻址与裸路径节点同款（change
/// solution-git-sync design D3）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionChild {
    /// 子项显示文案 — 服务拥有（同 ActionDesc::label）。
    pub label: String,
    /// 点击时作为 invoke 的 node 回传（如 worktree 裸路径）。
    pub node: String,
}
