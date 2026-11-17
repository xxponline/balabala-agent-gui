//! 加载边界: 解析 solutions → 展开 Project → 结构校验 (spec: Solution
//! load failure blocks the workspace — 任一失败都使工作区打不开)。

use std::path::{Path, PathBuf};

use super::files::load_solutions;
use super::solution::{SolutionItem, expand_item};
use super::{Project, WorkspaceModel};

/// 加载/校验错误 — 调用方 (应用层打开流程) 据此弹错误框并退出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// solutions 文件读/解析失败, 或含未知 SolutionType
    /// (serde unknown variant 也归这里 — 消息点名未知类型)。
    Parse(String),
    /// `simple` item 的 path 在磁盘上不存在 (spec: Solution load failure)。
    MissingPath(PathBuf),
    /// 项目依赖 / 模块引用成环 (spec: DAG constraint)。
    Cycle(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::Parse(m) => write!(f, "solution configuration invalid: {m}"),
            ModelError::MissingPath(p) => write!(
                f,
                "solution path does not exist: {}",
                p.display()
            ),
            ModelError::Cycle(m) => write!(f, "dependency cycle: {m}"),
        }
    }
}

impl std::error::Error for ModelError {}

/// 加载工作区的领域模型 (读 solutions 文件 + 展开 + 校验)。
///
/// - solutions 缺失 → 合法空模型 (spec: missing file is a valid empty
///   workspace)。
/// - 解析失败 / 未知类型 / 死 path / (合成的) 环 → Err。
pub fn load(workspace_dir: &Path) -> Result<WorkspaceModel, ModelError> {
    let solutions = load_solutions(workspace_dir).map_err(ModelError::Parse)?;
    // 死 path 拦截: 加载边界保证进主窗的 Project 全部真实存在
    for item in &solutions {
        match item {
            SolutionItem::Simple { path } => {
                if !path.exists() {
                    return Err(ModelError::MissingPath(path.clone()));
                }
            }
        }
    }
    let projects = expand_all(&solutions)?;
    // v1 输出无跨项目依赖/模块引用 → 边集为空; 结构骨架已在位,
    // 多输出 solution 类型落地时把边集填进来即可 (spec: validation
    // activates once solutions can emit multiple projects)。
    check_dag(projects.len(), &[])?;
    Ok(WorkspaceModel {
        solutions,
        projects,
    })
}

/// items → Projects (纯展开, 不查磁盘 — 存在性检查在 `load`)。
pub(super) fn expand_all(solutions: &[SolutionItem]) -> Result<Vec<Project>, ModelError> {
    Ok(solutions.iter().flat_map(expand_item).collect())
}

/// DAG 校验 (Kahn 拓扑): `edges = [(from, to)]` 有环 → `ModelError::Cycle`。
///
/// v1 传空边集 (空转骨架); 测试用合成 A↔B 边验证拒绝路径。
pub fn check_dag(node_count: usize, edges: &[(usize, usize)]) -> Result<(), ModelError> {
    let mut indegree = vec![0usize; node_count];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); node_count];
    for &(from, to) in edges {
        if from >= node_count || to >= node_count {
            return Err(ModelError::Cycle(format!(
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
        Err(ModelError::Cycle(format!(
            "nodes [{}] form a cycle",
            stuck.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_model::files::{create_workspace_files, marker_path, solutions_path};
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-load-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_solutions_file_is_valid_empty_model() {
        let dir = temp_dir("empty");
        create_workspace_files(&dir).unwrap();
        fs::remove_file(solutions_path(&dir)).unwrap();
        let model = load(&dir).expect("缺失 = 合法空工作区");
        assert!(model.solutions.is_empty());
        assert!(model.projects.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_simple_item_yields_one_project_with_same_named_root_module() {
        let dir = temp_dir("one");
        let project_dir = dir.join("foo");
        fs::create_dir_all(&project_dir).unwrap();
        create_workspace_files(&dir).unwrap();
        fs::write(
            solutions_path(&dir),
            serde_json::json!([{ "type": "simple", "path": project_dir }]).to_string(),
        )
        .unwrap();

        let model = load(&dir).expect("合法加载");
        assert_eq!(model.projects.len(), 1);
        let p = &model.projects[0];
        assert_eq!(p.name, "foo");
        assert_eq!(p.root, project_dir);
        assert_eq!(p.modules.len(), 1);
        assert_eq!(p.modules[0].name, "foo");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_simple_items_yield_two_projects() {
        let dir = temp_dir("two");
        let a = dir.join("alpha");
        let b = dir.join("beta");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        create_workspace_files(&dir).unwrap();
        fs::write(
            solutions_path(&dir),
            serde_json::json!([
                { "type": "simple", "path": a },
                { "type": "simple", "path": b }
            ])
            .to_string(),
        )
        .unwrap();
        let model = load(&dir).expect("合法加载");
        assert_eq!(model.projects.len(), 2);
        assert_eq!(model.projects[0].name, "alpha");
        assert_eq!(model.projects[1].name, "beta");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_solutions_reports_parse_error() {
        let dir = temp_dir("corrupt");
        create_workspace_files(&dir).unwrap();
        fs::write(solutions_path(&dir), "{ nope").unwrap();
        assert!(matches!(load(&dir), Err(ModelError::Parse(_))));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_solution_type_reports_parse_error_naming_the_type() {
        let dir = temp_dir("unknown-type");
        create_workspace_files(&dir).unwrap();
        fs::write(
            solutions_path(&dir),
            r#"[{ "type": "cargo-workspace", "path": "/x" }]"#,
        )
        .unwrap();
        match load(&dir) {
            Err(ModelError::Parse(msg)) => {
                assert!(
                    msg.contains("cargo-workspace"),
                    "错误应点名未知类型: {msg}"
                )
            }
            other => panic!("应为 Parse(未知类型), got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_solution_path_is_rejected_at_load() {
        let dir = temp_dir("dead-path");
        create_workspace_files(&dir).unwrap();
        fs::write(
            solutions_path(&dir),
            serde_json::json!([{ "type": "simple", "path": dir.join("ghost") }]).to_string(),
        )
        .unwrap();
        assert_eq!(
            load(&dir),
            Err(ModelError::MissingPath(dir.join("ghost")))
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dag_check_rejects_synthetic_cycle() {
        // v1 输出无边; 用合成 A↔B 验证校验骨架 (spec: cyclic output rejected)
        assert!(check_dag(2, &[(0, 1), (1, 0)]).is_err());
        assert!(check_dag(2, &[(0, 1)]).is_ok(), "链是 DAG");
        assert!(check_dag(0, &[]).is_ok(), "空图是 DAG");
    }

    #[test]
    fn load_error_display_is_dialog_readable() {
        assert!(ModelError::MissingPath("/a/ghost".into())
            .to_string()
            .contains("/a/ghost"));
        assert!(ModelError::Parse("bad json".into())
            .to_string()
            .contains("bad json"));
        let _ = marker_path(Path::new("/unused")); // 引用防 unused 导入
    }
}
