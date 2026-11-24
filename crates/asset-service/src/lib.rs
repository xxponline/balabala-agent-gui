//! `asset-service` — 右槽资产数据的总线服务 + 面板（change asset-service design D1）。
//!
//! **形态对齐 `agent-service`**：同一 crate 内既装总线服务
//! （[`service`] 的 `impl Service for AssetService`）也装 GUI 面板
//! （[`panel`] 的 `impl BasePanel for AssetOverview`），因此本 crate 依赖
//! `gpui-kit` —— 这是本项目既定先例（`agent-service` 为 `AgentTerminal`
//! 渲染层同样预置了 `gpui-kit`）。
//!
//! 职责链（同构 `solution-service`）：
//!
//! ```text
//! GUI ── asset.register (ask, DTO) ──▶ AssetService
//! AssetService ── spawn_task ──▶ 后台枚举   ← 单 worker, 不得内联 (design D4)
//! AssetService ── asset.tree_changed (tell) ──▶ GUI 镜像
//! AssetOverview ◀── 读镜像 ── service_layer.rs
//! ```
//!
//! 模块划分：
//! - [`contract`] — 线合同：`AssetContract`、注册 DTO、`TreeSnapshot`/`TreeChanged`、kind 常量
//! - [`service`] — `impl Service for AssetService`：注册集、三态视图、代数、`action_id` 分派
//! - [`filter`] — `FileFilter` trait 与实现（自 `src/project_model/filter.rs` 迁入）
//! - [`enumerate`] — 资产枚举（自 `src/project_model/enumerate.rs` 迁入，改操作 DTO）
//! - [`panel`] — `AssetOverview` 右槽面板（自 `src/assets/panel.rs` 迁入）
//!
//! 依赖方向：`src/`（二进制）→ 本 crate；`Project`/`Module` 留在
//! `src/project_model/`，经注册 DTO 转换后交给服务（design D3）——
//! 本 crate **不认识** `Project`，也不认识 GUI 实体（面板经
//! [`panel::AssetHost`] 窄缝回访宿主）。
//!
//! ## 执行归属跟着状态走（design D5 —— 本 crate 的核心判据）
//!
//! 节点上的动作由**声明它的服务**决定谁来执行，判据是"状态在哪"：
//!
//! | 状态位置 | 归属 | 例子 | 结果 |
//! |---|---|---|---|
//! | **能力在视图**（剪贴板、OS 集成） | [`ExecutedBy::View`] | 复制路径、在 Finder 显示 | 视图本地执行，**不发 invoke** |
//! | **状态在服务**（配置、落盘） | [`ExecutedBy::Service`] | 注册到资产树、git 管理 | 视图发 invoke，服务执行并回执 |
//!
//! [`ExecutedBy`] **必须显式标注、无缺省值** —— 缺省会让"忘标"静默降级：
//! 资产动作忘标会把剪贴板动作送进总线（服务在后台线程够不着 GPUI）；
//! solution 动作忘标会让配置绕过服务、**绕过写入口单点**（design D6）。
//!
//! 判据的落点：[`contract::view_row_actions`]（服务声明的两条 View 动作）、
//! [`panel`]（菜单从 `node.actions` 派生、效果在面板内落地 —— 全程 0 条
//! 服务消息）。

/// 判据中的执行归属枚举 —— 重新导出便于文档链接。
pub use contracts::shared::ExecutedBy;

pub mod contract;
pub mod enumerate;
pub mod filter;
pub mod panel;
pub mod service;

#[cfg(test)]
mod offload_tests;
#[cfg(test)]
mod testkit;
