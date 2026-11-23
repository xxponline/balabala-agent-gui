//! 解析内核（design D8）：`balabala.solutions.json` → [`SolutionItem`] →
//! Project 描述展开与校验。同步入口，service 内部调用；GUI 一律经 ask。
//!
//! 语义自 `src/project_model`（solution/validate）收编，对齐既有 load 契约：
//! 缺失文件 = 合法空工作区；解析失败 / 未知 SolutionType / 死 path →
//! 结构化 [`LoadError`]（spec: Solution load failures）。

use crate::contract::{LoadError, ProjectDesc, SolutionItem};
use std::path::{Path, PathBuf};

/// solutions 文件名（spec: Solution configuration file）。
pub const SOLUTIONS_FILE: &str = "balabala.solutions.json";

/// solutions 文件路径。
pub fn solutions_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(SOLUTIONS_FILE)
}

/// 解析 JSON 文本 → items。
///
/// 两级解析（design D8）：先结构（必须是对象数组）后变体——未知
/// `type` 单列 [`LoadError::UnknownType`]，其余 serde 失败归
/// [`LoadError::Parse`]。配置字段缺省注入由 DTO 的 `serde(default)` 承担
/// （缺失 = true，spec: legacy item without flags）。
pub fn parse_items(text: &str) -> Result<Vec<SolutionItem>, LoadError> {
    let values: Vec<serde_json::Value> =
        serde_json::from_str(text).map_err(|e| LoadError::Parse(e.to_string()))?;
    values.iter().map(parse_item).collect()
}

/// 单 item 解析：`type` 缺失/非对象 → Parse；未知变体 → UnknownType。
fn parse_item(value: &serde_json::Value) -> Result<SolutionItem, LoadError> {
    match value.get("type").and_then(|v| v.as_str()) {
        None => Err(LoadError::Parse(
            "solution item missing `type`".to_string(),
        )),
        Some("simple") => serde_json::from_value(value.clone())
            .map_err(|e| LoadError::Parse(e.to_string())),
        Some(other) => Err(LoadError::UnknownType(other.to_string())),
    }
}

/// 加载边界统一入口：读文件（缺失 = 合法空，spec）→ 解析 → 死 path 拦截
/// → 展开 + DAG 校验（v1 空边集，骨架对齐旧 `validate::load`）。
pub fn load_items(workspace_dir: &Path) -> Result<Vec<SolutionItem>, LoadError> {
    let path = solutions_path(workspace_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text =
        std::fs::read_to_string(&path).map_err(|e| LoadError::Parse(format!("read {path:?}: {e}")))?;
    let items = parse_items(&text)?;
    for item in &items {
        if !item.path().exists() {
            return Err(LoadError::MissingPath(item.path().to_path_buf()));
        }
    }
    check_dag(items.len(), &[])?;
    Ok(items)
}

/// 单 item → Project 描述（`is_git = root 是 git 目录 AND
/// solution.git_enabled`，spec: Git-managed resolution——随描述下发）。
pub fn expand_item(item: &SolutionItem) -> ProjectDesc {
    let root = item.path().to_path_buf();
    let name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/".to_string());
    let is_git = root.join(".git").exists() && item.git_enabled();
    ProjectDesc { name, root, is_git }
}

/// items → Project 描述全量集（展开 + git 解析）。
pub fn expand_items(items: &[SolutionItem]) -> Vec<ProjectDesc> {
    items.iter().map(expand_item).collect()
}

/// 落盘 solutions（pretty 保序；写失败文案给调用方——红条契约，
/// workspace-management: Persistence write failure surfacing）。
pub fn save_items(workspace_dir: &Path, items: &[SolutionItem]) -> Result<(), String> {
    let path = solutions_path(workspace_dir);
    let json = serde_json::to_string_pretty(items).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("write {}: {e}", path.display()))
}

/// DAG 校验（Kahn 拓扑）：`edges = [(from, to)]` 有环 → [`LoadError::Cycle`]。
///
/// v1 展开无跨项目依赖 → 传空边集（空转骨架；多输出 solution 类型落地时
/// 把边集填进来即可，spec: validation activates once solutions can emit
/// multiple projects）。
pub fn check_dag(node_count: usize, edges: &[(usize, usize)]) -> Result<(), LoadError> {
    let mut indegree = vec![0usize; node_count];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); node_count];
    for &(from, to) in edges {
        if from >= node_count || to >= node_count {
            return Err(LoadError::Cycle(format!(
                "edge {from}->{to} out of range ({node_count} nodes)"
            )));
        }
        adj[from].push(to);
        indegree[to] += 1;
    }
    let mut queue: Vec<usize> = (0..node_count).filter(|&n| indegree[n] == 0).collect();
    let mut visited = 0usize;
    while let Some(n) = queue.pop() {
        visited += 1;
        for &next in &adj[n] {
            indegree[next] -= 1;
            if indegree[next] == 0 {
                queue.push(next);
            }
        }
    }
    if visited == node_count {
        Ok(())
    } else {
        let stuck: Vec<String> = (0..node_count)
            .filter(|&n| indegree[n] > 0)
            .map(|n| n.to_string())
            .collect();
        Err(LoadError::Cycle(format!(
            "nodes [{}] form a cycle",
            stuck.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧格式 fixture（无配置字段）→ 缺省双 true（spec: legacy item
    /// without flags loads with unchanged behavior）。
    #[test]
    fn legacy_item_without_flags_defaults_both_true() {
        let items = parse_items(r#"[{"type": "simple", "path": "/a/foo"}]"#).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].register_to_asset_manager(), "缺省 = true");
        assert!(items[0].git_enabled(), "缺省 = true");
    }

    /// 显式 false 原样保留（缺省注入不覆盖显式值）。
    #[test]
    fn explicit_false_flags_survive_parsing() {
        let items = parse_items(
            r#"[{"type": "simple", "path": "/a/foo",
                 "register_to_asset_manager": false, "git_enabled": false}]"#,
        )
        .unwrap();
        assert!(!items[0].register_to_asset_manager());
        assert!(!items[0].git_enabled());
    }

    /// 三种错误 reason 各一条（task 1.2 验证面）。
    #[test]
    fn error_reasons_are_structured() {
        // parse: 非法 JSON
        assert!(matches!(parse_items("{oops"), Err(LoadError::Parse(_))));
        // parse: item 缺 type
        assert!(matches!(
            parse_items(r#"[{"path": "/a"}]"#),
            Err(LoadError::Parse(_))
        ));
        // unknown-type: 注册表外类型
        assert!(matches!(
            parse_items(r#"[{"type": "fancy", "path": "/a"}]"#),
            Err(LoadError::UnknownType(t)) if t == "fancy"
        ));
        // missing-path: 磁盘上不存在（写临时 solutions 文件，走真实 load 入口）
        let dir = std::env::temp_dir().join(format!("sol-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gone = dir.join("nope");
        let text = format!(r#"[{{"type": "simple", "path": "{}"}}]"#, gone.display());
        std::fs::write(solutions_path(&dir), &text).unwrap();
        assert!(matches!(
            load_items(&dir),
            Err(LoadError::MissingPath(p)) if p == gone
        ));
    }

    /// 缺失文件 = 合法空（spec: Missing solutions file is a valid empty
    /// workspace）；空目录不炸。
    #[test]
    fn missing_file_is_valid_empty() {
        let dir = std::env::temp_dir().join(format!("sol-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load_items(&dir).unwrap(), Vec::new());
    }

    /// DAG 骨架拒绝合成环（spec: Cyclic project dependency rejected）。
    #[test]
    fn cyclic_edges_rejected() {
        assert!(matches!(
            check_dag(2, &[(0, 1), (1, 0)]),
            Err(LoadError::Cycle(_))
        ));
    }

    /// task 4.1 验证面 (spec: Git-managed resolution combines root and
    /// solution): 三 scenario — git 根 + 开 / git 根 + 关 / 非 git 根。
    #[test]
    fn git_resolution_scenarios() {
        let base = std::env::temp_dir().join(format!("sol-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let git_root = base.join("gitproj");
        std::fs::create_dir_all(git_root.join(".git")).unwrap();
        let plain_root = base.join("plainproj");
        std::fs::create_dir_all(&plain_root).unwrap();

        let item = SolutionItem::Simple {
            path: git_root.clone(),
            register_to_asset_manager: true,
            git_enabled: true,
        };
        assert!(expand_item(&item).is_git, "git 根 + solution 开 → git-managed");

        let item = SolutionItem::Simple {
            path: git_root.clone(),
            register_to_asset_manager: true,
            git_enabled: false,
        };
        assert!(
            !expand_item(&item).is_git,
            "git 根 + solution 关 → 非 git-managed"
        );

        let item = SolutionItem::Simple {
            path: plain_root.clone(),
            register_to_asset_manager: true,
            git_enabled: true,
        };
        assert!(
            !expand_item(&item).is_git,
            "非 git 根无论 solution 开关 → 非 git-managed"
        );

        let _ = std::fs::remove_dir_all(&base);
    }
}
