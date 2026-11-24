//! 领域模型层: Project/Module 结构 (资产枚举链的 Project 域)。
//!
//! - **纯数据层** — 不依赖 gpui, 可无头单测。
//! - Solution/Project 的加载解析与配置语义已迁 solution 服务
//!   (change solution-service; 见 `crates/solution-service`)。
//! - **枚举与过滤已迁资产服务** (change asset-service task 2.2/2.4; 见
//!   `crates/asset-service`) —— 依赖方向 `src/` → `crates/asset-service`,
//!   本层只留 `Project`/`Module` 数据与 `Project → 注册 DTO` 的转换。
//! - `files.rs` (工作区 marker / solutions 文件契约) 与枚举无关, 留在本层。
//!
//! 行为契约: openspec/specs/project-model/spec.md。

pub mod files;
pub mod project;

pub use files::{MarkerStatus, create_workspace_files, marker_status};
pub use project::Project;
