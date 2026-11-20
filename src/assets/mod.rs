//! assets — 右槽资产树的单一包: 中立 DTO、枚举契约、服务层与视图面板。
//!
//! 职责链 (change: asset-package-merge): solution 注册 owner → 异步枚举
//! (project 域实现, 见 `crate::project_model::enumerate`) → `AssetState`
//! 落地 → 面板纯显示。依赖方向单向: `project_model → assets` —
//! 本包只见 [`traits`] 两个契约与 [`state`] DTO, 不见 Project/filter。
//!
//! 模块分步归位 (tasks 组1): [`state`]/[`traits`]/[`manager`]/[`panel`]
//! 已全部落位; 枚举实现归位 project 域见 1.4。

pub mod state;
pub mod traits;
pub mod manager;
pub mod panel;

/// 模块公共 API — DTO 家族 + 常用辅助 (部分仅测试与后续 change 消费;
/// bin crate 无外部使用者, 故放行未用)。
#[allow(unused_imports)]
pub use state::{
    AssetEffect, AssetKind, AssetMenu, AssetNode, AssetState, find_kind, project_id,
};
/// 过渡期重导出 — 消费方 (app/shell) 改净前尚无直接使用者。
#[allow(unused_imports)]
pub use manager::AssetManager;
pub use panel::AssetView;
