//! 领域模型层: Project/Module 结构 (资产枚举链的 Project 域)。
//!
//! - **纯数据层** — 不依赖 gpui, 可无头单测。
//! - Solution/Project 的加载解析与配置语义已迁 solution 服务
//!   (change solution-service; 见 `crates/solution-service`)。
//!
//! 行为契约: openspec/specs/project-model/spec.md。

pub mod enumerate;
pub mod files;
pub mod filter;
pub mod project;

pub use files::{MarkerStatus, create_workspace_files, marker_status};
pub use project::{Module, Project};
