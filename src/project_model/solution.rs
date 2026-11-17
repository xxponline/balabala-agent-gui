//! Solution 配置项与注册表 (design D1)。
//!
//! `balabala.solutions.json` 是个 JSON 数组, 每项带 `type` (SolutionType)。
//! **注册表 = 枚举 + `expand_item` match**: 加新 solution 类型 =
//! 加变体 + 加 match 臂, schema 与加载器不动。

use std::path::PathBuf;

/// 配置中的一个 solution item (`{"type": "simple", "path": "..."}`)。
///
/// 未知 `type` 在反序列化阶段被拒 (serde unknown variant) → 加载错误契约
/// (spec: Solution load failure blocks the workspace)。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SolutionItem {
    /// v1 唯一类型: 一个目录路径 → 一个 Project。
    Simple { path: PathBuf },
}

/// `simple` 类型的实现: 吃一个 Path, 输出一个 Project + 同名 root Module。
#[derive(Debug, Clone)]
pub struct SimpleSolution {
    pub path: PathBuf,
}

impl SimpleSolution {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// 展开: 不校验磁盘 (存在性检查在 validate 层, 错误契约统一)。
    pub fn expand(&self) -> crate::project_model::Project {
        crate::project_model::Project::from_root(self.path.clone())
    }
}

/// 注册表分发: SolutionType → 实现 → Project 列表。
///
/// v1 每 item 恰好一个 Project; 未来多输出类型在此返回 Vec 并进 DAG 校验。
pub fn expand_item(item: &SolutionItem) -> Vec<crate::project_model::Project> {
    match item {
        SolutionItem::Simple { path } => vec![SimpleSolution::new(path.clone()).expand()],
    }
}
