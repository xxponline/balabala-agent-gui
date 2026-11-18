//! asset_manager — 右槽数据的服务层 + UI ViewModel (change:
//! asset-manager-right-dock, design D1)。
//!
//! 职责链: solution 注册 Project → 异步枚举 (Project → Module → 文件) →
//! 过滤 (相对各 Project 根) → `AssetView` 落地 → 面板纯显示。
//! 本模块是旧 `workspace_property_source` 数据半边的继任者:
//! DTO (`dto`), gitignore 过滤 (`filter`), 枚举契约与实现 (`enumerator`),
//! 注册/守卫/钩子 (`manager`)。

pub mod dto;
pub mod enumerator;
pub mod manager;

/// 模块公共 API — DTO 家族 + 常用辅助 (部分仅测试与后续 change 消费;
/// bin crate 无外部使用者, 故放行未用)。
#[allow(unused_imports)]
pub use dto::{
    AssetEffect, AssetKind, AssetMenu, AssetNode, AssetView, find_kind, project_id,
};
pub use manager::AssetManager;
